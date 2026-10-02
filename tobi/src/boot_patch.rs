use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Context;
#[cfg(target_os = "linux")]
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
        patch_installed_boot_media_linux(target)
    }

    #[cfg(not(target_os = "linux"))]
    {
        BootPatchReport::warning(
            "post-flash boot patching is only available on Linux",
            vec![format!("Target: {}", target.path.display())],
        )
    }
}

#[cfg(target_os = "linux")]
fn patch_installed_boot_media_linux(target: &InstallTarget) -> BootPatchReport {
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

    let mut report = patch_mounted_boot_partition(&mount_dir, &partition, &target.path)
        .unwrap_or_else(|error| {
            BootPatchReport::warning(
                "could not update installed boot files",
                vec![
                    format!("Partition: {}", partition.display()),
                    format!("Details: {error:#}"),
                ],
            )
        });

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
            report
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
        return Ok(BootPatchReport::skipped(
            "installed boot files were not recognized",
            vec![format!("Mounted {}", partition.display())],
        ));
    };

    let plan = patch_plan(style);
    let mut changed = false;
    let mut details = vec![format!("Mounted {}", partition.display())];

    if original == plan.content {
        details.push(format!("Verified {}", uenv_path.display()));
    } else {
        fs::write(&uenv_path, plan.content)
            .with_context(|| format!("failed to write {}", uenv_path.display()))?;
        details.push(format!("Updated {}", uenv_path.display()));
        changed = true;
    }

    if style == BootPatchStyle::ArmbianOrTiDebian {
        let extlinux_report = patch_armbian_extlinux(boot_dir, target)?;
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
    ArmbianOrTiDebian,
    TiYocto,
}

struct PatchPlan {
    content: &'static str,
    summary: &'static str,
    detail: &'static str,
}

fn patch_plan(style: BootPatchStyle) -> PatchPlan {
    match style {
        BootPatchStyle::ArmbianOrTiDebian => PatchPlan {
            content: ARMBIAN_EMMC_UENV,
            summary: "updated boot files for eMMC boot on Armbian/TI Debian images",
            detail: "Set bootpart=0:1, rootfs lookup=mmc 0:2, mmcdev=0, and added an extlinux eMMC fallback.",
        },
        BootPatchStyle::TiYocto => PatchPlan {
            content: TI_YOCTO_EMMC_UENV,
            summary: "updated uEnv.txt for eMMC boot on TI Yocto images",
            detail: "Set mmcdev=0 and rootfs bootpart=0:2.",
        },
    }
}

struct FilePatchReport {
    changed: bool,
    details: Vec<String>,
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
