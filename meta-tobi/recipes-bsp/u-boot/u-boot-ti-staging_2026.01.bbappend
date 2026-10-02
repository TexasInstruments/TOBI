FILESEXTRAPATHS:prepend := "${THISDIR}/${PN}:"

# Start with BeaglePlay while the menu and media mappings are hardware-tested.
SRC_URI:append:beagleplay-ti = " \
    file://0001-board-beagleplay-add-TOBI-recovery-boot-menu.patch \
    file://0002-board-beagleplay-enable-IT66121-HDMI-boot-menu.patch \
    file://recolor-ti-logo.py \
"

inherit python3native

do_deploy:append:beagleplay-ti() {
    ${PYTHON} ${UNPACKDIR}/recolor-ti-logo.py \
        ${S}/tools/logos/ti_logo_414x97_32bpp.bmp \
        ${DEPLOYDIR}/ti_logo_414x97_32bpp.bmp.gz
}
