FILESEXTRAPATHS:prepend := "${THISDIR}/${PN}:"

SRC_URI:append:am62xxsip-evm = " file://tobi-lite-zram.cfg"

KERNEL_CONFIG_FRAGMENTS:append:am62xxsip-evm = " ${UNPACKDIR}/tobi-lite-zram.cfg"
