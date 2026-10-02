use std::fs;
#[cfg(any(target_os = "linux", test))]
use std::io::Write;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Context;
#[cfg(any(target_os = "linux", test))]
use anyhow::anyhow;

use crate::device::{InstallTarget, TargetKind};

const ARMBIAN_EMMC_UENV: &str = concat!(
    "bootpart=0:1\n",
    "bootdir=\n",
    "finduuid=part uuid mmc 0:2 uuid\n",
    "get_rd_mmc=load mmc ${bootpart} ${rdaddr} uInitrd\n",
    "uenvcmd=setenv mmcdev 0;setenv boot mmc;run get_rd_mmc;setenv rd_spec ${rdaddr}:${filesize};setexpr fdtfile sub ti/ti ti;run bootcmd_ti_mmc\n",
);

const TI_YOCTO_EMMC_UENV: &str = concat!(
    "mmcdev=0\n",
    "bootpart=0:2\n",
    "finduuid=part uuid mmc ${bootpart} uuid\n",
);

const TOBI_RECOVERY_UENVCMD: &str = "setexpr fdtfile sub ti/ti ti; run bootcmd_ti_mmc";
const TOBI_RECOVERY_OPTARGS: &str = "vt.global_cursor_default=1 console=ttyS2,115200n8 console=tty0 quiet loglevel=1 tobi.ttys=/dev/tty0,/dev/ttyS2";
#[cfg(any(target_os = "linux", test))]
const MAX_BOOT0_BOOTSTRAP_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootPatchReport {
    pub status: BootPatchStatus,
    pub summary: String,
    pub details: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootPatchStatus {
    Patched,
    AlreadyConfigured,
    Skipped,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmmcBootPartition {
    UserArea,
    Boot0,
}

impl EmmcBootPartition {
    #[cfg(any(target_os = "linux", test))]
    fn enable_argument(self) -> &'static str {
        match self {
            Self::UserArea => "7",
            Self::Boot0 => "1",
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn partition_config(self) -> u8 {
        match self {
            Self::UserArea => 0x78,
            Self::Boot0 => 0x48,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn label(self) -> &'static str {
        match self {
            Self::UserArea => "user-area",
            Self::Boot0 => "boot0",
        }
    }
}

impl BootPatchReport {
    pub fn patched(summary: impl Into<String>, details: Vec<String>) -> Self {
        Self {
            status: BootPatchStatus::Patched,
            summary: summary.into(),
            details,
        }
    }

    pub fn already_configured(summary: impl Into<String>, details: Vec<String>) -> Self {
        Self {
            status: BootPatchStatus::AlreadyConfigured,
            summary: summary.into(),
            details,
        }
    }

    pub fn skipped(summary: impl Into<String>, details: Vec<String>) -> Self {
        Self {
            status: BootPatchStatus::Skipped,
            summary: summary.into(),
            details,
        }
    }

    pub fn warning(summary: impl Into<String>, details: Vec<String>) -> Self {
        Self {
            status: BootPatchStatus::Warning,
            summary: summary.into(),
            details,
        }
    }

    pub fn phase_message(&self) -> String {
        match self.status {
            BootPatchStatus::Patched => format!("Boot patch complete: {}", self.summary),
            BootPatchStatus::AlreadyConfigured => {
                format!("Boot patch already applied: {}", self.summary)
            }
            BootPatchStatus::Skipped => format!("Boot patch skipped: {}", self.summary),
            BootPatchStatus::Warning => format!("Boot patch warning: {}", self.summary),
        }
    }

    pub fn final_message(&self) -> String {
        let label = match self.status {
            BootPatchStatus::Patched => "applied",
            BootPatchStatus::AlreadyConfigured => "already configured",
            BootPatchStatus::Skipped => "not needed",
            BootPatchStatus::Warning => "warning",
        };
        let mut message = format!("Boot patch: {label} - {}", self.summary);
        for detail in &self.details {
            message.push('\n');
            message.push_str("  ");
            message.push_str(detail);
        }
        message
    }
}

pub fn target_needs_boot_patch(target: &InstallTarget) -> bool {
    target.kind == TargetKind::Emmc
}

pub fn patch_installed_boot_media(target: &InstallTarget) -> BootPatchReport {
    patch_installed_boot_media_with_boot_partition(target, EmmcBootPartition::UserArea)
}

pub fn patch_installed_boot_media_with_boot_partition(
    target: &InstallTarget,
    boot_partition: EmmcBootPartition,
) -> BootPatchReport {
    if !target_needs_boot_patch(target) {
        return BootPatchReport::skipped(
            format!(
                "{} installs boot as written",
                target_kind_label(target.kind)
            ),
            Vec::new(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        patch_installed_boot_media_linux(target, boot_partition)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = boot_partition;
        BootPatchReport::warning(
            "post-flash boot patching is only available on Linux",
            vec![format!("Target: {}", target.path.display())],
        )
    }
}

#[cfg(target_os = "linux")]
fn patch_installed_boot_media_linux(
    target: &InstallTarget,
    boot_partition: EmmcBootPartition,
) -> BootPatchReport {
    let Some(partition) = boot_partition_path(target) else {
        return BootPatchReport::warning(
            "could not determine the installed boot partition",
            vec![format!("Target: {}", target.path.display())],
        );
    };

    reread_partition_table(&target.path);
    if let Err(error) = wait_for_path(&partition) {
        return BootPatchReport::warning(
            "boot partition did not appear after flashing",
            vec![
                format!("Partition: {}", partition.display()),
                format!("Details: {error:#}"),
            ],
        );
    }

    let mount_dir = boot_patch_mount_dir();
    if let Err(error) = fs::create_dir_all(&mount_dir) {
        return BootPatchReport::warning(
            "could not create boot patch mount directory",
            vec![
                format!("Directory: {}", mount_dir.display()),
                format!("Details: {error}"),
            ],
        );
    }

    let mounted = match MountedBootPartition::mount(&partition, &mount_dir) {
        Ok(mounted) => mounted,
        Err(error) => {
            let _ = fs::remove_dir(&mount_dir);
            return BootPatchReport::warning(
                "could not mount installed boot partition",
                vec![
                    format!("Partition: {}", partition.display()),
                    format!("Details: {error:#}"),
                ],
            );
        }
    };

    let recovery_layout =
        detect_boot_patch_style(&mount_dir, "") == Some(BootPatchStyle::TobiRecovery);
    let mut report = if boot_partition == EmmcBootPartition::Boot0 && !recovery_layout {
        BootPatchReport::warning(
            "eMMC boot0 bootstrap requires a recognized TOBI recovery image",
            vec!["The installed image was written, but its SPL was not automatically prepared for eMMC boot.".to_string()],
        )
    } else {
        patch_mounted_boot_partition(&mount_dir, &partition, &target.path).unwrap_or_else(|error| {
            BootPatchReport::warning(
                "could not update installed boot files",
                vec![
                    format!("Partition: {}", partition.display()),
                    format!("Details: {error:#}"),
                ],
            )
        })
    };

    if boot_partition == EmmcBootPartition::Boot0
        && matches!(
            report.status,
            BootPatchStatus::Patched | BootPatchStatus::AlreadyConfigured
        )
    {
        match prepare_boot0_bootstrap(&mount_dir, &target.path) {
            Ok(bootstrap) => {
                if bootstrap.changed {
                    report.status = BootPatchStatus::Patched;
                }
                report.details.extend(bootstrap.details);
            }
            Err(error) => {
                report.details.push(format!("Details: {error:#}"));
                report =
                    BootPatchReport::warning("eMMC boot0 bootstrap is incomplete", report.details);
            }
        }
    }

    match mounted.unmount() {
        Ok(()) => {
            match report.status {
                BootPatchStatus::Patched | BootPatchStatus::AlreadyConfigured => {
                    report
                        .details
                        .push(format!("Unmounted {}", mount_dir.display()));
                }
                BootPatchStatus::Skipped | BootPatchStatus::Warning => {}
            }
            let _ = fs::remove_dir(&mount_dir);
            finish_emmc_boot_configuration(report, &target.path, boot_partition, run_mmc_command)
        }
        Err(error) => {
            let mut details = report.details;
            details.push(format!("Unmount failed: {error:#}"));
            BootPatchReport::warning("boot partition may still be mounted", details)
        }
    }
}

fn patch_mounted_boot_partition(
    boot_dir: &Path,
    partition: &Path,
    target: &Path,
) -> anyhow::Result<BootPatchReport> {
    let uenv_path = boot_dir.join("uEnv.txt");
    let original = match fs::read_to_string(&uenv_path) {
        Ok(original) => original,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", uenv_path.display()));
        }
    };

    let Some(style) = detect_boot_patch_style(boot_dir, &original) else {
        return Ok(BootPatchReport::warning(
            "installed boot files were not recognized; eMMC boot setup is unverified",
            vec![format!("Mounted {}", partition.display())],
        ));
    };

    let plan = patch_plan(style, &original);
    let mut changed = false;
    let mut details = vec![format!("Mounted {}", partition.display())];

    if original == plan.content {
        details.push(format!("Verified {}", uenv_path.display()));
    } else {
        fs::write(&uenv_path, &plan.content)
            .with_context(|| format!("failed to write {}", uenv_path.display()))?;
        details.push(format!("Updated {}", uenv_path.display()));
        changed = true;
    }

    if style == BootPatchStyle::ArmbianOrTiDebian {
        let extlinux_report = patch_armbian_extlinux(boot_dir, target)?;
        changed |= extlinux_report.changed;
        details.extend(extlinux_report.details);
    } else if style == BootPatchStyle::TobiRecovery {
        let extlinux_report = repair_legacy_recovery_extlinux(boot_dir, target, &plan.content)?;
        changed |= extlinux_report.changed;
        details.extend(extlinux_report.details);
    }

    details.push(plan.detail.to_string());

    if !changed {
        return Ok(BootPatchReport::already_configured(plan.summary, details));
    }

    Ok(BootPatchReport::patched(plan.summary, details))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BootPatchStyle {
    TobiRecovery,
    ArmbianOrTiDebian,
    TiYocto,
}

struct PatchPlan {
    content: String,
    summary: &'static str,
    detail: &'static str,
}

fn patch_plan(style: BootPatchStyle, original: &str) -> PatchPlan {
    match style {
        BootPatchStyle::TobiRecovery => PatchPlan {
            content: tobi_recovery_uenv(original),
            summary: "updated TOBI recovery boot files for eMMC",
            detail: "Preserved /recovery boot paths and TOBI arguments; set mmcdev=0, bootpart=0:1, and rootfs lookup=mmc 0:2.",
        },
        BootPatchStyle::ArmbianOrTiDebian => PatchPlan {
            content: ARMBIAN_EMMC_UENV.to_string(),
            summary: "updated boot files for eMMC boot on Armbian/TI Debian images",
            detail: "Set bootpart=0:1, rootfs lookup=mmc 0:2, mmcdev=0, and added an extlinux eMMC fallback.",
        },
        BootPatchStyle::TiYocto => PatchPlan {
            content: TI_YOCTO_EMMC_UENV.to_string(),
            summary: "updated uEnv.txt for eMMC boot on TI Yocto images",
            detail: "Set mmcdev=0 and rootfs bootpart=0:2.",
        },
    }
}

fn tobi_recovery_uenv(original: &str) -> String {
    let recovery_environment_intact = uenv_value(original, "bootdir") == Some("/recovery")
        && uenv_value(original, "name_initramfs") == Some("recovery/uInitrd");
    let mut content = original.to_string();
    for (key, value) in [
        ("mmcdev", "0"),
        ("bootpart", "0:1"),
        ("finduuid", "part uuid mmc 0:2 uuid"),
        ("bootdir", "/recovery"),
        ("name_initramfs", "recovery/uInitrd"),
    ] {
        content = set_uenv_value(&content, key, value);
    }
    if !recovery_environment_intact || uenv_value(&content, "uenvcmd").is_none() {
        let recovery_command = if uenv_value(&content, "tobi_set_boot_source").is_some() {
            format!("run tobi_set_boot_source; {TOBI_RECOVERY_UENVCMD}")
        } else {
            TOBI_RECOVERY_UENVCMD.to_string()
        };
        content = set_uenv_value(&content, "uenvcmd", &recovery_command);
        content = remove_uenv_key(&content, "get_rd_mmc");
    }
    if uenv_value(&content, "optargs").is_none() {
        content = set_uenv_value(&content, "optargs", TOBI_RECOVERY_OPTARGS);
    }
    content
}

fn uenv_value<'a>(content: &'a str, key: &str) -> Option<&'a str> {
    content.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then_some(value.trim())
    })
}

fn set_uenv_value(content: &str, key: &str, value: &str) -> String {
    let mut updated = Vec::new();
    let mut found = false;
    for line in content.lines() {
        if line
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key)
        {
            if !found {
                updated.push(format!("{key}={value}"));
                found = true;
            }
        } else {
            updated.push(line.to_string());
        }
    }
    if !found {
        updated.push(format!("{key}={value}"));
    }
    format!("{}\n", updated.join("\n"))
}

fn remove_uenv_key(content: &str, key: &str) -> String {
    let lines = content
        .lines()
        .filter(|line| {
            !line
                .split_once('=')
                .is_some_and(|(name, _)| name.trim() == key)
        })
        .collect::<Vec<_>>();
    format!("{}\n", lines.join("\n"))
}

#[cfg(any(target_os = "linux", test))]
fn finish_emmc_boot_configuration(
    mut report: BootPatchReport,
    target: &Path,
    boot_partition: EmmcBootPartition,
    run: impl FnMut(&[&str], &Path) -> anyhow::Result<String>,
) -> BootPatchReport {
    if !matches!(
        report.status,
        BootPatchStatus::Patched | BootPatchStatus::AlreadyConfigured
    ) {
        return report;
    }
    match configure_emmc_boot(target, boot_partition, run) {
        Ok(configuration) => {
            if configuration.changed {
                report.status = BootPatchStatus::Patched;
            }
            report.details.extend(configuration.details);
            report
        }
        Err(error) => {
            report.details.push(format!("Details: {error:#}"));
            BootPatchReport::warning("eMMC boot configuration is incomplete", report.details)
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn configure_emmc_boot(
    target: &Path,
    boot_partition: EmmcBootPartition,
    mut run: impl FnMut(&[&str], &Path) -> anyhow::Result<String>,
) -> anyhow::Result<FilePatchReport> {
    let before = run(&["extcsd", "read"], target)?;
    let partition_config = ext_csd_byte(&before, "PARTITION_CONFIG")?;
    let boot_bus_conditions = ext_csd_byte(&before, "BOOT_BUS_CONDITIONS")?;
    let mut changed = false;

    // Only change reversible boot configuration bytes; BOOT_ACK is bit 6.
    // A board-specific caller must prepare any required boot0 bootstrap first.
    // This helper never writes boot0/boot1 or the one-time H/W reset bits.
    if partition_config & 0x78 != boot_partition.partition_config() {
        run(
            &["bootpart", "enable", boot_partition.enable_argument(), "1"],
            target,
        )?;
        changed = true;
    }
    // single_backward = SDR, x1 = reset to x1 after boot, x8 = boot bus width.
    // This reset setting is not mmc-utils' irreversible `hwreset` command.
    if boot_bus_conditions != 0x02 {
        run(&["bootbus", "set", "single_backward", "x1", "x8"], target)?;
        changed = true;
    }

    let after = if changed {
        run(&["extcsd", "read"], target)?
    } else {
        before
    };
    let partition_config = ext_csd_byte(&after, "PARTITION_CONFIG")?;
    let boot_bus_conditions = ext_csd_byte(&after, "BOOT_BUS_CONDITIONS")?;
    if partition_config & 0x78 != boot_partition.partition_config() || boot_bus_conditions != 0x02 {
        return Err(anyhow!(
            "eMMC configuration readback failed on {}: PARTITION_CONFIG=0x{partition_config:02x}, BOOT_BUS_CONDITIONS=0x{boot_bus_conditions:02x}; expected {} boot with BOOT_ACK and x8 SDR/reset",
            target.display(),
            boot_partition.label()
        ));
    }
    Ok(FilePatchReport {
        changed,
        details: vec![format!(
            "Verified {} eMMC {} boot, BOOT_ACK=1, and x8 SDR boot bus with reset to x1.",
            target.display(),
            boot_partition.label()
        )],
    })
}

#[cfg(any(target_os = "linux", test))]
fn ext_csd_byte(output: &str, register: &str) -> anyhow::Result<u8> {
    let prefix = format!("[{register}:");
    let value = output
        .lines()
        .find_map(|line| line.split_once(&prefix).map(|(_, tail)| tail))
        .and_then(|tail| tail.split_once(']').map(|(value, _)| value.trim()))
        .and_then(|value| value.strip_prefix("0x"))
        .ok_or_else(|| anyhow!("mmc extcsd read did not report {register}"))?;
    u8::from_str_radix(value, 16)
        .with_context(|| format!("invalid {register} value in mmc extcsd read output"))
}

#[cfg(target_os = "linux")]
fn run_mmc_command(args: &[&str], target: &Path) -> anyhow::Result<String> {
    let description = format!("mmc {} {}", args.join(" "), target.display());
    let output = std::process::Command::new("mmc")
        .args(args)
        .arg(target)
        .output()
        .with_context(|| format!("failed to start {description}; install mmc-utils"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{description} exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn prepare_boot0_bootstrap(boot_dir: &Path, target: &Path) -> anyhow::Result<FilePatchReport> {
    use std::os::unix::fs::FileTypeExt;

    let source = ["tiboot3.bin", "TIBOOT3.BIN"]
        .iter()
        .map(|name| boot_dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("installed TOBI boot partition has no tiboot3.bin"))?;
    let (boot0, force_ro) = emmc_boot0_paths(target)?;
    if !fs::metadata(&boot0)
        .with_context(|| format!("failed to inspect {}", boot0.display()))?
        .file_type()
        .is_block_device()
    {
        return Err(anyhow!(
            "{} is not an eMMC boot block device",
            boot0.display()
        ));
    }
    let mut current =
        fs::File::open(&boot0).with_context(|| format!("failed to read {}", boot0.display()))?;
    let capacity = block_device_capacity(&current)?;
    validate_bootstrap_size(fs::metadata(&source)?.len(), capacity)?;
    let mut loader = Vec::new();
    fs::File::open(&source)?
        .take(MAX_BOOT0_BOOTSTRAP_SIZE + 1)
        .read_to_end(&mut loader)
        .with_context(|| format!("failed to read {}", source.display()))?;
    validate_bootstrap_size(loader.len() as u64, capacity)?;
    if bootstrap_matches(&mut current, &loader)? {
        return Ok(FilePatchReport {
            changed: false,
            details: vec![format!(
                "Verified {} matches installed {} ({} bytes).",
                boot0.display(),
                source.display(),
                loader.len()
            )],
        });
    }
    drop(current);

    let access = Boot0WriteAccess::enable(&force_ro)?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&boot0)
            .with_context(|| format!("failed to open {} for bootstrap writing", boot0.display()))?;
        write_boot0_bootstrap(&mut file, &loader, capacity)
    })();
    let restore = access.restore();
    match (result, restore) {
        (Err(error), Err(restore_error)) => {
            return Err(anyhow!(
                "{error:#}; could not restore boot0 write protection: {restore_error:#}"
            ));
        }
        (Err(error), _) => return Err(error),
        (_, Err(error)) => return Err(error),
        (Ok(()), Ok(())) => {}
    }
    Ok(FilePatchReport {
        changed: true,
        details: vec![format!(
            "Copied installed {} to {} offset 0 ({} bytes); flushed, verified readback, and restored force_ro.",
            source.display(),
            boot0.display(),
            loader.len()
        )],
    })
}

#[cfg(any(target_os = "linux", test))]
fn emmc_boot0_paths(target: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let index = name.strip_prefix("mmcblk").unwrap_or("");
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(anyhow!(
            "{} is not a whole eMMC device path",
            target.display()
        ));
    }
    let boot0_name = format!("{name}boot0");
    Ok((
        target.with_file_name(&boot0_name),
        Path::new("/sys/class/block")
            .join(boot0_name)
            .join("force_ro"),
    ))
}

#[cfg(target_os = "linux")]
fn block_device_capacity(file: &fs::File) -> anyhow::Result<u64> {
    use std::os::fd::AsRawFd;

    let mut capacity = 0_u64;
    // Linux BLKGETSIZE64 reads capacity; it does not alter the device.
    let result = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            0x8008_1272_u64 as libc::c_ulong,
            &mut capacity as *mut u64,
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to read boot0 block capacity");
    }
    Ok(capacity)
}

#[cfg(any(target_os = "linux", test))]
fn validate_bootstrap_size(loader_size: u64, capacity: u64) -> anyhow::Result<()> {
    if loader_size == 0 || loader_size > MAX_BOOT0_BOOTSTRAP_SIZE || loader_size > capacity {
        return Err(anyhow!(
            "invalid tiboot3.bin size {loader_size} bytes for boot0 capacity {capacity} bytes (maximum {MAX_BOOT0_BOOTSTRAP_SIZE})"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn bootstrap_matches(file: &mut fs::File, loader: &[u8]) -> anyhow::Result<bool> {
    file.seek(SeekFrom::Start(0))?;
    let mut buffer = [0_u8; 64 * 1024];
    for expected in loader.chunks(buffer.len()) {
        file.read_exact(&mut buffer[..expected.len()])
            .context("failed to read boot0 bootstrap bytes")?;
        if &buffer[..expected.len()] != expected {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(any(target_os = "linux", test))]
fn write_boot0_bootstrap(file: &mut fs::File, loader: &[u8], capacity: u64) -> anyhow::Result<()> {
    validate_bootstrap_size(loader.len() as u64, capacity)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(loader)
        .context("failed to write boot0 bootstrap")?;
    file.sync_all().context("failed to flush boot0 bootstrap")?;
    if !bootstrap_matches(file, loader)? {
        return Err(anyhow!(
            "boot0 bootstrap readback does not match installed tiboot3.bin"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
struct Boot0WriteAccess {
    force_ro: PathBuf,
    original: String,
    restored: bool,
}

#[cfg(any(target_os = "linux", test))]
impl Boot0WriteAccess {
    fn enable(path: &Path) -> anyhow::Result<Self> {
        let original = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if !matches!(original.trim(), "0" | "1") {
            return Err(anyhow!("unexpected force_ro value in {}", path.display()));
        }
        let access = Self {
            force_ro: path.to_path_buf(),
            original,
            restored: false,
        };
        fs::write(path, "0").with_context(|| {
            format!(
                "failed to enable bootstrap writes through {}",
                path.display()
            )
        })?;
        if fs::read_to_string(path)?.trim() != "0" {
            return Err(anyhow!(
                "{} did not enable bootstrap writing",
                path.display()
            ));
        }
        Ok(access)
    }

    fn restore(mut self) -> anyhow::Result<()> {
        fs::write(&self.force_ro, &self.original)
            .with_context(|| format!("failed to restore {}", self.force_ro.display()))?;
        if fs::read_to_string(&self.force_ro)?.trim() != self.original.trim() {
            return Err(anyhow!(
                "{} did not restore write protection",
                self.force_ro.display()
            ));
        }
        self.restored = true;
        Ok(())
    }
}

#[cfg(any(target_os = "linux", test))]
impl Drop for Boot0WriteAccess {
    fn drop(&mut self) {
        if !self.restored {
            let _ = fs::write(&self.force_ro, &self.original);
        }
    }
}

struct FilePatchReport {
    changed: bool,
    details: Vec<String>,
}

fn repair_legacy_recovery_extlinux(
    boot_dir: &Path,
    target: &Path,
    uenv: &str,
) -> anyhow::Result<FilePatchReport> {
    let path = boot_dir.join("extlinux/extlinux.conf");
    let existing = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FilePatchReport {
                changed: false,
                details: Vec::new(),
            });
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let root_spec = root_spec_for_emmc_rootfs(target, 2);
    // Repair only the exact fallback generated by the old TOBI Armbian
    // patcher. Preserve recovery menus and other custom extlinux files.
    if existing != armbian_extlinux_conf(&root_spec, "/uInitrd") {
        return Ok(FilePatchReport {
            changed: false,
            details: Vec::new(),
        });
    }
    let optargs = uenv_value(uenv, "optargs").unwrap_or(TOBI_RECOVERY_OPTARGS);
    let content = format!(
        concat!(
            "TIMEOUT 30\n",
            "DEFAULT tobi-recovery\n\n",
            "LABEL tobi-recovery\n",
            "  MENU LABEL TOBI eMMC recovery\n",
            "  LINUX /recovery/Image\n",
            "  INITRD /recovery/uInitrd\n",
            "  FDTDIR /recovery/dtb\n",
            "  APPEND {optargs} root={root_spec} rw rootfstype=ext4 rootwait\n",
        ),
        optargs = optargs,
        root_spec = root_spec,
    );
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(FilePatchReport {
        changed: true,
        details: vec![format!(
            "Restored {} to /recovery paths and TOBI boot arguments.",
            path.display()
        )],
    })
}

fn patch_armbian_extlinux(boot_dir: &Path, target: &Path) -> anyhow::Result<FilePatchReport> {
    let root_spec = root_spec_for_emmc_rootfs(target, 2);
    let initrd_path = select_armbian_initrd_path(boot_dir);
    let content = armbian_extlinux_conf(&root_spec, &initrd_path);
    let extlinux_dir = boot_dir.join("extlinux");
    let extlinux_path = extlinux_dir.join("extlinux.conf");

    fs::create_dir_all(&extlinux_dir)
        .with_context(|| format!("failed to create {}", extlinux_dir.display()))?;

    let existing = fs::read_to_string(&extlinux_path).ok();
    if existing.as_deref() == Some(content.as_str()) {
        return Ok(FilePatchReport {
            changed: false,
            details: vec![
                format!("Verified {}", extlinux_path.display()),
                format!("extlinux root={root_spec}"),
                format!("extlinux initrd={initrd_path}"),
            ],
        });
    }

    fs::write(&extlinux_path, content)
        .with_context(|| format!("failed to write {}", extlinux_path.display()))?;

    Ok(FilePatchReport {
        changed: true,
        details: vec![
            format!("Updated {}", extlinux_path.display()),
            format!("extlinux root={root_spec}"),
            format!("extlinux initrd={initrd_path}"),
        ],
    })
}

fn armbian_extlinux_conf(root_spec: &str, initrd_path: &str) -> String {
    format!(
        concat!(
            "TIMEOUT 30\n",
            "DEFAULT tobi-emmc\n",
            "\n",
            "LABEL tobi-emmc\n",
            "  MENU LABEL TOBI eMMC boot\n",
            "  LINUX /Image\n",
            "  INITRD {initrd_path}\n",
            "  FDTDIR /dtb\n",
            "  APPEND console=ttyS2,115200n8 root={root_spec} rw rootfstype=ext4 rootwait vt.global_cursor_default=0\n",
        ),
        initrd_path = initrd_path,
        root_spec = root_spec,
    )
}

fn select_armbian_initrd_path(boot_dir: &Path) -> String {
    let mut candidates = fs::read_dir(boot_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("initrd.img-"))
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .map(|name| format!("/{name}"))
        .unwrap_or_else(|| "/uInitrd".to_string())
}

fn root_spec_for_emmc_rootfs(target: &Path, partition_number: u8) -> String {
    match mbr_partuuid(target, partition_number) {
        Ok(Some(partuuid)) => format!("PARTUUID={partuuid}"),
        _ => fallback_root_device(target, partition_number),
    }
}

fn mbr_partuuid(target: &Path, partition_number: u8) -> anyhow::Result<Option<String>> {
    let mut file = fs::File::open(target)
        .with_context(|| format!("failed to open {} for PARTUUID detection", target.display()))?;
    let mut sector = [0_u8; 512];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut sector)?;

    if sector[510] != 0x55 || sector[511] != 0xaa {
        return Ok(None);
    }

    let partition_index = usize::from(partition_number.saturating_sub(1));
    if partition_index >= 4 {
        return Ok(None);
    }

    let entry_offset = 446 + partition_index * 16;
    let partition_type = sector[entry_offset + 4];
    if partition_type == 0 {
        return Ok(None);
    }

    let disk_signature = u32::from_le_bytes(sector[440..444].try_into().expect("mbr signature"));
    if disk_signature == 0 {
        return Ok(None);
    }

    Ok(Some(format!("{disk_signature:08x}-{partition_number:02x}")))
}

fn fallback_root_device(target: &Path, partition_number: u8) -> String {
    let Some(name) = target.file_name().and_then(|name| name.to_str()) else {
        return format!("/dev/mmcblk0p{partition_number}");
    };
    let suffix =
        if name.starts_with("mmcblk") || name.starts_with("nvme") || name.starts_with("loop") {
            format!("p{partition_number}")
        } else {
            partition_number.to_string()
        };
    format!("/dev/{name}{suffix}")
}

fn detect_boot_patch_style(boot_dir: &Path, uenv: &str) -> Option<BootPatchStyle> {
    // Inspect the payload before the generic uInitrd detector, including images
    // whose recovery uEnv.txt was overwritten by an older TOBI release.
    if boot_dir.join("recovery/Image").is_file() && boot_dir.join("recovery/uInitrd").is_file() {
        return Some(BootPatchStyle::TobiRecovery);
    }

    if path_exists_any(boot_dir, &["armbianEnv.txt", "ARMBIANENV.TXT"])
        || uenv.contains("uInitrd")
        || uenv.contains("get_rd_mmc")
    {
        return Some(BootPatchStyle::ArmbianOrTiDebian);
    }

    if path_exists_any(
        boot_dir,
        &[
            "EFI/BOOT/GRUB.CFG",
            "EFI/BOOT/grub.cfg",
            "efi/boot/grub.cfg",
            "TIBOOT3.BIN",
            "tiboot3.bin",
        ],
    ) || uenv.contains("mmcdev")
        || uenv.contains("bootpart")
    {
        return Some(BootPatchStyle::TiYocto);
    }

    None
}

fn path_exists_any(base: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| base.join(name).exists())
}

#[cfg(target_os = "linux")]
fn boot_partition_path(target: &InstallTarget) -> Option<PathBuf> {
    if let Some(partition) = target
        .partitions
        .iter()
        .find(|partition| partition.name.eq_ignore_ascii_case("boot"))
    {
        return Some(partition.path.clone());
    }

    first_partition_path(&target.path)
}

#[cfg(target_os = "linux")]
fn first_partition_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let suffix =
        if name.starts_with("mmcblk") || name.starts_with("nvme") || name.starts_with("loop") {
            "p1"
        } else {
            "1"
        };
    Some(path.with_file_name(format!("{name}{suffix}")))
}

fn target_kind_label(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Emmc => "eMMC",
        TargetKind::Sd => "SD",
        TargetKind::Usb => "USB/storage",
        TargetKind::Nvme => "NVMe",
        TargetKind::File => "file target",
        TargetKind::Unknown => "block target",
    }
}

#[cfg(target_os = "linux")]
fn boot_patch_mount_dir() -> PathBuf {
    let base = if Path::new("/run").is_dir() {
        Path::new("/run")
    } else {
        Path::new("/tmp")
    };
    base.join(format!("tobi-installed-boot-{}", std::process::id()))
}

#[cfg(target_os = "linux")]
fn reread_partition_table(target: &Path) {
    let commands: &[(&str, &[&str])] = &[
        ("blockdev", &["--rereadpt"]),
        ("partprobe", &[]),
        ("partx", &["-u"]),
    ];

    for (command, args) in commands {
        let mut child = std::process::Command::new(command);
        child.args(*args).arg(target);
        if child
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            break;
        }
    }
}

#[cfg(target_os = "linux")]
fn wait_for_path(path: &Path) -> anyhow::Result<()> {
    for _ in 0..50 {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(anyhow!(
        "{} was not present after 5 seconds",
        path.display()
    ))
}

#[cfg(target_os = "linux")]
struct MountedBootPartition {
    mount_dir: PathBuf,
    mounted: bool,
}

#[cfg(target_os = "linux")]
impl MountedBootPartition {
    fn mount(partition: &Path, mount_dir: &Path) -> anyhow::Result<Self> {
        let vfat_result = run_mount_command(
            std::process::Command::new("mount")
                .arg("-t")
                .arg("vfat")
                .arg("-o")
                .arg("rw")
                .arg(partition)
                .arg(mount_dir),
        );
        if vfat_result.is_ok() {
            return Ok(Self {
                mount_dir: mount_dir.to_path_buf(),
                mounted: true,
            });
        }

        let generic_result = run_mount_command(
            std::process::Command::new("mount")
                .arg("-o")
                .arg("rw")
                .arg(partition)
                .arg(mount_dir),
        );
        if generic_result.is_ok() {
            return Ok(Self {
                mount_dir: mount_dir.to_path_buf(),
                mounted: true,
            });
        }

        Err(anyhow!(
            "{}; fallback mount also failed: {}",
            vfat_result.expect_err("vfat mount failed"),
            generic_result.expect_err("generic mount failed")
        ))
    }

    fn unmount(mut self) -> anyhow::Result<()> {
        let output = std::process::Command::new("umount")
            .arg(&self.mount_dir)
            .output()
            .context("failed to start umount")?;
        if output.status.success() {
            self.mounted = false;
            Ok(())
        } else {
            Err(anyhow!(
                "umount exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for MountedBootPartition {
    fn drop(&mut self) {
        if self.mounted {
            let _ = std::process::Command::new("umount")
                .arg(&self.mount_dir)
                .status();
        }
    }
}

#[cfg(target_os = "linux")]
fn run_mount_command(command: &mut std::process::Command) -> anyhow::Result<()> {
    let output = command.output().context("failed to start mount")?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "mount exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::InstallTarget;

    const TOBI_SD_UENV: &str = concat!(
        "bootpart=1:1\n",
        "bootdir=/recovery\n",
        "finduuid=part uuid ${boot} 1:2 uuid\n\n",
        "name_initramfs=recovery/uInitrd\n\n",
        "uenvcmd=setexpr fdtfile sub ti/ti ti; run bootcmd_ti_mmc\n\n",
        "optargs=vt.global_cursor_default=1 console=ttyS2,115200n8 console=tty0 quiet loglevel=1 tobi.ttys=/dev/tty0,/dev/ttyS2\n",
    );

    #[test]
    fn tobi_recovery_patch_preserves_real_boot_configuration_and_custom_arguments() {
        let dir = test_recovery_boot_partition();
        let original = format!("{TOBI_SD_UENV}# Local display settings\nextra_display=hdmi\n")
            .replace(
                "tobi.ttys=/dev/tty0,/dev/ttyS2",
                "tobi.ttys=/dev/tty0,/dev/ttyS2 tobi.custom=1",
            );
        fs::write(dir.path().join("uEnv.txt"), &original).expect("uenv");
        fs::create_dir_all(dir.path().join("extlinux")).expect("extlinux dir");
        let recovery_extlinux =
            "LABEL recovery\n  LINUX /recovery/Image\n  INITRD /recovery/uInitrd\n";
        fs::write(dir.path().join("extlinux/extlinux.conf"), recovery_extlinux)
            .expect("existing recovery fallback");
        let target = test_target_with_mbr(dir.path());

        let report = patch_mounted_boot_partition(dir.path(), Path::new("/dev/testp1"), &target)
            .expect("patch");
        assert_eq!(report.status, BootPatchStatus::Patched);
        let patched = fs::read_to_string(dir.path().join("uEnv.txt")).expect("patched uenv");
        assert_eq!(uenv_value(&patched, "mmcdev"), Some("0"));
        assert_eq!(uenv_value(&patched, "bootpart"), Some("0:1"));
        assert_eq!(
            uenv_value(&patched, "finduuid"),
            Some("part uuid mmc 0:2 uuid")
        );
        for key in [
            "bootdir",
            "name_initramfs",
            "uenvcmd",
            "optargs",
            "extra_display",
        ] {
            assert_eq!(
                uenv_value(&patched, key),
                uenv_value(&original, key),
                "{key}"
            );
        }
        assert!(patched.contains("# Local display settings"));
        assert_eq!(
            fs::read_to_string(dir.path().join("extlinux/extlinux.conf")).expect("extlinux"),
            recovery_extlinux
        );
        let repeated = patch_mounted_boot_partition(dir.path(), Path::new("/dev/testp1"), &target)
            .expect("repeat patch");
        assert_eq!(repeated.status, BootPatchStatus::AlreadyConfigured);
    }

    #[test]
    fn tobi_recovery_payload_restores_an_environment_overwritten_by_old_patcher() {
        let dir = test_recovery_boot_partition();
        fs::write(dir.path().join("uEnv.txt"), ARMBIAN_EMMC_UENV).expect("old corrupted uenv");
        let target = test_target_with_mbr(dir.path());

        let report = patch_mounted_boot_partition(dir.path(), Path::new("/dev/testp1"), &target)
            .expect("restore recovery");
        assert_eq!(report.status, BootPatchStatus::Patched);
        let restored = fs::read_to_string(dir.path().join("uEnv.txt")).expect("restored uenv");
        assert_eq!(uenv_value(&restored, "bootdir"), Some("/recovery"));
        assert_eq!(
            uenv_value(&restored, "name_initramfs"),
            Some("recovery/uInitrd")
        );
        assert_eq!(
            uenv_value(&restored, "uenvcmd"),
            Some(TOBI_RECOVERY_UENVCMD)
        );
        assert_eq!(
            uenv_value(&restored, "optargs"),
            Some(TOBI_RECOVERY_OPTARGS)
        );
        assert!(uenv_value(&restored, "get_rd_mmc").is_none());
        assert!(!dir.path().join("extlinux").exists());
    }

    #[test]
    fn emmc_recovery_retains_boot_card_identity_for_future_updates() {
        let original = include_str!("../../meta-tobi/recipes-bsp/bootfiles/files/uEnv.txt");
        let patched = tobi_recovery_uenv(original);
        assert_eq!(uenv_value(&patched, "mmcdev"), Some("0"));
        assert_eq!(uenv_value(&patched, "bootpart"), Some("0:1"));
        for key in ["tobi_set_boot_source", "uenvcmd"] {
            assert_eq!(uenv_value(&patched, key), uenv_value(original, key));
        }
        assert!(
            uenv_value(&patched, "uenvcmd")
                .unwrap()
                .contains("run tobi_set_boot_source")
        );

        let missing_command = remove_uenv_key(original, "uenvcmd");
        let repaired = tobi_recovery_uenv(&missing_command);
        assert!(
            uenv_value(&repaired, "uenvcmd")
                .unwrap()
                .starts_with("run tobi_set_boot_source;")
        );
    }

    #[test]
    fn tobi_recovery_repairs_exact_legacy_extlinux_fallback_without_deleting_it() {
        let dir = test_recovery_boot_partition();
        let target = test_target_with_mbr(dir.path());
        fs::write(dir.path().join("uEnv.txt"), ARMBIAN_EMMC_UENV).expect("old corrupted uenv");
        fs::create_dir_all(dir.path().join("extlinux")).expect("extlinux dir");
        let legacy = armbian_extlinux_conf("PARTUUID=1a2b3c4d-02", "/uInitrd");
        fs::write(dir.path().join("extlinux/extlinux.conf"), legacy).expect("legacy extlinux");

        patch_mounted_boot_partition(dir.path(), Path::new("/dev/testp1"), &target)
            .expect("restore recovery");
        let restored = fs::read_to_string(dir.path().join("extlinux/extlinux.conf"))
            .expect("restored fallback still exists");
        assert!(restored.contains("LINUX /recovery/Image"));
        assert!(restored.contains("INITRD /recovery/uInitrd"));
        assert!(restored.contains("FDTDIR /recovery/dtb"));
        assert!(restored.contains(TOBI_RECOVERY_OPTARGS));
        assert!(restored.contains("root=PARTUUID=1a2b3c4d-02"));
        assert!(!restored.contains("LINUX /Image"));
        assert!(!restored.contains("INITRD /uInitrd"));
    }

    #[test]
    fn missing_recovery_payload_does_not_override_distro_detection() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("recovery")).expect("recovery dir");
        fs::write(dir.path().join("recovery/Image"), "kernel").expect("kernel");
        assert_eq!(
            detect_boot_patch_style(dir.path(), TOBI_SD_UENV),
            Some(BootPatchStyle::ArmbianOrTiDebian)
        );
    }

    #[test]
    fn emmc_boot_configuration_changes_only_reversible_settings_and_verifies_readback() {
        let target = Path::new("/dev/mmcblk0");
        let mut calls = Vec::new();
        let mut reads = 0;
        let result = configure_emmc_boot(target, EmmcBootPartition::UserArea, |args, path| {
            assert_eq!(path, target);
            calls.push(args.join(" "));
            if args == ["extcsd", "read"] {
                reads += 1;
                Ok(test_extcsd(
                    if reads == 1 { 0x48 } else { 0x78 },
                    if reads == 1 { 0 } else { 2 },
                ))
            } else {
                Ok(String::new())
            }
        })
        .expect("configuration");
        assert!(result.changed);
        assert_eq!(
            calls,
            [
                "extcsd read",
                "bootpart enable 7 1",
                "bootbus set single_backward x1 x8",
                "extcsd read"
            ]
        );
    }

    #[test]
    fn emmc_boot_configuration_is_idempotent_when_readback_is_already_correct() {
        let mut calls = Vec::new();
        let result = configure_emmc_boot(
            Path::new("/dev/mmcblk0"),
            EmmcBootPartition::UserArea,
            |args, _| {
                calls.push(args.join(" "));
                Ok(test_extcsd(0x78, 0x02))
            },
        )
        .expect("configuration");
        assert!(!result.changed);
        assert_eq!(calls, ["extcsd read"]);
    }

    #[test]
    fn explicit_boot0_strategy_does_not_select_user_area_boot() {
        let mut calls = Vec::new();
        let mut reads = 0;
        configure_emmc_boot(
            Path::new("/dev/mmcblk0"),
            EmmcBootPartition::Boot0,
            |args, _| {
                calls.push(args.join(" "));
                if args == ["extcsd", "read"] {
                    reads += 1;
                    Ok(test_extcsd(if reads == 1 { 0x78 } else { 0x48 }, 2))
                } else {
                    Ok(String::new())
                }
            },
        )
        .expect("boot0 configuration");
        assert_eq!(calls, ["extcsd read", "bootpart enable 1 1", "extcsd read"]);
    }

    #[test]
    fn emmc_command_failure_or_stale_readback_cannot_report_boot_success() {
        for fail_command in [true, false] {
            let report =
                BootPatchReport::patched("boot files updated", vec!["Unmounted boot".to_string()]);
            let report = finish_emmc_boot_configuration(
                report,
                Path::new("/dev/mmcblk0"),
                EmmcBootPartition::UserArea,
                |args, _| {
                    if args == ["extcsd", "read"] {
                        Ok(test_extcsd(0x48, 0))
                    } else if fail_command {
                        Err(anyhow!("mock mmc command failed"))
                    } else {
                        Ok(String::new())
                    }
                },
            );
            assert_eq!(report.status, BootPatchStatus::Warning);
            assert!(report.summary.contains("incomplete"));
            assert!(
                report
                    .details
                    .iter()
                    .any(|detail| detail.contains(if fail_command {
                        "mock mmc command failed"
                    } else {
                        "readback failed"
                    }))
            );
        }
    }

    #[test]
    fn unreadable_or_missing_extcsd_registers_prevent_configuration_writes() {
        let mut calls = 0;
        let error = configure_emmc_boot(
            Path::new("/dev/mmcblk0"),
            EmmcBootPartition::UserArea,
            |_, _| {
                calls += 1;
                Ok("mmc-utils output without boot registers".to_string())
            },
        )
        .err()
        .expect("bad readback");
        assert_eq!(calls, 1);
        assert!(error.to_string().contains("PARTITION_CONFIG"));
    }

    #[test]
    fn bootstrap_writes_only_loader_bytes_at_offset_zero_and_checks_readback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("boot0.bin");
        fs::write(&path, [0xaa_u8; 256]).expect("boot0 fixture");
        let loader = b"installed TOBI tiboot3 bootstrap";
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("boot0 fixture");
        write_boot0_bootstrap(&mut file, loader, 256).expect("bootstrap");
        assert!(bootstrap_matches(&mut file, loader).expect("readback"));
        let bytes = fs::read(&path).expect("boot0 bytes");
        assert_eq!(&bytes[..loader.len()], loader);
        assert_eq!(&bytes[loader.len()..], &[0xaa_u8; 256][loader.len()..]);
        assert_eq!(bytes.len(), 256);
    }

    #[test]
    fn invalid_bootstrap_size_is_rejected_before_any_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("boot0.bin");
        let initial = [0xaa_u8; 16];
        fs::write(&path, initial).expect("boot0 fixture");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("boot0 fixture");
        assert!(write_boot0_bootstrap(&mut file, &[], 16).is_err());
        assert!(write_boot0_bootstrap(&mut file, &[0xbb_u8; 17], 16).is_err());
        assert!(validate_bootstrap_size(MAX_BOOT0_BOOTSTRAP_SIZE + 1, u64::MAX).is_err());
        assert_eq!(fs::read(path).expect("unchanged boot0"), initial);
    }

    #[test]
    fn temporary_boot0_write_access_restores_original_state_including_error_cleanup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("force_ro");
        for original in ["0\n", "1\n"] {
            fs::write(&path, original).expect("force_ro fixture");
            let access = Boot0WriteAccess::enable(&path).expect("enable");
            assert_eq!(fs::read_to_string(&path).expect("enabled"), "0");
            access.restore().expect("restore");
            assert_eq!(fs::read_to_string(&path).expect("restored"), original);
            let access = Boot0WriteAccess::enable(&path).expect("enable");
            drop(access);
            assert_eq!(fs::read_to_string(&path).expect("error cleanup"), original);
        }
    }

    #[test]
    fn boot0_paths_require_whole_mmc_devices() {
        assert_eq!(
            emmc_boot0_paths(Path::new("/dev/mmcblk0")).expect("whole eMMC"),
            (
                PathBuf::from("/dev/mmcblk0boot0"),
                PathBuf::from("/sys/class/block/mmcblk0boot0/force_ro")
            )
        );
        for path in [
            "/dev/mmcblk0p1",
            "/dev/mmcblk0boot0",
            "/dev/sda",
            "/dev/mmcblk",
        ] {
            assert!(emmc_boot0_paths(Path::new(path)).is_err(), "{path}");
        }
    }

    fn test_recovery_boot_partition() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("recovery")).expect("recovery dir");
        fs::write(dir.path().join("recovery/Image"), "kernel").expect("recovery kernel");
        fs::write(dir.path().join("recovery/uInitrd"), "initramfs").expect("recovery initramfs");
        dir
    }

    fn test_extcsd(partition_config: u8, boot_bus_conditions: u8) -> String {
        format!(
            "Boot configuration bytes [PARTITION_CONFIG: 0x{partition_config:02x}]\nBoot bus Conditions [BOOT_BUS_CONDITIONS: 0x{boot_bus_conditions:02x}]\n"
        )
    }

    #[test]
    fn armbian_patch_uses_emmc_mmc_index_and_rootfs_partition() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("armbianEnv.txt"), "rootfstype=ext4\n").expect("armbian env");
        fs::write(
            dir.path().join("uEnv.txt"),
            "bootpart=1:1\nfinduuid=part uuid ${boot} 1:2 uuid\nname_rd=uInitrd\n",
        )
        .expect("uenv");
        fs::write(dir.path().join("initrd.img-6.18-test"), "initrd").expect("initrd");
        let target = test_target_with_mbr(dir.path());

        let report = patch_mounted_boot_partition(dir.path(), Path::new("/dev/testp1"), &target)
            .expect("patch");

        assert_eq!(report.status, BootPatchStatus::Patched);
        let patched = fs::read_to_string(dir.path().join("uEnv.txt")).expect("patched uenv");
        assert!(patched.contains("bootpart=0:1"));
        assert!(patched.contains("finduuid=part uuid mmc 0:2 uuid"));
        assert!(patched.contains("setenv mmcdev 0"));
        assert!(patched.len() <= 256);
        let extlinux =
            fs::read_to_string(dir.path().join("extlinux/extlinux.conf")).expect("extlinux");
        assert!(extlinux.contains("LINUX /Image"));
        assert!(extlinux.contains("INITRD /initrd.img-6.18-test"));
        assert!(extlinux.contains("FDTDIR /dtb"));
        assert!(extlinux.contains("root=PARTUUID=1a2b3c4d-02"));
    }

    #[test]
    fn ti_yocto_patch_uses_emmc_mmc_index_and_second_partition() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("EFI/BOOT")).expect("efi dir");
        fs::write(dir.path().join("EFI/BOOT/GRUB.CFG"), "linux /Image\n").expect("grub");
        fs::write(dir.path().join("uEnv.txt"), "# empty by default\n").expect("uenv");
        let target = test_target_with_mbr(dir.path());

        let report = patch_mounted_boot_partition(dir.path(), Path::new("/dev/testp1"), &target)
            .expect("patch");

        assert_eq!(report.status, BootPatchStatus::Patched);
        let patched = fs::read_to_string(dir.path().join("uEnv.txt")).expect("patched uenv");
        assert!(patched.contains("mmcdev=0"));
        assert!(patched.contains("bootpart=0:2"));
        assert!(patched.contains("finduuid=part uuid mmc ${bootpart} uuid"));
    }

    #[test]
    fn unrecognized_emmc_boot_files_cannot_claim_boot_configuration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let report = patch_mounted_boot_partition(
            dir.path(),
            Path::new("/dev/mmcblk0p1"),
            Path::new("/dev/mmcblk0"),
        )
        .expect("inspect boot layout");
        assert_eq!(report.status, BootPatchStatus::Warning);
        assert!(report.summary.contains("unverified"));
    }

    #[test]
    fn non_emmc_targets_skip_patching() {
        let target = InstallTarget {
            id: "sd".to_string(),
            name: "SD".to_string(),
            path: PathBuf::from("/dev/mmcblk1"),
            size_bytes: None,
            kind: TargetKind::Sd,
            removable: true,
            partitions: Vec::new(),
            warning: None,
        };

        let report = patch_installed_boot_media(&target);

        assert_eq!(report.status, BootPatchStatus::Skipped);
    }

    #[test]
    fn already_configured_report_is_displayable() {
        let report = BootPatchReport::already_configured(
            "uEnv.txt already targets eMMC",
            vec!["Verified uEnv.txt".to_string()],
        );

        assert_eq!(report.status, BootPatchStatus::AlreadyConfigured);
        assert!(
            report
                .final_message()
                .contains("Boot patch: already configured")
        );
    }

    fn test_target_with_mbr(dir: &Path) -> PathBuf {
        let target = dir.join("target.img");
        let mut mbr = [0_u8; 512];
        mbr[440..444].copy_from_slice(&0x1a2b3c4d_u32.to_le_bytes());
        let part2 = 446 + 16;
        mbr[part2 + 4] = 0x83;
        mbr[510] = 0x55;
        mbr[511] = 0xaa;
        fs::write(&target, mbr).expect("target mbr");
        target
    }
}
