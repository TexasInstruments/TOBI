SUMMARY = "TOBI-lite bootable SD-card image"
DESCRIPTION = "Flashable AM62-SIP SD-card image that boots the low-memory TOBI-lite initramfs into RAM."
LICENSE = "GPL-2.0-only"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/GPL-2.0-only;md5=801f80980d171dd6425610833a22dbe6"

COMPATIBLE_MACHINE = "am62xxsip-evm"

inherit core-image tobi-recovery

IMAGE_FEATURES = ""
IMAGE_LINGUAS = ""

IMAGE_FSTYPES = "wic.xz wic.bmap"
WKS_FILE = "sdimage-2part.wks"
WKS_FILES = "${WKS_FILE}"

PACKAGE_INSTALL = "\
    base-files \
    base-passwd \
    busybox \
"

IMAGE_ROOTFS_SIZE = "65536"

TOBI_RECOVERY_IMAGE = "tobi-lite-initramfs"

IMAGE_BOOT_FILES += "\
    tobi-uEnv.txt;uEnv.txt \
"

do_image_wic[depends] += "\
    tobi-bootfiles:do_deploy \
"

export IMAGE_BASENAME = "tobi-lite-sd-image"
