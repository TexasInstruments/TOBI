use std::collections::BTreeSet;
#[cfg(target_os = "linux")]
use std::fs;
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use crate::board::{self, DetectedBoard};
use crate::custom_image::{
    CustomImage, custom_placeholder, is_custom_placeholder, scan_custom_images,
};
use crate::device::{DeviceMode, InstallTarget, TargetKind, list_devices};
use crate::installer::{InstallEvent, InstallRequest, RunMode, reboot_now, start_install};
use crate::manifest::{self, Catalog, ImageEntry};
use crate::memory::{MemoryCheck, check_image_memory, set_lite_xz_memory_guard};

pub const TI_PROXY_URL: &str = "http://webproxy.ext.ti.com:80";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Welcome,
    ImageSelect,
    CustomImageSelect,
    TargetSelect,
    Confirm,
    Installing,
    Complete,
    Error,
    ProxyConfig,
}

#[derive(Clone, Debug)]
pub struct ProgressState {
    pub phase: String,
    pub current: u64,
    pub total: Option<u64>,
    pub source_current: Option<u64>,
    pub source_total: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct SystemStatus {
    pub time: String,
    pub ip: String,
    pub ethernet: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyConfigField {
    Time,
    ProxyChoice,
    Proxy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyMode {
    Ti,
    Manual,
}

#[derive(Clone, Debug)]
pub struct RunnerGame {
    tick: u64,
    score: u64,
    runner_y: i16,
    velocity: i16,
    viewport_width: i16,
    obstacles: Vec<RunnerObstacle>,
    next_obstacle: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunnerObstacle {
    x: i16,
    kind: ObstacleKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObstacleKind {
    Cactus,
    Rock,
}

pub struct App {
    catalog: Catalog,
    board: DetectedBoard,
    devices: Vec<InstallTarget>,
    custom_images: Vec<CustomImage>,
    run_mode: RunMode,
    allow_write: bool,
    lite_mode: bool,
    manifest_source: String,
    proxy_url: Option<String>,
    proxy_input: String,
    proxy_time_input: String,
    proxy_config_field: ProxyConfigField,
    proxy_mode: ProxyMode,
    screen: Screen,
    image_index: usize,
    custom_image_index: usize,
    target_index: usize,
    expanded_targets: BTreeSet<String>,
    active_image: Option<ImageEntry>,
    active_image_is_custom: bool,
    progress: Option<ProgressState>,
    install_started_at: Option<Instant>,
    success_completed_at: Option<Instant>,
    status: String,
    warning: Option<String>,
    install_rx: Option<Receiver<InstallEvent>>,
    runner: RunnerGame,
    system_status: SystemStatus,
    system_status_refreshed_at: Option<Instant>,
    system_status_override: Option<SystemStatus>,
}

impl App {
    pub fn new(
        mut catalog: Catalog,
        board: DetectedBoard,
        devices: Vec<InstallTarget>,
        run_mode: RunMode,
        allow_write: bool,
        manifest_source: String,
        proxy_url: Option<String>,
        warning: Option<String>,
    ) -> Self {
        filter_catalog_for_board(&mut catalog, &board);
        catalog.images.push(custom_placeholder());
        sort_images_for_display(&mut catalog.images);
        let image_index = catalog
            .images
            .iter()
            .position(|image| image.recommended)
            .unwrap_or(0);

        Self {
            catalog,
            board,
            devices,
            custom_images: Vec::new(),
            run_mode,
            allow_write,
            lite_mode: false,
            manifest_source,
            proxy_input: proxy_url.clone().unwrap_or_default(),
            proxy_time_input: current_utc_datetime_input(),
            proxy_config_field: ProxyConfigField::Time,
            proxy_mode: ProxyMode::Ti,
            proxy_url,
            screen: Screen::Welcome,
            image_index,
            custom_image_index: 0,
            target_index: 0,
            expanded_targets: BTreeSet::new(),
            active_image: None,
            active_image_is_custom: false,
            progress: None,
            install_started_at: None,
            success_completed_at: None,
            status: "Press Enter to continue.".to_string(),
            warning,
            install_rx: None,
            runner: RunnerGame::default(),
            system_status: read_system_status(),
            system_status_refreshed_at: Some(Instant::now()),
            system_status_override: None,
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn board(&self) -> &DetectedBoard {
        &self.board
    }

    pub fn devices(&self) -> &[InstallTarget] {
        &self.devices
    }

    pub fn custom_images(&self) -> &[CustomImage] {
        &self.custom_images
    }

    pub fn run_mode(&self) -> RunMode {
        self.run_mode
    }

    pub fn set_lite_mode(&mut self, lite_mode: bool) {
        self.lite_mode = lite_mode;
        set_lite_xz_memory_guard(lite_mode);
        if lite_mode {
            self.status = "TOBI-lite installer ready. Press Enter to continue.".to_string();
        }
    }

    pub fn lite_mode(&self) -> bool {
        self.lite_mode
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn image_index(&self) -> usize {
        self.image_index
    }

    pub fn custom_image_index(&self) -> usize {
        self.custom_image_index
    }

    pub fn target_index(&self) -> usize {
        self.target_index
    }

    pub fn selected_catalog_image(&self) -> Option<&ImageEntry> {
        self.catalog.images.get(self.image_index)
    }

    pub fn selected_image(&self) -> Option<&ImageEntry> {
        if matches!(
            self.screen,
            Screen::TargetSelect
                | Screen::Confirm
                | Screen::Installing
                | Screen::Complete
                | Screen::Error
        ) {
            self.active_image
                .as_ref()
                .or_else(|| self.catalog.images.get(self.image_index))
        } else {
            self.catalog.images.get(self.image_index)
        }
    }

    pub fn selected_custom_image(&self) -> Option<&CustomImage> {
        self.custom_images.get(self.custom_image_index)
    }

    pub fn selected_target(&self) -> Option<&InstallTarget> {
        self.devices.get(self.target_index)
    }

    pub fn is_target_expanded(&self, target_id: &str) -> bool {
        self.expanded_targets.contains(target_id)
    }

    pub fn progress(&self) -> Option<&ProgressState> {
        self.progress.as_ref()
    }

    pub fn install_elapsed(&self) -> Option<Duration> {
        self.install_started_at.map(|started| started.elapsed())
    }

    pub fn install_rate_bytes_per_second(&self) -> Option<u64> {
        let elapsed = self.install_elapsed()?.as_secs().max(1);
        let current = self.progress.as_ref()?.current;
        Some(current / elapsed)
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn has_warning(&self) -> bool {
        self.warning.is_some() && self.screen != Screen::ProxyConfig
    }

    pub fn proxy_input(&self) -> &str {
        &self.proxy_input
    }

    pub fn configured_proxy_url(&self) -> Option<&str> {
        self.proxy_url.as_deref()
    }

    pub fn proxy_time_input(&self) -> &str {
        &self.proxy_time_input
    }

    pub fn proxy_config_field(&self) -> ProxyConfigField {
        self.proxy_config_field
    }

    pub fn proxy_mode(&self) -> ProxyMode {
        self.proxy_mode
    }

    pub fn active_proxy_url_for_display(&self) -> &str {
        match self.proxy_mode {
            ProxyMode::Ti => TI_PROXY_URL,
            ProxyMode::Manual => self.proxy_input(),
        }
    }

    pub fn memory_check(&self) -> Option<MemoryCheck> {
        self.selected_image().map(check_image_memory)
    }

    pub fn runner(&self) -> &RunnerGame {
        &self.runner
    }

    pub fn set_terminal_size(&mut self, width: u16, _height: u16) {
        self.runner.set_viewport_width(width);
    }

    pub fn system_status(&self) -> &SystemStatus {
        &self.system_status
    }

    pub fn start_proxy_setup_test(&mut self, warning: String) {
        self.system_status_override = Some(proxy_test_system_status());
        self.refresh_system_status_now();
        self.start_proxy_config();
        self.warning = Some(warning);
        self.status = "Connectivity test mode: set UTC time, then choose TI proxy or manual proxy."
            .to_string();
    }

    pub fn can_quit(&self) -> bool {
        self.screen != Screen::Installing
    }

    pub fn can_reboot_after_complete(&self) -> bool {
        self.screen == Screen::Complete
            && self.run_mode == RunMode::Live
            && self
                .selected_target()
                .is_some_and(|target| target.kind != TargetKind::File)
    }

    pub fn complete_auto_reboot_seconds(&self) -> Option<u64> {
        if !self.can_reboot_after_complete() {
            return None;
        }

        let elapsed = self.success_completed_at?.elapsed();
        let remaining = Duration::from_secs(10).saturating_sub(elapsed);
        if remaining.is_zero() {
            Some(0)
        } else {
            Some(remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0))
        }
    }

    pub fn auto_reboot_if_due(&mut self) {
        if self.complete_auto_reboot_seconds() == Some(0) {
            self.reboot_or_start_over();
        }
    }

    pub fn start_over(&mut self) {
        self.screen = Screen::Welcome;
        self.active_image = None;
        self.active_image_is_custom = false;
        self.progress = None;
        self.install_started_at = None;
        self.success_completed_at = None;
        self.install_rx = None;
        self.runner = RunnerGame::default();
        self.status = "Press Enter to continue.".to_string();
    }

    pub fn reboot_or_start_over(&mut self) {
        if !self.can_reboot_after_complete() {
            self.start_over();
            return;
        }

        self.status = "Reboot requested. If the board does not restart, reboot it manually from the serial console.".to_string();
        if let Err(error) = reboot_now() {
            self.screen = Screen::Error;
            self.status = format!(
                "Install succeeded, but TOBI could not reboot the board.\n\n{error:#}\n\nYou can reboot manually or press R to start over."
            );
        }
    }

    pub fn next(&mut self) {
        match self.screen {
            Screen::ImageSelect => {
                self.image_index = next_index(self.image_index, self.catalog.images.len());
            }
            Screen::CustomImageSelect => {
                self.custom_image_index =
                    next_index(self.custom_image_index, self.custom_images.len());
            }
            Screen::ProxyConfig => {}
            Screen::TargetSelect => {
                self.target_index = next_index(self.target_index, self.devices.len());
            }
            _ => {}
        }
    }

    pub fn previous(&mut self) {
        match self.screen {
            Screen::ImageSelect => {
                self.image_index = previous_index(self.image_index, self.catalog.images.len());
            }
            Screen::CustomImageSelect => {
                self.custom_image_index =
                    previous_index(self.custom_image_index, self.custom_images.len());
            }
            Screen::ProxyConfig => {}
            Screen::TargetSelect => {
                self.target_index = previous_index(self.target_index, self.devices.len());
            }
            _ => {}
        }
    }

    pub fn expand_target(&mut self) {
        if self.screen != Screen::TargetSelect {
            return;
        }
        let Some(target) = self.selected_target() else {
            return;
        };
        if !target.partitions.is_empty() {
            let target_id = target.id.clone();
            let target_name = target.name.clone();
            self.expanded_targets.insert(target_id);
            self.status = format!("Showing partitions for {target_name}.");
        }
    }

    pub fn collapse_target(&mut self) {
        if self.screen != Screen::TargetSelect {
            return;
        }
        let Some(target) = self.selected_target() else {
            return;
        };
        let target_id = target.id.clone();
        let target_name = target.name.clone();
        if self.expanded_targets.remove(&target_id) {
            self.status = format!("Hiding partitions for {target_name}.");
        }
    }

    pub fn back(&mut self) {
        match self.screen {
            Screen::ImageSelect => {
                self.screen = Screen::Welcome;
                self.status = "Press Enter to continue.".to_string();
            }
            Screen::CustomImageSelect => {
                self.screen = Screen::ImageSelect;
                self.status = "Choose an operating system image.".to_string();
            }
            Screen::TargetSelect => {
                if self.active_image_is_custom {
                    self.screen = Screen::CustomImageSelect;
                    self.status = "Choose a custom image from attached media.".to_string();
                } else {
                    self.screen = Screen::ImageSelect;
                    self.status = "Choose an operating system image.".to_string();
                }
            }
            Screen::Confirm => {
                self.screen = Screen::TargetSelect;
                self.status = "Choose target media.".to_string();
            }
            Screen::Complete | Screen::Error => self.start_over(),
            Screen::ProxyConfig => self.cancel_proxy_config(),
            _ => {}
        }
    }

    pub fn activate_selected(&mut self) {
        match self.screen {
            Screen::Welcome => {
                self.screen = Screen::ImageSelect;
                self.status = "Choose an operating system image.".to_string();
            }
            Screen::ImageSelect => {
                let Some(image) = self.selected_catalog_image().cloned() else {
                    self.status = "No image selected.".to_string();
                    return;
                };
                if is_custom_placeholder(&image) {
                    self.refresh_custom_images();
                    self.screen = Screen::CustomImageSelect;
                    self.status = "Choose a custom image from attached media.".to_string();
                } else {
                    self.active_image = Some(image);
                    self.active_image_is_custom = false;
                    self.screen = Screen::TargetSelect;
                    self.status = "Choose target media.".to_string();
                }
            }
            Screen::CustomImageSelect => {
                let Some(custom_image) = self.selected_custom_image().cloned() else {
                    self.status =
                        "No custom images found. Attach media and press R to rescan.".to_string();
                    return;
                };
                self.active_image = Some(custom_image.into_image_entry());
                self.active_image_is_custom = true;
                self.screen = Screen::TargetSelect;
                self.status = "Choose target media.".to_string();
            }
            Screen::TargetSelect => {
                self.screen = Screen::Confirm;
                self.status = "Confirm destructive write.".to_string();
            }
            Screen::Confirm => self.start_install(),
            Screen::Complete => self.reboot_or_start_over(),
            Screen::Error => self.start_over(),
            Screen::ProxyConfig => self.apply_proxy_config(),
            _ => {}
        }
    }

    pub fn dismiss_warning(&mut self) {
        self.warning = None;
    }

    pub fn start_proxy_config(&mut self) {
        self.warning = None;
        self.proxy_config_field = ProxyConfigField::Time;
        self.proxy_mode = ProxyMode::Ti;
        self.proxy_time_input.clear();
        self.proxy_input.clear();
        self.screen = Screen::ProxyConfig;
        self.status = "Enter UTC time first, then choose TI proxy or manual proxy.".to_string();
    }

    pub fn cancel_proxy_config(&mut self) {
        self.screen = Screen::ImageSelect;
        self.status = "Choose an operating system image.".to_string();
    }

    pub fn proxy_push(&mut self, ch: char) {
        if !ch.is_control() {
            match self.proxy_config_field {
                ProxyConfigField::Time => self.proxy_time_input.push(ch),
                ProxyConfigField::ProxyChoice => match ch {
                    't' | 'T' => self.select_ti_proxy(),
                    'm' | 'M' => self.select_manual_proxy(),
                    _ => {}
                },
                ProxyConfigField::Proxy => self.proxy_input.push(ch),
            }
        }
    }

    pub fn proxy_backspace(&mut self) {
        match self.proxy_config_field {
            ProxyConfigField::Time => {
                self.proxy_time_input.pop();
            }
            ProxyConfigField::ProxyChoice => {}
            ProxyConfigField::Proxy => {
                self.proxy_input.pop();
            }
        }
    }

    pub fn set_proxy_time_input(&mut self, value: impl Into<String>) {
        self.proxy_time_input = value.into();
    }

    pub fn set_proxy_input(&mut self, value: impl Into<String>) {
        self.proxy_input = value.into();
    }

    pub fn select_ti_proxy(&mut self) {
        self.proxy_mode = ProxyMode::Ti;
        if self.proxy_config_field != ProxyConfigField::Time {
            self.proxy_config_field = ProxyConfigField::ProxyChoice;
            self.status = "TI proxy selected. Press Enter to retry the online catalog.".to_string();
        }
    }

    pub fn select_manual_proxy(&mut self) {
        self.proxy_mode = ProxyMode::Manual;
        if self.proxy_config_field != ProxyConfigField::Time {
            self.proxy_config_field = ProxyConfigField::Proxy;
            self.status =
                "Manual proxy selected. Enter proxy URL and press Enter to retry.".to_string();
        }
    }

    pub fn toggle_proxy_mode(&mut self) {
        if self.proxy_config_field != ProxyConfigField::ProxyChoice {
            return;
        }
        self.proxy_mode = match self.proxy_mode {
            ProxyMode::Ti => ProxyMode::Manual,
            ProxyMode::Manual => ProxyMode::Ti,
        };
        self.status = match self.proxy_mode {
            ProxyMode::Ti => "TI proxy selected. Press Enter to retry the online catalog.",
            ProxyMode::Manual => "Manual proxy selected. Press Enter to enter the proxy URL.",
        }
        .to_string();
    }

    pub fn warn_proxy_choice_command(&mut self) {
        self.warning =
            Some("Type 'ti' to use the TI proxy or 'manual' to enter a proxy URL.".to_string());
    }

    pub fn next_proxy_field(&mut self) {
        match self.proxy_config_field {
            ProxyConfigField::Time => {
                self.proxy_config_field = ProxyConfigField::ProxyChoice;
                self.status = "Choose TI proxy or manual proxy, then press Enter.".to_string();
            }
            ProxyConfigField::ProxyChoice => {
                self.select_manual_proxy();
            }
            ProxyConfigField::Proxy => {}
        }
    }

    pub fn previous_proxy_field(&mut self) {
        match self.proxy_config_field {
            ProxyConfigField::Time => {}
            ProxyConfigField::ProxyChoice => {
                self.proxy_config_field = ProxyConfigField::Time;
                self.status =
                    "Confirm or edit the UTC time before retrying the online catalog.".to_string();
            }
            ProxyConfigField::Proxy => {
                self.proxy_config_field = ProxyConfigField::ProxyChoice;
                self.status = "Choose TI proxy or manual proxy, then press Enter.".to_string();
            }
        }
    }

    pub fn submit_proxy_config(&mut self) {
        if self.proxy_config_field == ProxyConfigField::Time {
            match validate_utc_datetime_input(&self.proxy_time_input) {
                Ok(()) => self.next_proxy_field(),
                Err(error) => {
                    self.warning = Some(format!("{error:#}"));
                }
            }
            return;
        }

        if self.proxy_config_field == ProxyConfigField::ProxyChoice {
            match self.proxy_mode {
                ProxyMode::Ti => self.apply_proxy_config(),
                ProxyMode::Manual => self.select_manual_proxy(),
            }
            return;
        }

        self.apply_proxy_config();
    }

    pub fn apply_proxy_config(&mut self) {
        if let Err(error) = set_system_time_for_proxy(self.run_mode, &self.proxy_time_input) {
            self.warning = Some(format!(
                "Could not set system time before retrying the online catalog.\n\n{error:#}"
            ));
            self.proxy_config_field = ProxyConfigField::Time;
            return;
        }
        self.refresh_system_status_now();

        let proxy = match self.proxy_mode {
            ProxyMode::Ti => Some(TI_PROXY_URL.to_string()),
            ProxyMode::Manual => {
                let proxy = self.proxy_input.trim().to_string();
                (!proxy.is_empty()).then_some(proxy)
            }
        };

        match manifest::load_catalog_with_proxy(&self.manifest_source, proxy.as_deref()) {
            Ok(catalog) => {
                self.proxy_url = proxy;
                self.replace_catalog(catalog);
                self.screen = Screen::ImageSelect;
                self.status = "Online OS catalog loaded.".to_string();
                self.warning = None;
            }
            Err(error) => {
                self.warning = Some(format!(
                    "Could not load the online catalog with this proxy. You can adjust the proxy and try again, or press Esc to continue with custom local images.\n\n{error:#}"
                ));
            }
        }
    }

    pub fn refresh_devices(&mut self) {
        if self.screen == Screen::CustomImageSelect {
            self.refresh_custom_images();
            return;
        }

        match list_devices(DeviceMode::from(self.run_mode), None) {
            Ok(devices) => {
                self.devices = devices;
                self.target_index = self.target_index.min(self.devices.len().saturating_sub(1));
                let target_ids = self
                    .devices
                    .iter()
                    .map(|target| target.id.clone())
                    .collect::<BTreeSet<_>>();
                self.expanded_targets
                    .retain(|target_id| target_ids.contains(target_id));
                self.status = "Storage targets refreshed.".to_string();
            }
            Err(error) => {
                self.status = format!("Could not refresh targets: {error:#}");
            }
        }
    }

    pub fn refresh_custom_images(&mut self) {
        match scan_custom_images() {
            Ok(images) => {
                self.custom_images = images;
                self.custom_image_index = self
                    .custom_image_index
                    .min(self.custom_images.len().saturating_sub(1));
                self.status = if self.custom_images.is_empty() {
                    "No custom images found. Attach media and press R to rescan.".to_string()
                } else {
                    format!("Found {} custom image(s).", self.custom_images.len())
                };
            }
            Err(error) => {
                self.status = format!("Could not scan attached media: {error:#}");
            }
        }
    }

    pub fn poll_install_events(&mut self) {
        let Some(rx) = self.install_rx.take() else {
            return;
        };

        let mut keep_rx = true;
        while let Ok(event) = rx.try_recv() {
            match event {
                InstallEvent::Phase(phase) => {
                    self.status = phase;
                }
                InstallEvent::Progress {
                    phase,
                    current,
                    total,
                    source_current,
                    source_total,
                } => {
                    self.status = phase.clone();
                    self.progress = Some(ProgressState {
                        phase,
                        current,
                        total,
                        source_current,
                        source_total,
                    });
                }
                InstallEvent::Complete(message) => {
                    self.status = message;
                    self.screen = Screen::Complete;
                    self.success_completed_at = Some(Instant::now());
                    keep_rx = false;
                }
                InstallEvent::Failed(message) => {
                    self.status = message;
                    self.screen = Screen::Error;
                    self.success_completed_at = None;
                    keep_rx = false;
                }
            }
        }

        if keep_rx {
            self.install_rx = Some(rx);
        }
    }

    pub fn tick_runner(&mut self) {
        if self.screen == Screen::Installing && !self.lite_mode {
            self.runner.tick();
        }
    }

    pub fn refresh_system_status_if_due(&mut self) {
        if self.system_status_override.is_some() {
            self.refresh_system_status_now();
            return;
        }

        let now = Instant::now();
        let should_refresh = self
            .system_status_refreshed_at
            .map(|last| now.duration_since(last) >= Duration::from_secs(1))
            .unwrap_or(true);
        if should_refresh {
            self.system_status = read_system_status();
            self.system_status_refreshed_at = Some(now);
        }
    }

    fn refresh_system_status_now(&mut self) {
        self.system_status = self
            .system_status_override
            .clone()
            .unwrap_or_else(read_system_status);
        self.system_status_refreshed_at = Some(Instant::now());
    }

    pub fn runner_jump_or_restart(&mut self) {
        if self.screen == Screen::Installing {
            self.runner.jump_or_restart();
        }
    }

    fn start_install(&mut self) {
        let Some(image) = self.selected_image().cloned() else {
            self.status = "No image selected.".to_string();
            return;
        };
        let Some(target) = self.selected_target().cloned() else {
            self.status = "No target selected.".to_string();
            return;
        };

        let memory = check_image_memory(&image);
        if memory.enough == Some(false) {
            self.screen = Screen::Error;
            self.status = format!(
                "Not enough available RAM for the installer working set. {}. TOBI streams images, so the full image does not need to fit in RAM.",
                memory.summary()
            );
            return;
        }

        self.screen = Screen::Installing;
        self.runner = RunnerGame::default();
        self.install_started_at = Some(Instant::now());
        self.status = "Starting install.".to_string();
        self.progress = Some(ProgressState {
            phase: "Starting".to_string(),
            current: 0,
            total: image.extract_size,
            source_current: Some(0),
            source_total: image.image_download_size,
        });
        let reboot_after_install =
            self.run_mode == RunMode::Live && target.kind != TargetKind::File;
        self.install_rx = Some(start_install(InstallRequest {
            image,
            target,
            run_mode: self.run_mode,
            allow_write: self.allow_write,
            proxy_url: self.proxy_url.clone(),
            reboot_after_install,
        }));
    }

    fn replace_catalog(&mut self, mut catalog: Catalog) {
        if self.board.id.is_none() {
            self.board = board::detect_board(self.run_mode, &catalog);
        }
        filter_catalog_for_board(&mut catalog, &self.board);
        catalog.images.push(custom_placeholder());
        sort_images_for_display(&mut catalog.images);
        self.catalog = catalog;
        self.image_index = self
            .catalog
            .images
            .iter()
            .position(|image| image.recommended)
            .unwrap_or(0);
        self.active_image = None;
        self.active_image_is_custom = false;
    }
}

impl Default for RunnerGame {
    fn default() -> Self {
        let mut game = Self {
            tick: 0,
            score: 0,
            runner_y: 0,
            velocity: 0,
            viewport_width: 80,
            obstacles: Vec::new(),
            next_obstacle: 0,
        };
        game.reset_obstacles();
        game
    }
}

impl RunnerObstacle {
    pub(crate) fn new(x: i16, kind: ObstacleKind) -> Self {
        Self { x, kind }
    }

    pub fn x(&self) -> i16 {
        self.x
    }

    pub fn kind(&self) -> ObstacleKind {
        self.kind
    }
}

impl RunnerGame {
    pub fn score(&self) -> u64 {
        self.score
    }

    pub fn runner_y(&self) -> i16 {
        self.runner_y
    }

    pub fn obstacles(&self) -> &[RunnerObstacle] {
        &self.obstacles
    }

    fn tick(&mut self) {
        self.tick = self.tick.saturating_add(1);
        self.score = self.score.saturating_add(1);

        for obstacle in &mut self.obstacles {
            obstacle.x -= 1;
        }
        self.recycle_obstacles();

        if self.runner_y > 0 || self.velocity > 0 {
            self.runner_y += self.velocity;
            self.velocity -= 1;
            if self.runner_y <= 0 {
                self.runner_y = 0;
                self.velocity = 0;
            }
        }
    }

    fn jump_or_restart(&mut self) {
        if self.runner_y == 0 {
            self.velocity = 5;
        }
    }

    fn set_viewport_width(&mut self, width: u16) {
        let install_popup_width = i16::try_from(width.saturating_mul(72) / 100).unwrap_or(80);
        self.viewport_width = install_popup_width.saturating_sub(4).max(32);
        if self.obstacles.is_empty() {
            self.reset_obstacles();
        }
    }

    fn reset_obstacles(&mut self) {
        self.next_obstacle = 0;
        let visible_start = (self.viewport_width / 2).max(18);
        self.obstacles = vec![
            self.next_runner_obstacle_at(visible_start + 4),
            self.next_runner_obstacle_at(self.viewport_width + 8),
            self.next_runner_obstacle_at(self.viewport_width + 34),
        ];
    }

    fn recycle_obstacles(&mut self) {
        let mut next_x = self
            .obstacles
            .iter()
            .map(|obstacle| obstacle.x)
            .max()
            .unwrap_or(self.viewport_width)
            .max(self.viewport_width)
            + 24;

        for index in 0..self.obstacles.len() {
            if self.obstacles[index].x >= -8 {
                continue;
            }

            self.obstacles[index] = self.next_runner_obstacle_at(next_x);
            next_x += 24;
        }
    }

    fn next_runner_obstacle_at(&mut self, x: i16) -> RunnerObstacle {
        let kind = if self.next_obstacle.is_multiple_of(2) {
            ObstacleKind::Cactus
        } else {
            ObstacleKind::Rock
        };
        self.next_obstacle = self.next_obstacle.saturating_add(1);
        RunnerObstacle::new(x, kind)
    }
}

fn next_index(current: usize, len: usize) -> usize {
    if len == 0 { 0 } else { (current + 1) % len }
}

fn previous_index(current: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (current + len - 1) % len
    }
}

fn filter_catalog_for_board(catalog: &mut Catalog, board: &DetectedBoard) {
    let mut device_ids = BTreeSet::new();
    if let Some(id) = board.id.as_ref().filter(|id| !id.trim().is_empty()) {
        device_ids.insert(id.clone());
    }
    if device_ids.is_empty() {
        for device in &catalog.devices {
            if device
                .compatible
                .iter()
                .any(|entry| board.compatible.iter().any(|detected| detected == entry))
            {
                device_ids.insert(device.id.clone());
            }
        }
    }

    if device_ids.is_empty() {
        return;
    }

    catalog.images.retain(|image| {
        image.devices.is_empty()
            || image
                .devices
                .iter()
                .any(|device| device_ids.contains(device))
    });
}

fn sort_images_for_display(images: &mut [ImageEntry]) {
    images.sort_by(|a, b| {
        category_rank(a.category_label())
            .cmp(&category_rank(b.category_label()))
            .then_with(|| a.category_label().cmp(b.category_label()))
            .then_with(|| b.recommended.cmp(&a.recommended))
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn category_rank(category: &str) -> u8 {
    match category {
        "Yocto" => 0,
        "Edge AI" => 1,
        "Debian" => 2,
        "Community" => 3,
        "Virtualization" => 4,
        "Buildroot" => 5,
        "Custom" => 99,
        _ => 50,
    }
}

fn read_system_status() -> SystemStatus {
    let ip = read_primary_ipv4().unwrap_or_else(|| "no IPv4".to_string());
    let ethernet = read_ethernet_status(&ip);
    SystemStatus {
        time: command_output("date", &["-u", "+%Y-%m-%d %H:%M:%S UTC"])
            .unwrap_or_else(|| "time unavailable".to_string()),
        ip,
        ethernet,
    }
}

fn proxy_test_system_status() -> SystemStatus {
    let ip = "192.168.1.50".to_string();
    SystemStatus {
        time: format!("{} UTC", current_utc_datetime_input()),
        ethernet: format!("connected (eth0 {ip}, DHCP test)"),
        ip,
    }
}

fn current_utc_datetime_input() -> String {
    command_output("date", &["-u", "+%Y-%m-%d %H:%M:%S"])
        .unwrap_or_else(|| "2026-01-01 00:00:00".to_string())
}

fn set_system_time_for_proxy(run_mode: RunMode, input: &str) -> anyhow::Result<()> {
    validate_utc_datetime_input(input)?;
    if run_mode == RunMode::Mock {
        return Ok(());
    }

    set_system_utc_time(input.trim())
}

fn validate_utc_datetime_input(input: &str) -> anyhow::Result<()> {
    let input = input.trim();
    if input.len() != 19 {
        bail!("UTC time must use YYYY-MM-DD HH:MM:SS format.");
    }

    let bytes = input.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b' ')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit())
    {
        bail!("UTC time must use YYYY-MM-DD HH:MM:SS format.");
    }

    let year = parse_datetime_part(input, 0..4, "year")?;
    let month = parse_datetime_part(input, 5..7, "month")?;
    let day = parse_datetime_part(input, 8..10, "day")?;
    let hour = parse_datetime_part(input, 11..13, "hour")?;
    let minute = parse_datetime_part(input, 14..16, "minute")?;
    let second = parse_datetime_part(input, 17..19, "second")?;

    if !(2020..=2099).contains(&year) {
        bail!("UTC year must be between 2020 and 2099.");
    }
    if !(1..=12).contains(&month) {
        bail!("UTC month must be between 01 and 12.");
    }
    if !(1..=days_in_month(year, month)).contains(&day) {
        bail!("UTC day is not valid for this month.");
    }
    if hour > 23 || minute > 59 || second > 59 {
        bail!("UTC time must be a valid 24-hour time.");
    }

    Ok(())
}

fn parse_datetime_part(
    input: &str,
    range: std::ops::Range<usize>,
    label: &str,
) -> anyhow::Result<u32> {
    input[range]
        .parse::<u32>()
        .with_context(|| format!("invalid UTC {label}"))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(target_os = "linux")]
fn set_system_utc_time(input: &str) -> anyhow::Result<()> {
    let attempts: [&[&str]; 2] = [
        &["-u", "-s", input],
        &["-u", "-D", "%Y-%m-%d %H:%M:%S", "-s", input],
    ];
    let mut last_error = String::new();

    for args in attempts {
        let output = Command::new("date")
            .args(args)
            .output()
            .context("failed to run date to set system time")?;
        if output.status.success() {
            return Ok(());
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if last_error.is_empty() {
            last_error = format!("date exited with status {}", output.status);
        }
    }

    bail!("{last_error}");
}

#[cfg(not(target_os = "linux"))]
fn set_system_utc_time(_input: &str) -> anyhow::Result<()> {
    bail!("setting system time is supported only on live Linux targets");
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn read_primary_ipv4() -> Option<String> {
    read_primary_ipv4_from_ip().or_else(read_primary_ipv4_from_ifconfig)
}

fn read_primary_ipv4_from_ip() -> Option<String> {
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "scope", "global"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let iface = fields.get(1)?.trim_end_matches(':');
        let addr = fields.get(3)?.split('/').next()?;
        Some(format!("{iface} {addr}"))
    })
}

#[cfg(target_os = "linux")]
fn read_ipv4_for_iface(iface: &str) -> Option<String> {
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", iface, "scope", "global"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let addr = fields.get(3)?.split('/').next()?;
        Some(addr.to_string())
    })
}

fn read_primary_ipv4_from_ifconfig() -> Option<String> {
    let output = Command::new("ifconfig").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut iface = None;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) && line.contains(':') {
            iface = line.split(':').next().map(|value| value.to_string());
            continue;
        }

        let Some(current_iface) = iface.as_deref() else {
            continue;
        };
        if current_iface == "lo" || current_iface == "lo0" {
            continue;
        }

        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.first() == Some(&"inet") {
            let Some(addr) = fields.get(1) else {
                continue;
            };
            if !addr.starts_with("127.") {
                return Some(format!("{current_iface} {addr}"));
            }
        }
    }

    None
}

fn read_ethernet_status(primary_ip: &str) -> String {
    read_linux_ethernet_status()
        .or_else(read_macos_ethernet_status)
        .unwrap_or_else(|| {
            if primary_ip == "no IPv4" {
                "not connected".to_string()
            } else {
                format!("network online ({primary_ip})")
            }
        })
}

#[cfg(target_os = "linux")]
fn read_linux_ethernet_status() -> Option<String> {
    let entries = fs::read_dir("/sys/class/net").ok()?;
    let mut ethernet_seen = false;

    for entry in entries.filter_map(Result::ok) {
        let iface = entry.file_name().to_string_lossy().into_owned();
        if !is_likely_ethernet_iface(&iface) {
            continue;
        }

        let path = entry.path();
        let interface_type = fs::read_to_string(path.join("type")).ok();
        if interface_type.as_deref().map(str::trim) != Some("1") {
            continue;
        }
        ethernet_seen = true;

        let carrier = fs::read_to_string(path.join("carrier")).ok();
        let operstate = fs::read_to_string(path.join("operstate")).ok();
        let link_up = carrier.as_deref().map(str::trim) == Some("1")
            || operstate.as_deref().map(str::trim) == Some("up");
        if !link_up {
            continue;
        }

        if let Some(addr) = read_ipv4_for_iface(&iface) {
            return Some(format!("connected ({iface} {addr})"));
        }
        return Some(format!("connected ({iface}, waiting for DHCP)"));
    }

    ethernet_seen.then(|| "not connected".to_string())
}

#[cfg(not(target_os = "linux"))]
fn read_linux_ethernet_status() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn is_likely_ethernet_iface(iface: &str) -> bool {
    iface != "lo"
        && !iface.starts_with("wl")
        && !iface.starts_with("docker")
        && !iface.starts_with("veth")
        && !iface.starts_with("br-")
        && !iface.starts_with("tun")
        && !iface.starts_with("tap")
}

#[cfg(target_os = "macos")]
fn read_macos_ethernet_status() -> Option<String> {
    let ports = command_output("networksetup", &["-listallhardwareports"])?;
    let mut ethernet_seen = false;

    for block in ports.split("\n\n") {
        let is_ethernet = block
            .lines()
            .find_map(|line| line.strip_prefix("Hardware Port: "))
            .map(|port| port.contains("Ethernet"))
            .unwrap_or(false);
        if !is_ethernet {
            continue;
        }

        let Some(iface) = block
            .lines()
            .find_map(|line| line.strip_prefix("Device: "))
            .map(str::trim)
        else {
            continue;
        };
        ethernet_seen = true;

        if let Some(status) = read_macos_iface_status(iface) {
            return Some(status);
        }
    }

    ethernet_seen.then(|| "not connected".to_string())
}

#[cfg(not(target_os = "macos"))]
fn read_macos_ethernet_status() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn read_macos_iface_status(iface: &str) -> Option<String> {
    let text = command_output("ifconfig", &[iface])?;
    let active = text
        .lines()
        .any(|line| line.trim_start() == "status: active");
    if !active {
        return None;
    }

    let addr = text.lines().find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.first() == Some(&"inet") {
            fields.get(1).copied()
        } else {
            None
        }
    });

    Some(match addr {
        Some(addr) => format!("connected ({iface} {addr})"),
        None => format!("connected ({iface}, waiting for DHCP)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{BoardSource, DetectedBoard};
    use crate::device::{InstallTarget, TargetKind};
    use crate::manifest::{Catalog, ImageEntry, ImageFormat};

    #[test]
    fn starts_on_welcome() {
        let app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        assert_eq!(app.screen(), Screen::Welcome);
    }

    #[test]
    fn enter_advances_from_welcome_to_image_list() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        app.activate_selected();
        assert_eq!(app.screen(), Screen::ImageSelect);
    }

    #[test]
    fn enter_advances_from_image_to_target() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        app.activate_selected();
        app.activate_selected();
        assert_eq!(app.screen(), Screen::TargetSelect);
    }

    #[test]
    fn recommended_image_is_selected_first() {
        let app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        assert_eq!(app.selected_image().expect("image").name, "Image");
    }

    #[test]
    fn custom_placeholder_is_added() {
        let app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        assert!(app.catalog().images.iter().any(is_custom_placeholder));
    }

    #[test]
    fn board_filter_hides_images_for_other_boards() {
        let mut catalog = catalog();
        catalog.images[0].devices = vec!["sk-am62p-lp".to_string()];
        let mut other_image = catalog.images[0].clone();
        other_image.id = "other-board-image".to_string();
        other_image.name = "Other Board Image".to_string();
        other_image.devices = vec!["sk-am64b".to_string()];
        catalog.images.push(other_image);

        let app = App::new(
            catalog,
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );

        assert!(app.catalog().images.iter().any(|image| image.id == "image"));
        assert!(
            !app.catalog()
                .images
                .iter()
                .any(|image| image.id == "other-board-image")
        );
    }

    #[test]
    fn board_filter_uses_detected_id_before_generic_compatible_aliases() {
        let mut catalog = catalog();
        catalog.devices = vec![
            crate::manifest::DeviceEntry {
                id: "sk-am64b".to_string(),
                name: "SK-AM64B".to_string(),
                compatible: vec!["ti,am642-sk".to_string(), "ti,am642".to_string()],
            },
            crate::manifest::DeviceEntry {
                id: "tmds64evm".to_string(),
                name: "TMDS64EVM".to_string(),
                compatible: vec!["ti,am642-evm".to_string(), "ti,am642".to_string()],
            },
        ];
        catalog.images[0].devices = vec!["tmds64evm".to_string()];
        let mut sk_only = catalog.images[0].clone();
        sk_only.id = "sk-am64b-only".to_string();
        sk_only.name = "SK-AM64B Only".to_string();
        sk_only.devices = vec!["sk-am64b".to_string()];
        catalog.images.push(sk_only);

        let app = App::new(
            catalog,
            DetectedBoard {
                id: Some("tmds64evm".to_string()),
                name: "TMDS64EVM".to_string(),
                compatible: vec!["ti,am642-evm".to_string(), "ti,am642".to_string()],
                source: BoardSource::DeviceTree,
            },
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );

        assert!(app.catalog().images.iter().any(|image| image.id == "image"));
        assert!(
            !app.catalog()
                .images
                .iter()
                .any(|image| image.id == "sk-am64b-only")
        );
    }

    #[test]
    fn complete_screen_reboots_only_live_block_targets() {
        let mut live_app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Live,
            true,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        live_app.screen = Screen::Complete;
        assert!(live_app.can_reboot_after_complete());

        let mut mock_app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        mock_app.screen = Screen::Complete;
        assert!(!mock_app.can_reboot_after_complete());
    }

    #[test]
    fn live_complete_screen_counts_down_to_reboot() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Live,
            true,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        app.screen = Screen::Complete;
        app.success_completed_at = Some(Instant::now());
        assert!(matches!(app.complete_auto_reboot_seconds(), Some(1..=10)));

        app.success_completed_at = Some(Instant::now() - Duration::from_secs(10));
        assert_eq!(app.complete_auto_reboot_seconds(), Some(0));
    }

    #[test]
    fn runner_tick_moves_obstacles_toward_runner() {
        let mut runner = RunnerGame::default();
        let before = runner
            .obstacles()
            .iter()
            .map(RunnerObstacle::x)
            .collect::<Vec<_>>();

        runner.tick();

        let after = runner
            .obstacles()
            .iter()
            .map(RunnerObstacle::x)
            .collect::<Vec<_>>();
        assert_eq!(before.len(), after.len());
        assert!(
            before
                .iter()
                .zip(after.iter())
                .all(|(before, after)| after < before)
        );
    }

    #[test]
    fn runner_jump_does_not_move_obstacles() {
        let mut runner = RunnerGame::default();
        let before = runner.obstacles().to_vec();

        runner.jump_or_restart();

        assert_eq!(runner.runner_y(), 0);
        assert_eq!(runner.obstacles(), before.as_slice());
    }

    #[test]
    fn proxy_config_prompts_for_time_before_proxy() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            Some("http://proxy.example.com:8080".to_string()),
            None,
        );

        app.start_proxy_config();

        assert_eq!(app.proxy_config_field, ProxyConfigField::Time);
        assert_eq!(app.proxy_mode, ProxyMode::Ti);
        assert!(app.proxy_time_input.is_empty());
        assert!(app.proxy_input.is_empty());
    }

    #[test]
    fn proxy_config_enter_advances_after_valid_time_to_proxy_choice() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        app.start_proxy_config();
        app.proxy_time_input = "2026-05-08 12:34:56".to_string();

        app.submit_proxy_config();

        assert_eq!(app.proxy_config_field, ProxyConfigField::ProxyChoice);
        assert_eq!(app.proxy_mode, ProxyMode::Ti);
        assert!(app.warning.is_none());
    }

    #[test]
    fn proxy_config_uses_ti_proxy_by_default_after_time() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        app.start_proxy_config();
        app.proxy_time_input = "2026-05-08 12:34:56".to_string();

        app.submit_proxy_config();
        app.submit_proxy_config();

        assert_eq!(app.screen, Screen::ImageSelect);
        assert_eq!(app.configured_proxy_url(), Some(TI_PROXY_URL));
        assert!(app.warning.is_none());
    }

    #[test]
    fn proxy_config_manual_choice_prompts_for_proxy_url() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        app.start_proxy_config();
        app.proxy_time_input = "2026-05-08 12:34:56".to_string();

        app.submit_proxy_config();
        app.select_manual_proxy();

        assert_eq!(app.proxy_config_field, ProxyConfigField::Proxy);
        assert_eq!(app.proxy_mode, ProxyMode::Manual);
    }

    #[test]
    fn proxy_config_rejects_invalid_time_before_proxy() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "sample/catalog.json".to_string(),
            None,
            None,
        );
        app.start_proxy_config();
        app.proxy_time_input = "2026-02-30 12:34:56".to_string();

        app.submit_proxy_config();

        assert_eq!(app.proxy_config_field, ProxyConfigField::Time);
        assert!(app.warning.is_some());
    }

    #[test]
    fn proxy_setup_test_starts_on_time_field_with_dhcp_status() {
        let mut app = App::new(
            catalog(),
            board(),
            targets(),
            RunMode::Mock,
            false,
            "https://example.invalid/catalog.json".to_string(),
            None,
            None,
        );

        app.start_proxy_setup_test("test catalog failure".to_string());

        assert_eq!(app.screen, Screen::ProxyConfig);
        assert_eq!(app.proxy_config_field, ProxyConfigField::Time);
        assert_eq!(app.proxy_mode, ProxyMode::Ti);
        assert_eq!(app.system_status.ip, "192.168.1.50");
        assert!(app.system_status.ethernet.contains("DHCP test"));
        assert!(app.proxy_time_input.is_empty());
        assert!(app.proxy_input.is_empty());
        assert_eq!(app.warning.as_deref(), Some("test catalog failure"));
    }

    #[test]
    fn validates_utc_datetime_format() {
        assert!(validate_utc_datetime_input("2026-02-28 23:59:59").is_ok());
        assert!(validate_utc_datetime_input("2028-02-29 00:00:00").is_ok());
        assert!(validate_utc_datetime_input("2026-02-29 00:00:00").is_err());
        assert!(validate_utc_datetime_input("2026-05-08T12:00:00").is_err());
        assert!(validate_utc_datetime_input("2019-05-08 12:00:00").is_err());
    }

    fn catalog() -> Catalog {
        Catalog {
            schema_version: 1,
            generated_at: None,
            devices: vec![],
            images: vec![ImageEntry {
                id: "image".to_string(),
                name: "Image".to_string(),
                description: "Test image".to_string(),
                devices: vec![],
                category: Some("Yocto".to_string()),
                recommended: true,
                version: "1".to_string(),
                release_date: "2026-05-07".to_string(),
                channel: "dev".to_string(),
                url: "mock://image".to_string(),
                format: ImageFormat::Raw,
                image_download_sha256: None,
                extract_sha256: None,
                image_download_size: Some(1),
                extract_size: Some(1),
                decoder_memory_size: None,
                bmap_url: None,
                signature_url: None,
            }],
        }
    }

    fn board() -> DetectedBoard {
        DetectedBoard {
            id: Some("sk-am62p-lp".to_string()),
            name: "SK-AM62P-LP".to_string(),
            compatible: vec!["ti,am62pxx-evm".to_string()],
            source: BoardSource::Mock,
        }
    }

    fn targets() -> Vec<InstallTarget> {
        vec![InstallTarget {
            id: "target".to_string(),
            name: "Target".to_string(),
            path: "/dev/mmcblk0".into(),
            size_bytes: Some(1),
            kind: TargetKind::Emmc,
            removable: false,
            partitions: Vec::new(),
            warning: None,
        }]
    }
}
