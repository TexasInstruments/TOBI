SUMMARY = "TOBI-lite low-memory installer initramfs"
DESCRIPTION = "AM62-SIP focused TOBI initramfs with HDMI and serial UI while trimming unused kernel modules."
LICENSE = "GPL-2.0-only"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/GPL-2.0-only;md5=801f80980d171dd6425610833a22dbe6"

COMPATIBLE_MACHINE = "am62xxsip-evm"

inherit core-image

IMAGE_FEATURES = ""
IMAGE_LINGUAS = ""
IMAGE_FSTYPES = "cpio.xz"

PACKAGE_INSTALL = "\
    base-files \
    base-passwd \
    busybox \
    ca-certificates \
    iproute2 \
    kmod \
    kernel-modules \
    mmc-utils \
    tobi-initramfs-init \
    tobi-prebuilt \
    xz \
    zstd \
"

IMAGE_ROOTFS_SIZE = "32768"

TOBI_LITE_KEEP_MODULES = "\
    am65-cpts \
    davinci-mdio \
    cqhci \
    cdns-dphy \
    cdns-mhdp8546 \
    dp83867 \
    dp83869 \
    display-connector \
    drm \
    drm-client-lib \
    drm-client-modeset \
    drm-display-helper \
    drm-dma-helper \
    drm-kms-helper \
    dwc3 \
    dwc3-am62 \
    ext4 \
    fat \
    fbcon \
    fixed-phy \
    gpio-backlight \
    gpio-regulator \
    hid-generic \
    it66121 \
    jbd2 \
    mbcache \
    mdio-bitbang \
    micrel \
    mmc-block \
    motorcomm \
    nls-cp437 \
    nls-iso8859-1 \
    of-mdio \
    panel-simple \
    panel-simple-dsi \
    phy-cadence-torrent \
    phy-gmii-sel \
    pwm-tiecap \
    pwm-tiehrpwm \
    scsi-mod \
    sd-mod \
    sdhci \
    sdhci-am654 \
    sdhci-pltfm \
    realtek \
    simple-bridge \
    sii902x \
    ti-am65-cpsw-nuss \
    tidss \
    uas \
    usb-storage \
    usbhid \
    vfat \
    xhci-hcd \
    xhci-plat-hcd \
    zram \
    zsmalloc \
"

tobi_lite_prune_kernel_modules() {
    [ -d "${IMAGE_ROOTFS}/lib/modules" ] || return 0

    TOBI_LITE_KEEP_MODULES="${TOBI_LITE_KEEP_MODULES}" python3 - "${IMAGE_ROOTFS}" <<'PY'
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
keep_names = {name.replace("-", "_") for name in os.environ["TOBI_LITE_KEEP_MODULES"].split()}

def module_name(path):
    name = path.name
    for suffix in (".ko.xz", ".ko.zst", ".ko.gz", ".ko"):
        if name.endswith(suffix):
            name = name[:-len(suffix)]
            break
    return name.replace("-", "_")

for module_dir in (root / "lib/modules").glob("*"):
    if not module_dir.is_dir():
        continue

    modules = {}
    deps = {}
    dep_file = module_dir / "modules.dep"
    if dep_file.exists():
        for line in dep_file.read_text(errors="ignore").splitlines():
            if ":" not in line:
                continue
            module, dep_line = line.split(":", 1)
            module_path = module_dir / module
            modules[module_name(module_path)] = module
            deps[module] = dep_line.split()

    wanted = set()
    stack = []
    for name in keep_names:
        module = modules.get(name)
        if module:
            wanted.add(module)
            stack.append(module)

    while stack:
        module = stack.pop()
        for dep in deps.get(module, []):
            if dep not in wanted:
                wanted.add(dep)
                stack.append(dep)

    for module_path in module_dir.rglob("*.ko*"):
        rel = module_path.relative_to(module_dir).as_posix()
        if rel not in wanted and module_name(module_path) not in keep_names:
            module_path.unlink()
PY

    for module_dir in "${IMAGE_ROOTFS}"/lib/modules/*; do
        [ -d "$module_dir" ] || continue
        version="${module_dir##*/}"
        if command -v depmodwrapper >/dev/null 2>&1; then
            depmodwrapper -b "${IMAGE_ROOTFS}" "$version" >/dev/null 2>&1 || true
        elif command -v depmod >/dev/null 2>&1; then
            depmod -b "${IMAGE_ROOTFS}" "$version" >/dev/null 2>&1 || true
        fi
    done
}

ROOTFS_POSTPROCESS_COMMAND += "tobi_lite_prune_kernel_modules; "

export IMAGE_BASENAME = "tobi-lite-initramfs"
