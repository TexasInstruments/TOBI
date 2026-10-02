FILESEXTRAPATHS:prepend := "${THISDIR}/${PN}:"

SRC_URI:append = " file://0003-spl-tobi-boot-mmc-from-filesystem.patch"

# Start with BeaglePlay while the menu and media mappings are hardware-tested.
SRC_URI:append:beagleplay-ti = " \
    file://0001-board-beagleplay-add-TOBI-recovery-boot-menu.patch \
    file://0002-board-beagleplay-enable-IT66121-HDMI-boot-menu.patch \
    file://recolor-ti-logo.py \
"

inherit python3native

# Both K3 SPL stages must load TOBI's boot files from partition 1. Check the
# generated configurations so a BSP change cannot silently restore raw boot.
do_configure:append() {
    for config_file in $(find ${B} -name .config -type f); do
        if grep -q '^CONFIG_ARCH_K3=y$' "$config_file" && \
           grep -q '^CONFIG_SPL_MMC=y$' "$config_file"; then
            for option in SPL_TOBI_MMC_FS_BOOT SYS_MMCSD_FS_BOOT SPL_FS_FAT SPL_LIBDISK_SUPPORT; do
                grep -q "^CONFIG_$option=y$" "$config_file" || \
                    bbfatal "TOBI filesystem boot requires CONFIG_$option=y in $config_file"
            done
            grep -q '^CONFIG_SYS_MMCSD_FS_BOOT_PARTITION=1$' "$config_file" || \
                bbfatal "TOBI filesystem boot requires FAT partition 1 in $config_file"
        fi
    done
}

do_deploy:append:beagleplay-ti() {
    ${PYTHON} ${UNPACKDIR}/recolor-ti-logo.py \
        ${S}/tools/logos/ti_logo_414x97_32bpp.bmp \
        ${DEPLOYDIR}/ti_logo_414x97_32bpp.bmp.gz
}
