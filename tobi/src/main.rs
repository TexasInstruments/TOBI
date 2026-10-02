mod app;
mod board;
mod boot_patch;
mod custom_image;
mod device;
mod installer;
mod manifest;
mod memory;
mod qr;
mod ui;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use app::{App, Screen};
use clap::{Parser, ValueEnum};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    Clear as TerminalClear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode,
};
use device::DeviceMode;
use installer::RunMode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/Grippy98/TOBI/master/tobi/sample/catalog.json";

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Args {
    #[arg(long, default_value = DEFAULT_MANIFEST_URL)]
    manifest: String,

    #[arg(long, value_enum, default_value_t = CliRunMode::Live)]
    mode: CliRunMode,

    #[arg(long)]
    target: Option<PathBuf>,

    #[arg(long)]
    allow_write: bool,

    #[arg(long, conflicts_with = "allow_write")]
    no_allow_write: bool,

    #[arg(long)]
    proxy: Option<String>,

    #[arg(long)]
    no_alt_screen: bool,

    #[arg(long)]
    serial_ui: bool,

    #[arg(long)]
    lite: bool,

    #[arg(long)]
    test_proxy_setup: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliRunMode {
    Mock,
    Live,
}

impl Args {
    fn write_allowed(&self) -> bool {
        !self.no_allow_write && (self.allow_write || self.mode == CliRunMode::Live)
    }
}

impl From<CliRunMode> for RunMode {
    fn from(value: CliRunMode) -> Self {
        match value {
            CliRunMode::Mock => Self::Mock,
            CliRunMode::Live => Self::Live,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let run_mode = RunMode::from(args.mode);
    let serial_ui = args.serial_ui;
    let use_alt_screen = !args.no_alt_screen;
    let test_proxy_setup = args.test_proxy_setup;
    let (catalog, warning) = load_startup_catalog(&args)?;
    let board = board::detect_board(run_mode, &catalog);
    let devices = device::list_devices(DeviceMode::from(run_mode), args.target.as_deref())
        .context("failed to enumerate install targets")?;
    let mut app = App::new(
        catalog,
        board,
        devices,
        run_mode,
        args.write_allowed(),
        args.manifest,
        args.proxy,
        warning,
    );
    if args.lite {
        app.set_lite_mode(true);
    }
    if test_proxy_setup {
        app.start_proxy_setup_test(proxy_setup_test_warning());
    }

    if serial_ui {
        return run_serial_app(app);
    }

    let mut terminal = setup_terminal(use_alt_screen)?;
    let app_result = run_app(&mut terminal, app);
    restore_terminal(&mut terminal, use_alt_screen)?;
    app_result
}

fn load_startup_catalog(args: &Args) -> anyhow::Result<(manifest::Catalog, Option<String>)> {
    if args.test_proxy_setup {
        return Ok((manifest::fallback_catalog(), None));
    }

    match manifest::load_catalog_with_proxy(&args.manifest, args.proxy.as_deref()) {
        Ok(catalog) => Ok((catalog, None)),
        Err(error) if manifest::is_remote_source(&args.manifest) => Ok((
            manifest::fallback_catalog(),
            Some(format!(
                "Could not fetch the online OS catalog. You can still flash a custom local image from attached media. Press P to set UTC time, choose TI proxy or manual proxy, and retry, or Enter to continue.\n\n{error:#}"
            )),
        )),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to load manifest {}", args.manifest));
        }
    }
}

fn proxy_setup_test_warning() -> String {
    "Connectivity test mode: DHCP and local IP are present, but the online catalog is treated as unreachable. Set UTC time, choose TI proxy or manual proxy, and submit to retry the configured catalog.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_live_write_mode() {
        let args = Args::parse_from(["tobi"]);
        assert_eq!(args.mode, CliRunMode::Live);
        assert!(args.write_allowed());
    }

    #[test]
    fn mock_mode_is_still_available_for_safe_local_runs() {
        let args = Args::parse_from(["tobi", "--mode", "mock"]);
        assert_eq!(args.mode, CliRunMode::Mock);
        assert!(!args.write_allowed());
    }

    #[test]
    fn live_write_permission_can_be_disabled_explicitly() {
        let args = Args::parse_from(["tobi", "--mode", "live", "--no-allow-write"]);
        assert_eq!(args.mode, CliRunMode::Live);
        assert!(!args.write_allowed());
    }

    #[test]
    fn proxy_setup_test_mode_is_available() {
        let args = Args::parse_from(["tobi", "--mode", "mock", "--test-proxy-setup"]);
        assert_eq!(args.mode, CliRunMode::Mock);
        assert!(args.test_proxy_setup);
    }

    #[test]
    fn lite_mode_is_available_for_low_memory_images() {
        let args = Args::parse_from(["tobi", "--mode", "mock", "--lite"]);
        assert_eq!(args.mode, CliRunMode::Mock);
        assert!(args.lite);
    }
}

fn setup_terminal(
    use_alt_screen: bool,
) -> anyhow::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = std::io::stdout();
    if use_alt_screen {
        execute!(stdout, EnterAlternateScreen, TerminalClear(ClearType::All))
            .context("failed to enter alternate screen")?;
    } else {
        execute!(stdout, TerminalClear(ClearType::All)).context("failed to clear terminal")?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
    terminal.clear().context("failed to clear terminal")?;
    Ok(terminal)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    use_alt_screen: bool,
) -> anyhow::Result<()> {
    disable_raw_mode().context("failed to disable terminal raw mode")?;
    if use_alt_screen {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)
            .context("failed to leave alternate screen")?;
    }
    terminal.show_cursor().context("failed to show cursor")
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut app: App,
) -> anyhow::Result<()> {
    loop {
        app.poll_install_events();
        app.tick_runner();
        app.refresh_system_status_if_due();
        app.auto_reboot_if_due();
        terminal.draw(|frame| {
            let area = frame.area();
            app.set_terminal_size(area.width, area.height);
            ui::render(frame, &app)
        })?;

        if event::poll(Duration::from_millis(80))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') if app.can_quit() => {
                    return Ok(());
                }
                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                _ if app.has_warning() => match key.code {
                    KeyCode::Enter | KeyCode::Esc => app.dismiss_warning(),
                    KeyCode::Char('p') | KeyCode::Char('P') => app.start_proxy_config(),
                    _ => {}
                },
                _ if app.screen() == Screen::ProxyConfig => match key.code {
                    KeyCode::Enter => app.submit_proxy_config(),
                    KeyCode::Esc => app.cancel_proxy_config(),
                    KeyCode::Left | KeyCode::Right => app.toggle_proxy_mode(),
                    KeyCode::Tab | KeyCode::Down => app.next_proxy_field(),
                    KeyCode::BackTab | KeyCode::Up => app.previous_proxy_field(),
                    KeyCode::Backspace => app.proxy_backspace(),
                    KeyCode::Char(ch) => app.proxy_push(ch),
                    _ => {}
                },
                _ if app.screen() == Screen::Installing => match key.code {
                    KeyCode::Char(' ') | KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') => {
                        app.runner_jump_or_restart();
                    }
                    _ => {}
                },
                _ if matches!(app.screen(), Screen::Complete | Screen::Error) => match key.code {
                    KeyCode::Enter => app.activate_selected(),
                    KeyCode::Char('r') | KeyCode::Char('R') => app.start_over(),
                    _ => {}
                },
                KeyCode::Right if app.screen() == Screen::TargetSelect => app.expand_target(),
                KeyCode::Left if app.screen() == Screen::TargetSelect => app.collapse_target(),
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => app.previous(),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => app.next(),
                KeyCode::Enter => app.activate_selected(),
                KeyCode::Backspace => app.back(),
                KeyCode::Char('r') | KeyCode::Char('R') if app.screen() != Screen::Installing => {
                    app.refresh_devices()
                }
                KeyCode::Esc if app.can_quit() => return Ok(()),
                _ => {}
            }
        }
    }
}

fn run_serial_app(mut app: App) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    serial_render(&app)?;
    let mut last_render = Instant::now();

    loop {
        app.poll_install_events();
        app.refresh_system_status_if_due();
        app.auto_reboot_if_due();

        let installing = app.screen() == Screen::Installing;
        if installing && last_render.elapsed() >= Duration::from_secs(1) {
            serial_render(&app)?;
            last_render = Instant::now();
        }

        while let Ok(line) = rx.try_recv() {
            if serial_handle_command(&mut app, &line) {
                return Ok(());
            }
            serial_render(&app)?;
            last_render = Instant::now();
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn serial_handle_command(app: &mut App, line: &str) -> bool {
    let command = line.trim();
    let lower = command.to_ascii_lowercase();

    if matches!(lower.as_str(), "q" | "quit") && app.can_quit() {
        return true;
    }

    if app.has_warning() {
        if lower == "p" {
            app.start_proxy_config();
        } else {
            app.dismiss_warning();
        }
        return false;
    }

    match app.screen() {
        Screen::Welcome => {
            if command.is_empty() || matches!(lower.as_str(), "enter" | "next") {
                app.activate_selected();
            }
        }
        Screen::ImageSelect => {
            if serial_select_index(
                command,
                app.catalog().images.len(),
                app.image_index(),
                || app.next(),
            ) {
                app.activate_selected();
            } else {
                serial_handle_list_command(app, &lower);
            }
        }
        Screen::CustomImageSelect => {
            if serial_select_index(
                command,
                app.custom_images().len(),
                app.custom_image_index(),
                || app.next(),
            ) {
                app.activate_selected();
            } else {
                serial_handle_list_command(app, &lower);
            }
        }
        Screen::TargetSelect => {
            if serial_select_index(command, app.devices().len(), app.target_index(), || {
                app.next()
            }) {
                app.activate_selected();
            } else {
                serial_handle_list_command(app, &lower);
            }
        }
        Screen::Confirm => match lower.as_str() {
            "" | "enter" | "y" | "yes" | "flash" => app.activate_selected(),
            "b" | "back" => app.back(),
            _ => {}
        },
        Screen::Installing => {}
        Screen::Complete | Screen::Error => match lower.as_str() {
            "" | "enter" => app.activate_selected(),
            "r" | "restart" => app.start_over(),
            _ => {}
        },
        Screen::ProxyConfig => serial_handle_proxy_config_command(app, command, &lower),
    }

    false
}

fn serial_handle_proxy_config_command(app: &mut App, command: &str, lower: &str) {
    if let Some(time) = command.strip_prefix("time ") {
        app.set_proxy_time_input(time.trim().to_string());
        app.submit_proxy_config();
        return;
    }
    if matches!(lower, "ti" | "t" | "ti proxy" | "use ti proxy") {
        app.select_ti_proxy();
        if !matches!(app.proxy_config_field(), app::ProxyConfigField::Time) {
            app.submit_proxy_config();
        }
        return;
    }
    if matches!(lower, "manual" | "m" | "manual proxy") {
        app.select_manual_proxy();
        return;
    }
    if let Some(proxy) = command.strip_prefix("proxy ") {
        app.select_manual_proxy();
        app.set_proxy_input(proxy.trim().to_string());
        if !matches!(app.proxy_config_field(), app::ProxyConfigField::Time) {
            app.submit_proxy_config();
        }
        return;
    }

    match lower {
        "" | "enter" => app.submit_proxy_config(),
        "n" | "next" | "tab" | "down" => app.next_proxy_field(),
        "p" | "prev" | "previous" | "up" => app.previous_proxy_field(),
        "left" | "right" => app.toggle_proxy_mode(),
        "b" | "back" | "esc" | "cancel" => app.cancel_proxy_config(),
        _ => match app.proxy_config_field() {
            app::ProxyConfigField::Time => {
                app.set_proxy_time_input(command.to_string());
                app.submit_proxy_config();
            }
            app::ProxyConfigField::ProxyChoice => {
                app.warn_proxy_choice_command();
            }
            app::ProxyConfigField::Proxy => {
                app.set_proxy_input(command.to_string());
                app.submit_proxy_config();
            }
        },
    }
}

fn serial_select_index(
    command: &str,
    len: usize,
    mut current: usize,
    mut next: impl FnMut(),
) -> bool {
    let Ok(selected) = command.parse::<usize>() else {
        return false;
    };
    let Some(index) = selected.checked_sub(1) else {
        return false;
    };
    if index >= len {
        return false;
    }
    while current != index {
        next();
        current = (current + 1) % len;
    }
    true
}

fn serial_handle_list_command(app: &mut App, command: &str) {
    match command {
        "" | "enter" => app.activate_selected(),
        "n" | "next" | "j" | "down" => app.next(),
        "p" | "prev" | "previous" | "k" | "up" => app.previous(),
        "b" | "back" => app.back(),
        "r" | "refresh" => app.refresh_devices(),
        _ => {}
    }
}

fn serial_render(app: &App) -> anyhow::Result<()> {
    let mut out = io::stdout();
    write!(out, "\r\n\r\n=== TOBI {:?} ===\r\n", app.screen())?;

    if app.has_warning() {
        let warning = app.warning().unwrap_or_default();
        write!(out, "\r\nWARNING:\r\n{}\r\n", warning)?;
        write!(
            out,
            "\r\nPress Enter to continue, P to configure proxy, or Q to quit.\r\n"
        )?;
        out.flush()?;
        return Ok(());
    }

    write!(out, "Status: {}\r\n", app.status())?;
    match app.screen() {
        Screen::Welcome => {
            write!(out, "\r\nPress Enter to choose an image, or Q to quit.\r\n")?;
        }
        Screen::ImageSelect => {
            write!(out, "\r\nImages:\r\n")?;
            for (index, image) in app.catalog().images.iter().enumerate() {
                let marker = if index == app.image_index() { ">" } else { " " };
                write!(out, "{} {:2}. {}\r\n", marker, index + 1, image.name)?;
            }
            write!(
                out,
                "\r\nType a number, Enter to select, N/P to move, R to rescan, Q to quit.\r\n"
            )?;
        }
        Screen::CustomImageSelect => {
            write!(out, "\r\nCustom images:\r\n")?;
            for (index, image) in app.custom_images().iter().enumerate() {
                let marker = if index == app.custom_image_index() {
                    ">"
                } else {
                    " "
                };
                write!(out, "{} {:2}. {}\r\n", marker, index + 1, image.name)?;
            }
            write!(
                out,
                "\r\nType a number, Enter to select, N/P to move, R to rescan.\r\n"
            )?;
        }
        Screen::TargetSelect => {
            write!(out, "\r\nTargets:\r\n")?;
            for (index, target) in app.devices().iter().enumerate() {
                let marker = if index == app.target_index() {
                    ">"
                } else {
                    " "
                };
                write!(
                    out,
                    "{} {:2}. {}  {}  {}\r\n",
                    marker,
                    index + 1,
                    target.name,
                    target.path.display(),
                    serial_format_bytes(target.size_bytes)
                )?;
            }
            write!(
                out,
                "\r\nType a number, Enter to select, N/P to move, R to rescan.\r\n"
            )?;
        }
        Screen::Confirm => {
            write!(out, "\r\nImage: {}\r\n", serial_selected_image_name(app))?;
            write!(out, "Target: {}\r\n", serial_selected_target_name(app))?;
            write!(
                out,
                "\r\nType YES or press Enter to flash. Type B to go back.\r\n"
            )?;
        }
        Screen::Installing => {
            if let Some(progress) = app.progress() {
                write!(out, "\r\nPhase: {}\r\n", progress.phase)?;
                write!(
                    out,
                    "Written: {}\r\n",
                    serial_format_progress(progress.current, progress.total)
                )?;
                if let Some(current) = progress.source_current {
                    write!(
                        out,
                        "Downloaded: {}\r\n",
                        serial_format_progress(current, progress.source_total)
                    )?;
                }
            }
            write!(
                out,
                "\r\nInstall is running. Ctrl-C exits this serial UI only.\r\n"
            )?;
        }
        Screen::Complete | Screen::Error => {
            write!(out, "\r\n{}\r\n", app.status())?;
            write!(
                out,
                "\r\nPress Enter to continue, R to restart, or Q to quit.\r\n"
            )?;
        }
        Screen::ProxyConfig => {
            if let Some(warning) = app.warning() {
                write!(out, "\r\nCatalog load failed:\r\n{}\r\n", warning)?;
            }
            write!(
                out,
                "\r\nUTC time: [{}]\r\nProxy mode: {}\r\nProxy: [{}]\r\n",
                serial_input_display(app.proxy_time_input(), 19),
                match app.proxy_mode() {
                    app::ProxyMode::Ti => "TI proxy",
                    app::ProxyMode::Manual => "manual",
                },
                serial_input_display(app.active_proxy_url_for_display(), 36)
            )?;
            write!(
                out,
                "\r\nType 'time YYYY-MM-DD HH:MM:SS', then 'ti' or 'manual'. For manual, type 'proxy http://host:port'. Press Enter to submit the active field, N/P to switch fields, or B to cancel.\r\n"
            )?;
        }
    }

    out.flush()?;
    Ok(())
}

fn serial_input_display(value: &str, min_width: usize) -> String {
    if value.is_empty() {
        return " ".repeat(min_width);
    }

    let mut value = value.to_string();
    if value.len() < min_width {
        value.push_str(&" ".repeat(min_width - value.len()));
    }
    value
}

fn serial_selected_image_name(app: &App) -> String {
    app.selected_image()
        .map(|image| image.name.clone())
        .unwrap_or_else(|| "none".to_string())
}

fn serial_selected_target_name(app: &App) -> String {
    app.selected_target()
        .map(|target| format!("{} ({})", target.name, target.path.display()))
        .unwrap_or_else(|| "none".to_string())
}

fn serial_format_progress(current: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => format!(
            "{} / {}",
            serial_format_bytes(Some(current)),
            serial_format_bytes(Some(total))
        ),
        None => serial_format_bytes(Some(current)),
    }
}

fn serial_format_bytes(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return "unknown".to_string();
    };
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
