use anyhow::bail;

use crate::device::{InstallTarget, TargetKind};
use crate::installer::RunMode;

#[cfg(any(target_os = "linux", test))]
use anyhow::Context;
#[cfg(test)]
use anyhow::anyhow;
#[cfg(any(target_os = "linux", test))]
use std::fs;
#[cfg(any(target_os = "linux", test))]
use std::path::{Path, PathBuf};

/// Identify the physical media that booted this RAM-only TOBI session.
///
/// Card identity takes precedence over the legacy root PARTUUID. Linux device
/// numbers and the order of the installer target list are never boot identities.
pub fn current_boot_target(
    mode: RunMode,
    devices: &[InstallTarget],
) -> anyhow::Result<InstallTarget> {
    match mode {
        RunMode::Mock => {
            let targets = devices
                .iter()
                .filter(|target| target.kind == TargetKind::Sd)
                .collect::<Vec<_>>();
            let [target] = targets.as_slice() else {
                bail!("mock updates require exactly one mock SD target");
            };
            if target.id != "mock-sd" {
                bail!("mock updates require the explicit mock SD target");
            }
            Ok((*target).clone())
        }
        RunMode::Live => {
            #[cfg(target_os = "linux")]
            {
                live_boot_target(devices)
            }
            #[cfg(not(target_os = "linux"))]
            {
                bail!("automatic TOBI updates are only supported in the Linux RAM environment")
            }
        }
    }
}

/// Capture card identity so a replacement at the same device path cannot inherit
/// an update confirmation, including when legacy images share a PARTUUID.
pub fn boot_target_identity(mode: RunMode, target: &InstallTarget) -> anyhow::Result<String> {
    match mode {
        RunMode::Mock => {
            if target.kind != TargetKind::Sd || target.id != "mock-sd" {
                bail!("mock updates require the explicit mock SD target");
            }
            Ok(format!("mock:mock-sd:{}", target.path.display()))
        }
        RunMode::Live => {
            #[cfg(target_os = "linux")]
            {
                physical_target_identity(Path::new("/sys/class/block"), target)
            }
            #[cfg(not(target_os = "linux"))]
            {
                bail!("automatic TOBI updates are only supported in the Linux RAM environment")
            }
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn physical_target_identity(sys_block: &Path, target: &InstallTarget) -> anyhow::Result<String> {
    let disk = inspect_disk(sys_block, target)?;
    let device = disk.members[0];
    Ok(format!(
        "mmc:{}:{}:{}",
        disk.cid, device.major, device.minor
    ))
}

#[cfg(target_os = "linux")]
fn live_boot_target(devices: &[InstallTarget]) -> anyhow::Result<InstallTarget> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let cmdline = fs::read_to_string("/proc/cmdline").context("cannot read the boot identity")?;
    let mountinfo =
        fs::read_to_string("/proc/self/mountinfo").context("cannot verify RAM-only runtime")?;
    let sys_block = Path::new("/sys/class/block");
    // Inspect all physical cards, even when --target restricted the UI list.
    // Otherwise a second card with a cloned PARTUUID could be hidden from us.
    let inventory = crate::device::list_devices(crate::device::DeviceMode::Live, None)
        .context("cannot enumerate physical boot media")?;
    let selected = resolve_boot_target(
        &cmdline,
        &mountinfo,
        sys_block,
        &inventory,
        probe_partition_uuid,
    )?;

    let selected_name = whole_mmc_disk_name(&selected.path)?;
    let supplied_matches = devices
        .iter()
        .filter(|target| target.kind == selected.kind)
        .filter(|target| whole_mmc_disk_name(&target.path).is_ok_and(|name| name == selected_name))
        .count();
    if supplied_matches != 1 {
        bail!("the uniquely identified boot media is not an unambiguous available update target");
    }

    let metadata = fs::metadata(&selected.path)
        .with_context(|| format!("cannot inspect {}", selected.path.display()))?;
    if !metadata.file_type().is_block_device() {
        bail!("automatic updates require a whole physical block device");
    }
    let device = DeviceNumber {
        major: libc::major(metadata.rdev()),
        minor: libc::minor(metadata.rdev()),
    };
    let sys_device = read_device_number(&sys_block.join(&selected_name).join("dev"))?;
    if device != sys_device {
        bail!("the boot media device changed while preparing its update");
    }
    ensure_swap_disabled(
        &fs::read_to_string("/proc/swaps").context("cannot verify a swap-free RAM runtime")?,
    )?;
    Ok(selected)
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Debug, Eq, PartialEq)]
enum BootIdentity {
    Cid(String),
    PartUuid(String),
}

#[cfg(any(target_os = "linux", test))]
fn boot_identity(cmdline: &str) -> anyhow::Result<BootIdentity> {
    let markers = cmdline
        .split_ascii_whitespace()
        .filter(|argument| *argument == "tobi.boot_cid" || argument.starts_with("tobi.boot_cid="))
        .collect::<Vec<_>>();
    if !markers.is_empty() {
        let [marker] = markers.as_slice() else {
            bail!("the boot command line contains more than one TOBI card identity");
        };
        let value = marker
            .strip_prefix("tobi.boot_cid=")
            .context("the TOBI card identity is missing its value")?;
        return normalize_boot_cid(value).map(BootIdentity::Cid);
    }

    let roots = cmdline
        .split_ascii_whitespace()
        .filter_map(|argument| argument.strip_prefix("root="))
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        bail!("no unique card identity or legacy root PARTUUID identifies the boot media");
    };
    let uuid = root
        .strip_prefix("PARTUUID=")
        .context("legacy boot media can only be identified by root=PARTUUID")?;
    Ok(BootIdentity::PartUuid(normalize_partition_uuid(uuid)?))
}

#[cfg(any(target_os = "linux", test))]
fn normalize_boot_cid(value: &str) -> anyhow::Result<String> {
    let words = value.split(':').collect::<Vec<_>>();
    if words.len() != 4 {
        bail!("the TOBI card identity must contain four hexadecimal words");
    }
    let mut cid = String::with_capacity(32);
    for word in words {
        if word.is_empty() || word.len() > 8 || !word.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("the TOBI card identity contains an invalid hexadecimal word");
        }
        let word = u32::from_str_radix(word, 16).context("invalid TOBI card identity word")?;
        cid.push_str(&format!("{word:08x}"));
    }
    Ok(cid)
}

#[cfg(any(target_os = "linux", test))]
fn normalize_card_cid(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("a physical MMC device exposes an invalid card identity");
    }
    Ok(value.to_ascii_lowercase())
}

#[cfg(any(target_os = "linux", test))]
fn normalize_partition_uuid(value: &str) -> anyhow::Result<String> {
    let groups = value.split('-').collect::<Vec<_>>();
    let lengths = groups.iter().map(|group| group.len()).collect::<Vec<_>>();
    if (lengths != [8, 2] && lengths != [8, 4, 4, 4, 12])
        || !groups
            .iter()
            .all(|group| group.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        bail!("the legacy root PARTUUID is malformed or contains a partition offset");
    }
    Ok(value.to_ascii_lowercase())
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeviceNumber {
    major: u32,
    minor: u32,
}

#[cfg(any(target_os = "linux", test))]
impl DeviceNumber {
    fn parse(value: &str) -> anyhow::Result<Self> {
        let (major, minor) = value
            .trim()
            .split_once(':')
            .context("block device identity is missing major/minor numbers")?;
        if major.is_empty()
            || minor.is_empty()
            || !major.bytes().all(|byte| byte.is_ascii_digit())
            || !minor.bytes().all(|byte| byte.is_ascii_digit())
        {
            bail!("block device identity contains invalid major/minor numbers");
        }
        Ok(Self {
            major: major.parse().context("invalid block device major number")?,
            minor: minor.parse().context("invalid block device minor number")?,
        })
    }
}

#[cfg(any(target_os = "linux", test))]
fn read_device_number(path: &Path) -> anyhow::Result<DeviceNumber> {
    DeviceNumber::parse(
        &fs::read_to_string(path).with_context(|| {
            format!("cannot read block device identity from {}", path.display())
        })?,
    )
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug)]
struct Mount {
    device: DeviceNumber,
    mountpoint: String,
    filesystem: String,
}

#[cfg(any(target_os = "linux", test))]
fn parse_mounts(mountinfo: &str) -> anyhow::Result<Vec<Mount>> {
    mountinfo
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
            let separator = fields
                .iter()
                .position(|field| *field == "-")
                .context("mount information is missing its filesystem separator")?;
            if separator < 6 || fields.len() < separator + 4 {
                bail!("mount information is incomplete");
            }
            Ok(Mount {
                device: DeviceNumber::parse(fields[2])?,
                mountpoint: fields[4].to_string(),
                filesystem: fields[separator + 1].to_string(),
            })
        })
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn ensure_ram_root(mounts: &[Mount]) -> anyhow::Result<()> {
    let roots = mounts
        .iter()
        .filter(|mount| mount.mountpoint == "/")
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        bail!("automatic updates require one verifiable RAM root filesystem");
    };
    if root.device.major != 0 || !matches!(root.filesystem.as_str(), "rootfs" | "ramfs" | "tmpfs") {
        bail!("automatic updates require TOBI to run from its RAM-only initramfs");
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
struct DiskSnapshot<'a> {
    target: &'a InstallTarget,
    cid: String,
    members: Vec<DeviceNumber>,
    partitions: Vec<PathBuf>,
    holder_dirs: Vec<PathBuf>,
}

#[cfg(any(target_os = "linux", test))]
fn whole_mmc_disk_name(path: &Path) -> anyhow::Result<String> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("the boot target has no valid kernel disk name")?;
    let suffix = name
        .strip_prefix("mmcblk")
        .context("automatic updates require physical SD or eMMC media")?;
    if path.parent() != Some(Path::new("/dev"))
        || suffix.is_empty()
        || !suffix.bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("automatic updates require a whole physical MMC disk");
    }
    Ok(name.to_string())
}

#[cfg(any(target_os = "linux", test))]
fn inspect_disk<'a>(
    sys_block: &Path,
    target: &'a InstallTarget,
) -> anyhow::Result<DiskSnapshot<'a>> {
    let name = whole_mmc_disk_name(&target.path)?;
    let sys_disk = sys_block.join(&name);
    if sys_disk.join("partition").exists() {
        bail!("the update target is a partition rather than a whole MMC disk");
    }
    let card_type = fs::read_to_string(sys_disk.join("device/type")).with_context(|| {
        format!(
            "cannot verify physical card type for {}",
            target.path.display()
        )
    })?;
    let type_matches = match target.kind {
        TargetKind::Sd => card_type.trim() == "SD",
        TargetKind::Emmc => card_type.trim() == "MMC",
        _ => false,
    };
    if !type_matches {
        bail!("the physical card type disagrees with the update target");
    }
    let cid = normalize_card_cid(
        &fs::read_to_string(sys_disk.join("device/cid"))
            .with_context(|| format!("cannot read card identity for {}", target.path.display()))?,
    )?;
    let mut members = vec![read_device_number(&sys_disk.join("dev"))?];
    let mut partitions = Vec::new();
    let mut holder_dirs = vec![sys_disk.join("holders")];
    for entry in fs::read_dir(&sys_disk)
        .with_context(|| format!("cannot enumerate partitions for {}", target.path.display()))?
    {
        let entry = entry?;
        if !entry.path().join("partition").exists() {
            continue;
        }
        let partition_name = entry.file_name();
        let partition_name = partition_name
            .to_str()
            .context("a boot media partition has an invalid kernel name")?;
        let suffix = partition_name
            .strip_prefix(&format!("{name}p"))
            .context("a boot media partition does not belong to its physical disk")?;
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            bail!("a boot media partition has an invalid kernel name");
        }
        members.push(read_device_number(&entry.path().join("dev"))?);
        partitions.push(Path::new("/dev").join(partition_name));
        holder_dirs.push(entry.path().join("holders"));
    }
    Ok(DiskSnapshot {
        target,
        cid,
        members,
        partitions,
        holder_dirs,
    })
}

#[cfg(any(target_os = "linux", test))]
fn resolve_boot_target(
    cmdline: &str,
    mountinfo: &str,
    sys_block: &Path,
    inventory: &[InstallTarget],
    mut partition_uuid: impl FnMut(&Path) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<InstallTarget> {
    let mounts = parse_mounts(mountinfo)?;
    ensure_ram_root(&mounts)?;
    let identity = boot_identity(cmdline)?;
    let disks = inventory
        .iter()
        .filter(|target| matches!(target.kind, TargetKind::Sd | TargetKind::Emmc))
        .map(|target| inspect_disk(sys_block, target))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut matching = Vec::new();
    for (index, disk) in disks.iter().enumerate() {
        match &identity {
            BootIdentity::Cid(cid) if &disk.cid == cid => matching.push(index),
            BootIdentity::PartUuid(expected) => {
                for partition in &disk.partitions {
                    if let Some(actual) = partition_uuid(partition)?
                        && normalize_partition_uuid(&actual)? == *expected
                    {
                        matching.push(index);
                    }
                }
            }
            _ => {}
        }
    }
    let [index] = matching.as_slice() else {
        bail!("the boot identity does not uniquely identify one physical SD or eMMC device");
    };
    let selected = &disks[*index];
    if mounts
        .iter()
        .any(|mount| selected.members.contains(&mount.device))
    {
        bail!(
            "the boot media or one of its partitions is mounted; skip the update and reboot into the RAM-only TOBI environment"
        );
    }
    for holders in &selected.holder_dirs {
        if fs::read_dir(holders)
            .with_context(|| format!("cannot check block device holders at {}", holders.display()))?
            .next()
            .transpose()?
            .is_some()
        {
            bail!("the boot media is in use by another block device");
        }
    }
    Ok(selected.target.clone())
}

#[cfg(target_os = "linux")]
fn probe_partition_uuid(partition: &Path) -> anyhow::Result<Option<String>> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let metadata = fs::metadata(partition)
        .with_context(|| format!("cannot inspect partition {}", partition.display()))?;
    if !metadata.file_type().is_block_device() {
        bail!("legacy boot identity probing requires physical block partitions");
    }
    let name = partition
        .file_name()
        .context("the boot partition has no kernel name")?;
    let expected = read_device_number(&Path::new("/sys/class/block").join(name).join("dev"))?;
    if expected.major != libc::major(metadata.rdev())
        || expected.minor != libc::minor(metadata.rdev())
    {
        bail!("a physical partition changed while probing the legacy boot identity");
    }
    // Low-level probing avoids the blkid cache. Its partition UUID tag is
    // PART_ENTRY_UUID, whereas normal cached output labels that tag PARTUUID.
    let output = std::process::Command::new("blkid")
        .args(["-p", "-s", "PART_ENTRY_UUID", "-o", "value"])
        .arg(partition)
        .output()
        .with_context(|| format!("cannot probe partition identity on {}", partition.display()))?;
    if !output.status.success() {
        if output.status.code() == Some(2) && output.stderr.is_empty() {
            return Ok(None);
        }
        bail!(
            "cannot safely probe the partition UUID on {}",
            partition.display()
        );
    }
    let value = String::from_utf8(output.stdout).context("partition UUID output is not UTF-8")?;
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalize_partition_uuid(value)?))
    }
}

#[cfg(any(target_os = "linux", test))]
fn ensure_swap_disabled(swaps: &str) -> anyhow::Result<()> {
    let mut lines = swaps.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().context("swap information is missing")?;
    if !header.starts_with("Filename") {
        bail!("swap information is malformed");
    }
    if lines.next().is_some() {
        bail!("automatic updates require swapping to be disabled in the RAM-only TOBI environment");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const RAM_MOUNTS: &str =
        "1 0 0:1 / / rw - rootfs rootfs rw\n2 1 0:2 / /proc rw - proc proc rw\n";
    const CID_A: &str = "00000001000000020000000300000004";
    const CID_B: &str = "00000005000000060000000700000008";

    fn target(name: &str, kind: TargetKind) -> InstallTarget {
        InstallTarget {
            id: name.to_string(),
            name: name.to_string(),
            path: Path::new("/dev").join(name),
            size_bytes: Some(16_000_000_000),
            kind,
            removable: kind == TargetKind::Sd,
            partitions: Vec::new(),
            warning: None,
        }
    }

    struct Fixture {
        temp: TempDir,
        disks: Vec<InstallTarget>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                temp: TempDir::new().expect("temporary sysfs"),
                disks: Vec::new(),
            }
        }

        fn add(&mut self, name: &str, kind: TargetKind, cid: &str, minor: u32) {
            let root = self.temp.path().join(name);
            fs::create_dir_all(root.join("device")).unwrap();
            fs::create_dir_all(root.join("holders")).unwrap();
            fs::write(root.join("dev"), format!("179:{minor}\n")).unwrap();
            fs::write(root.join("device/cid"), format!("{cid}\n")).unwrap();
            fs::write(
                root.join("device/type"),
                if kind == TargetKind::Sd {
                    "SD\n"
                } else {
                    "MMC\n"
                },
            )
            .unwrap();
            for partition in 1..=2 {
                let part = root.join(format!("{name}p{partition}"));
                fs::create_dir_all(part.join("holders")).unwrap();
                fs::write(part.join("partition"), format!("{partition}\n")).unwrap();
                fs::write(part.join("dev"), format!("179:{}\n", minor + partition)).unwrap();
            }
            self.disks.push(target(name, kind));
        }

        fn resolve(&self, cmdline: &str, mounts: &str) -> anyhow::Result<InstallTarget> {
            resolve_boot_target(cmdline, mounts, self.temp.path(), &self.disks, |_| {
                bail!("CID selection must not inspect a legacy UUID")
            })
        }
    }

    #[test]
    fn normalizes_four_cid_words_without_changing_word_order() {
        assert_eq!(normalize_boot_cid("1:2:3:4").unwrap(), CID_A);
        assert_eq!(
            normalize_boot_cid("ABCDEF01:00000002:aB:FFFFFFFF").unwrap(),
            "abcdef0100000002000000abffffffff"
        );
        assert_eq!(
            normalize_card_cid(&format!("{}\n", CID_A.to_uppercase())).unwrap(),
            CID_A
        );
    }

    #[test]
    fn rejects_malformed_duplicate_and_missing_card_markers_without_fallback() {
        for marker in [
            "tobi.boot_cid",
            "tobi.boot_cid=",
            "tobi.boot_cid=1:2:3",
            "tobi.boot_cid=1:2:3:4:5",
            "tobi.boot_cid=1:2::4",
            "tobi.boot_cid=100000000:2:3:4",
            "tobi.boot_cid=0x1:2:3:4",
            "tobi.boot_cid=1:2:3:g",
            "tobi.boot_cid=1:2:3:4 tobi.boot_cid=1:2:3:4",
        ] {
            assert!(
                boot_identity(&format!("{marker} root=PARTUUID=12345678-02")).is_err(),
                "{marker}"
            );
        }
        assert!(boot_identity("console=ttyS0 root=/dev/mmcblk1p2").is_err());
        assert!(boot_identity("console=ttyS0").is_err());
    }

    #[test]
    fn cid_selects_the_card_when_sd_and_emmc_linux_names_are_swapped() {
        let mut first = Fixture::new();
        first.add("mmcblk0", TargetKind::Emmc, CID_B, 0);
        first.add("mmcblk1", TargetKind::Sd, CID_A, 8);
        assert_eq!(
            first
                .resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS)
                .unwrap()
                .path,
            Path::new("/dev/mmcblk1")
        );

        let mut swapped = Fixture::new();
        swapped.add("mmcblk0", TargetKind::Sd, CID_A, 0);
        swapped.add("mmcblk1", TargetKind::Emmc, CID_B, 8);
        assert_eq!(
            swapped
                .resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS)
                .unwrap()
                .path,
            Path::new("/dev/mmcblk0")
        );
    }

    #[test]
    fn cid_wins_over_a_legacy_root_uuid_and_does_not_probe_blkid() {
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_A, 0);
        assert!(
            fixture
                .resolve("tobi.boot_cid=1:2:3:4 root=PARTUUID=bad", RAM_MOUNTS)
                .is_ok()
        );
    }

    #[test]
    fn duplicated_or_missing_card_identity_is_not_an_update_target() {
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_A, 0);
        fixture.add("mmcblk1", TargetKind::Sd, CID_A, 8);
        assert!(
            fixture
                .resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS)
                .is_err()
        );
        assert!(
            fixture
                .resolve("tobi.boot_cid=5:6:7:8", RAM_MOUNTS)
                .is_err()
        );
        fs::write(fixture.temp.path().join("mmcblk1/device/cid"), "bad CID\n").unwrap();
        assert!(
            fixture
                .resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS)
                .is_err()
        );
        fs::remove_file(fixture.temp.path().join("mmcblk1/device/cid")).unwrap();
        assert!(
            fixture
                .resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS)
                .is_err()
        );
    }

    #[test]
    fn file_partition_boot_partition_and_wrong_card_type_are_rejected() {
        for name in [
            "mmcblk0p1",
            "mmcblk0boot0",
            "mmcblk0rpmb",
            "sda",
            "image.img",
        ] {
            assert!(
                whole_mmc_disk_name(&Path::new("/dev").join(name)).is_err(),
                "{name}"
            );
        }
        assert!(whole_mmc_disk_name(Path::new("/tmp/mmcblk0")).is_err());
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_A, 0);
        fs::write(fixture.temp.path().join("mmcblk0/device/type"), "SD\n").unwrap();
        assert!(
            fixture
                .resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS)
                .is_err()
        );
    }

    #[test]
    fn mounted_disk_partition_and_bind_mount_prevent_overwrite() {
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_A, 0);
        for mounted in [
            "3 1 179:0 / /media rw - vfat /dev/mmcblk0 rw\n",
            "3 1 179:1 / /boot rw - vfat /dev/mmcblk0p1 rw\n",
            "3 1 179:2 /subdirectory /bound rw - ext4 /dev/mmcblk0p2 rw\n",
        ] {
            assert!(
                fixture
                    .resolve("tobi.boot_cid=1:2:3:4", &format!("{RAM_MOUNTS}{mounted}"))
                    .is_err()
            );
        }
        let other_mount = "3 1 179:9 / /other rw - vfat /dev/mmcblk1p1 rw\n";
        assert!(
            fixture
                .resolve(
                    "tobi.boot_cid=1:2:3:4",
                    &format!("{RAM_MOUNTS}{other_mount}")
                )
                .is_ok()
        );
    }

    #[test]
    fn disk_backed_overlay_missing_or_ambiguous_root_is_not_ram_only() {
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_A, 0);
        for mounts in [
            "1 0 179:2 / / rw - ext4 /dev/mmcblk0p2 rw\n",
            "1 0 0:42 / / rw - overlay overlay rw\n",
            "1 0 179:2 / / rw - rootfs rootfs rw\n",
            "1 0 0:1 / /proc rw - proc proc rw\n",
            "1 0 0:1 / / rw - rootfs rootfs rw\n2 0 0:2 / / rw - tmpfs tmpfs rw\n",
            "incomplete mount information",
        ] {
            assert!(
                fixture.resolve("tobi.boot_cid=1:2:3:4", mounts).is_err(),
                "{mounts}"
            );
        }
        for filesystem in ["rootfs", "ramfs", "tmpfs"] {
            assert!(
                fixture
                    .resolve(
                        "tobi.boot_cid=1:2:3:4",
                        &format!("1 0 0:1 / / rw - {filesystem} {filesystem} rw\n")
                    )
                    .is_ok()
            );
        }
    }

    #[test]
    fn block_device_holders_prevent_indirect_overwrite() {
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_A, 0);
        let holder = fixture.temp.path().join("mmcblk0/mmcblk0p2/holders/dm-0");
        fs::create_dir(&holder).unwrap();
        assert!(
            fixture
                .resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS)
                .is_err()
        );
        fs::remove_dir(holder).unwrap();
        assert!(fixture.resolve("tobi.boot_cid=1:2:3:4", RAM_MOUNTS).is_ok());
    }

    #[test]
    fn legacy_uuid_resolves_only_one_exact_partition_across_all_cards() {
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_B, 0);
        fixture.add("mmcblk1", TargetKind::Sd, CID_A, 8);
        let mut inspected = Vec::new();
        let selected = resolve_boot_target(
            "root=PARTUUID=ABCDEF12-02",
            RAM_MOUNTS,
            fixture.temp.path(),
            &fixture.disks,
            |partition| {
                inspected.push(partition.to_path_buf());
                Ok((partition == Path::new("/dev/mmcblk1p2")).then(|| "abcdef12-02".to_string()))
            },
        )
        .unwrap();
        assert_eq!(selected.path, Path::new("/dev/mmcblk1"));
        assert_eq!(
            inspected.len(),
            4,
            "every physical card partition must be probed"
        );
        let cloned = resolve_boot_target(
            "root=PARTUUID=abcdef12-02",
            RAM_MOUNTS,
            fixture.temp.path(),
            &fixture.disks,
            |partition| {
                Ok(partition
                    .to_string_lossy()
                    .ends_with("p2")
                    .then(|| "abcdef12-02".to_string()))
            },
        );
        assert!(cloned.is_err(), "cloned UUIDs must not pick the first card");
    }

    #[test]
    fn legacy_uuid_rejects_offsets_filesystem_uuids_duplicate_roots_and_probe_failures() {
        for cmdline in [
            "root=PARTUUID=abcdef12-02/PARTNROFF=1",
            "root=PARTUUID=abcdef12",
            "root=UUID=12345678-1234-1234-1234-123456789abc",
            "root=PARTUUID=abcdef12-02 root=PARTUUID=abcdef12-02",
        ] {
            assert!(boot_identity(cmdline).is_err(), "{cmdline}");
        }
        assert_eq!(
            boot_identity("root=PARTUUID=ABCDEF12-1234-5678-90AB-123456789ABC").unwrap(),
            BootIdentity::PartUuid("abcdef12-1234-5678-90ab-123456789abc".to_string())
        );
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Emmc, CID_A, 0);
        assert!(
            resolve_boot_target(
                "root=PARTUUID=abcdef12-02",
                RAM_MOUNTS,
                fixture.temp.path(),
                &fixture.disks,
                |_| Err(anyhow!("probe failed")),
            )
            .is_err()
        );
    }

    #[test]
    fn mock_resolution_is_explicit_unique_and_never_probes_real_storage() {
        let mut mock = target("mmcblk1", TargetKind::Sd);
        mock.id = "mock-sd".to_string();
        assert_eq!(
            current_boot_target(RunMode::Mock, &[mock.clone()])
                .unwrap()
                .id,
            "mock-sd"
        );
        assert!(current_boot_target(RunMode::Mock, &[]).is_err());
        assert!(current_boot_target(RunMode::Mock, &[target("mmcblk1", TargetKind::Sd)]).is_err());
        assert!(current_boot_target(RunMode::Mock, &[mock.clone(), mock]).is_err());
        assert!(
            current_boot_target(RunMode::Mock, &[target("mmcblk0", TargetKind::Emmc)]).is_err()
        );
    }

    #[test]
    fn cloned_replacement_changes_identity_at_the_same_path_and_device_number() {
        let mut fixture = Fixture::new();
        fixture.add("mmcblk0", TargetKind::Sd, CID_A, 0);
        let target = &fixture.disks[0];
        let before = physical_target_identity(fixture.temp.path(), target).unwrap();
        assert_eq!(before, format!("mmc:{CID_A}:179:0"));
        fs::write(fixture.temp.path().join("mmcblk0/device/cid"), CID_B).unwrap();
        let after = physical_target_identity(fixture.temp.path(), target).unwrap();
        assert_eq!(after, format!("mmc:{CID_B}:179:0"));
        assert_ne!(
            before, after,
            "a cloned replacement must invalidate confirmation"
        );
        assert_eq!(target.path, Path::new("/dev/mmcblk0"));
    }

    #[test]
    fn mock_identity_is_deterministic_without_accessing_its_path() {
        let mut mock = target("mmcblk1", TargetKind::Sd);
        mock.id = "mock-sd".to_string();
        mock.path = PathBuf::from("/does-not-exist/mock-sd");
        assert_eq!(
            boot_target_identity(RunMode::Mock, &mock).unwrap(),
            "mock:mock-sd:/does-not-exist/mock-sd"
        );
        assert_eq!(
            boot_target_identity(RunMode::Mock, &mock).unwrap(),
            boot_target_identity(RunMode::Mock, &mock).unwrap()
        );
        mock.kind = TargetKind::File;
        assert!(boot_target_identity(RunMode::Mock, &mock).is_err());
    }

    #[test]
    fn swap_must_be_disabled_for_a_ram_only_update() {
        assert!(ensure_swap_disabled("Filename Type Size Used Priority\n").is_ok());
        assert!(
            ensure_swap_disabled(
                "Filename Type Size Used Priority\n/dev/mmcblk0p2 partition 100 0 -2\n"
            )
            .is_err()
        );
        assert!(ensure_swap_disabled("").is_err());
        assert!(ensure_swap_disabled("unrecognized format\n").is_err());
    }
}
