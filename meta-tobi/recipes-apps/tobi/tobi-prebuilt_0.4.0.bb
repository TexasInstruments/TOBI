SUMMARY = "TOBI - TI Out of Box Installer"
DESCRIPTION = "Terminal OS installer for Texas Instruments Sitara starter kit evaluation modules."
HOMEPAGE = "https://github.com/TexasInstruments/TOBI"
LICENSE = "GPL-2.0-only"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/GPL-2.0-only;md5=801f80980d171dd6425610833a22dbe6"

PV = "2026.10.2"

TOBI_PREBUILT ?= ""
S = "${UNPACKDIR}"

python () {
    if not d.getVar("TOBI_PREBUILT"):
        bb.fatal("TOBI_PREBUILT must point to a target-architecture tobi binary")
}

do_configure[noexec] = "1"
do_compile[noexec] = "1"

do_install() {
    install -d ${D}${bindir}
    install -m 0755 ${TOBI_PREBUILT} ${D}${bindir}/tobi
}

FILES:${PN} += "${bindir}/tobi"
RDEPENDS:${PN} += "liblzma"
INSANE_SKIP:${PN} += "already-stripped"
