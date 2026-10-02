# Add a self-contained TOBI recovery payload to a WIC boot partition.
# The partition is normally mounted at /boot by Linux, so U-Boot's
# /recovery path appears as /boot/recovery in the running OS.

TOBI_RECOVERY_IMAGE ?= "tobi-initramfs"
TOBI_RECOVERY_INITRAMFS ?= "${TOBI_RECOVERY_IMAGE}-${MACHINE}.rootfs.cpio.xz"

TOBI_RECOVERY_DTB_FILES = ""
TOBI_RECOVERY_DTB_FILES:am62pxx-evm = "k3-am62p5-sk.dtb"
TOBI_RECOVERY_DTB_FILES:am62xx-evm = "k3-am625-sk.dtb"
TOBI_RECOVERY_DTB_FILES:am62xx-lp-evm = "k3-am62-lp-sk.dtb"
TOBI_RECOVERY_DTB_FILES:am62xxsip-evm = "k3-am6254atl-sk.dtb"
TOBI_RECOVERY_DTB_FILES:beagleplay-ti = "k3-am625-beagleplay.dtb"
TOBI_RECOVERY_DTB_FILES:am62axx-evm = "k3-am62a7-sk.dtb"
TOBI_RECOVERY_DTB_FILES:am62lxx-evm = "k3-am62l3-evm.dtb"
TOBI_RECOVERY_DTB_FILES:am64xx-evm = "k3-am642-sk.dtb k3-am642-evm.dtb"
TOBI_RECOVERY_DTB_FILES:am68-sk = "k3-am68-sk-base-board.dtb"
TOBI_RECOVERY_DTB_FILES:am69-sk = "k3-am69-sk.dtb"

IMAGE_BOOT_FILES += " \
    Image;recovery/Image \
    ${@' '.join(['%s;recovery/dtb/ti/%s' % (dtb, dtb) for dtb in d.getVar('TOBI_RECOVERY_DTB_FILES').split()])} \
    ${TOBI_RECOVERY_INITRAMFS};recovery/uInitrd \
"

do_image_wic[depends] += " \
    ${TOBI_RECOVERY_IMAGE}:do_image_complete \
    virtual/kernel:do_deploy \
"
