# Building TOBI Board Images

TI documents Processor SDK Linux Yocto builds using `oe-layersetup`. For the current Sitara SDK `12.00.00.07.04` board set, the non-Chromium layer config is:

```text
configs/processor-sdk/processor-sdk-master-12.00.00.07.04-config.txt
```

Use Ubuntu 22.04 or TI's Yocto container for repeatable builds. On Apple silicon, the included ARM64 Ubuntu Docker flow builds natively without Rosetta. TI notes that full SDK image builds can require very large disk space; `tobi-initramfs` should be much smaller, but the BSP checkout and shared state are still substantial.

## Supported Board Matrix

| Board | Yocto `MACHINE` | Notes |
| --- | --- | --- |
| SK-AM62P-LP | `am62pxx-evm` | AM62P starter kit |
| SK-AM62-LP | `am62xx-lp-evm` | AM62x low-power starter kit |
| SK-AM62-SIP | `am62xxsip-evm` | AM62x SIP starter kit |
| SK-AM62B | `am62xx-evm` | AM62x starter kit family |
| BeaglePlay | `beagleplay-ti` | BeagleBoard.org AM62x single-board computer |
| SK-AM62A-LP | `am62axx-evm` | AM62A Edge AI starter kit |
| TMDS62LEVM | `am62lxx-evm` | AM62L evaluation module |
| SK-AM64B | `am64xx-evm` | AM64x starter kit |
| TMDS64EVM | `am64xx-evm` | AM64x GP evaluation module; shares the AM64x Yocto machine |
| SK-AM68 | `am68-sk` | AM68 starter kit |
| SK-AM69 | `am69-sk` | AM69 starter kit |

## Layout

```text
tobi/       Rust application
meta-tobi/  Yocto layer
yocto/      helper scripts and notes
```

## Build Outline

```sh
git clone https://git.ti.com/git/arago-project/oe-layersetup.git tisdk
cd tisdk
./oe-layertool-setup.sh -f configs/processor-sdk/processor-sdk-master-12.00.00.07.04-config.txt
cd build
. conf/setenv
bitbake-layers add-layer /absolute/path/to/meta-tobi
```

The initramfs defaults to the public catalog hosted by the TOBI repository:

```text
https://raw.githubusercontent.com/TexasInstruments/TOBI/master/catalog.json
```

Override it with `TOBI_MANIFEST_URL` in the initramfs environment, or with the kernel argument:

```text
tobi.manifest=https://example.com/catalog.json
```

No downloadable-image catalog is embedded into the initramfs. If the hosted catalog cannot be reached at runtime, TOBI keeps running and offers only the local custom-image flow.

If the network is present but a proxy is required, the TUI recovery flow asks the user to set UTC system time first, then choose the TI proxy (`http://webproxy.ext.ti.com:80`) or enter a manual proxy URL before retrying the hosted catalog.

To force that proxy/time path for board testing, add this kernel argument:

```text
tobi.test_proxy_setup=1
```

Build or provide an AArch64 Linux `tobi` binary and point Yocto at it:

```sh
echo 'TOBI_PREBUILT = "/absolute/path/to/out/aarch64-linux/tobi"' >> conf/local.conf
MACHINE=am62pxx-evm bitbake tobi-initramfs
```

Set `MACHINE` to any entry from the board matrix to build that board's initramfs or SD-card image.

The matrix has eleven boards and ten machine builds because SK-AM64B and TMDS64EVM share `am64xx-evm`. Build outputs retain Yocto machine filenames; release `2026.10.2` uses `TOBI-2026.10.2-<BOARD>.img.xz` with the board spelling shown in the matrix. Publish both AM64x board labels from the shared machine output. The `v2026.10.2` release assets contain only those eleven compressed disk images and `SHA256SUMS`; initramfs, standalone binaries, and bmaps remain build outputs.

On x86_64 Linux hosts, the recommended Docker flow keeps BitBake native to the host and only cross-compiles the standalone `tobi` app to AArch64:

```sh
./yocto/scripts/build-tobi-ubuntu-x86_64.sh
```

This writes:

```text
out/aarch64-linux/tobi
```

Cargo dependencies and build outputs are cached under `out/.cache/ubuntu-x86_64-tobi-cross` so repeat builds are faster. Override `OUT_DIR`, `CACHE_DIR`, `IMAGE`, or `TARGET` if you want different paths, a different local builder tag, or another Rust target.

Use that file as `TOBI_PREBUILT` for Yocto integration.

The full initramfs can be built from an x86_64 Linux host with:

```sh
./yocto/scripts/build-tobi-initramfs-ubuntu-x86_64.sh
```

This writes copied artifacts to:

```text
out/yocto/tobi-initramfs-am62pxx-evm.rootfs.cpio.xz
```

To build a user-flashable two-part SD-card image:

```sh
./yocto/scripts/build-tobi-sd-image-ubuntu-x86_64.sh
```

To build one specific board:

```sh
MACHINE=am62axx-evm ./yocto/scripts/build-tobi-sd-image-ubuntu-x86_64.sh
```

To build every currently defined board image:

```sh
./yocto/scripts/build-tobi-all-sd-images-ubuntu-x86_64.sh
```

The x86_64 helper keeps the TI SDK checkout, downloads, sstate cache, and build home in Docker named volumes prefixed with `tobi-x86_64-yocto-`.

On Apple silicon, the Rust app can be built inside native ARM64 Ubuntu Docker without Rosetta:

```sh
./yocto/scripts/build-tobi-ubuntu-arm64.sh
```

This writes:

```text
out/aarch64-linux/tobi
```

Cargo dependencies and build outputs are cached under `out/.cache/ubuntu-arm64-tobi` so repeat builds are faster. Override `OUT_DIR`, `CACHE_DIR`, or `IMAGE` if you want different paths or a different local builder tag.

Use that file as `TOBI_PREBUILT` for the first Yocto integration test.

The full initramfs can also be built from Apple silicon inside native ARM64 Ubuntu Docker:

```sh
./yocto/scripts/build-tobi-initramfs-ubuntu-arm64.sh
```

This writes copied artifacts to:

```text
out/yocto/tobi-initramfs-am62pxx-evm.rootfs.cpio.xz
```

The helper keeps the TI SDK checkout, downloads, sstate cache, and build home in Docker named volumes. That avoids BitBake's `TMPDIR` case-sensitivity check on macOS/APFS and keeps repeat builds incremental.

To build a user-flashable two-part SD-card image:

```sh
./yocto/scripts/build-tobi-sd-image-ubuntu-arm64.sh
```

To build one specific board:

```sh
MACHINE=am62axx-evm ./yocto/scripts/build-tobi-sd-image-ubuntu-arm64.sh
```

To build every currently defined board image:

```sh
./yocto/scripts/build-tobi-all-sd-images-ubuntu-arm64.sh
```

This writes:

```text
out/yocto/tobi-sd-image-<machine>.rootfs.wic.xz
out/yocto/tobi-sd-image-<machine>.rootfs.wic.bmap
```

Inspect the compressed and uncompressed initramfs sizes with:

```sh
xz -l out/yocto/tobi-initramfs-am62pxx-evm.rootfs.cpio.xz
```

The expected output is a compressed initramfs under the TI deploy directory, usually:

```text
deploy-ti/images/am62pxx-evm/tobi-initramfs-am62pxx-evm.cpio.xz
```

## BeaglePlay Recovery Boot Test

`meta-tobi` carries a `u-boot-ti-staging_2026.01.bbappend` for `beagleplay-ti`. It enables the TOBI boot menu on both the UART and HDMI and builds TOBI SD images with their Linux kernel, initramfs, and DTB under the boot filesystem's `/recovery` directory. The U-Boot IT66121 path is video-only and keeps UART output active as a fallback.

```sh
MACHINE=beagleplay-ti ./yocto/scripts/build-tobi-sd-image-ubuntu-x86_64.sh
```

With the debug UART and an HDMI monitor connected, verify all of these cases:

1. Confirm the three-second splash and ten-second menu timeout, then let the SD entry boot.
2. Select eMMC and confirm its `uEnv.txt`, boot script, extlinux, or EFI flow boots without scanning SD as a fallback.
3. Select TOBI Recovery and confirm it loads `/recovery/Image`, `/recovery/uInitrd`, and `/recovery/dtb/ti/k3-am625-beagleplay.dtb`.
4. Repeat each entry with its media removed or a required file renamed and confirm the menu returns after the error.
5. Confirm the same menu is visible over HDMI, then repeat a boot with HDMI disconnected and confirm UART operation is unchanged.

For an additional WIC image, opt in from that image recipe or `.bbappend`:

```bitbake
inherit tobi-recovery
```

This only populates `IMAGE_BOOT_FILES`; ensure the WKS boot partition has room for the added kernel and initramfs. Do not enable it globally on secure production images until the recovery signing, rollback, and update policy is defined.

## eMMC Install And Hardware Validation

TOBI's default eMMC install writes the full disk image to the user data area. The post-install patch selects eMMC in supported OS boot configurations. For a TOBI recovery image, it keeps `/recovery/Image`, `/recovery/uInitrd`, the board DTBs, and the TOBI kernel arguments intact instead of replacing them with a normal OS boot configuration.

After a successful install on a DIP-switch board, power off, remove the SD card, choose **MMCSD boot, eMMC port 0, filesystem (FS) mode** using the board user guide and SoC TRM, then power on. TI distinguishes this from **eMMC boot**, which loads the ROM's first-stage loader from hardware Boot0/Boot1; see the [MMC boot-mode definitions](https://software-dl.ti.com/processor-sdk-linux/esd/AM62PX/latest/exports/docs/linux/Foundational_Components/U-Boot/UG-Memory-K3.html) and [UDA filesystem boot procedure](https://software-dl.ti.com/processor-sdk-linux/esd/AM62X/latest/exports/docs/linux/How_to_Guides/Target/How_to_mmcsd_boot_emmc_uda.html).

Use the manual for the exact board revision to translate BOOTMODE bits into numbered physical switches. The SK-AM62P-LP uses SW4/SW5; SK-AM64B uses SW2/SW3. Their switch banks and bit orders differ, so a binary setting copied from another board or an unlabeled forum image is insufficient. The [SK-AM62P-LP guide](https://www.ti.com/lit/pdf/spruja2) and [SK-AM64B guide](https://www.ti.com/lit/pdf/spruj64) provide the physical pin mapping and ON/OFF definitions.

BeaglePlay has fixed boot straps controlled by USR: pressed during power-on selects SD filesystem boot; released selects eMMC Boot0. TOBI therefore makes a board-specific exception when installing a TOBI recovery image: copy the installed FAT partition's `tiboot3.bin` to Boot0 at offset zero, verify the bytes by reading them back, and enable Boot0 for the ROM's first-stage load. The patched SPL then reads `tispl.bin` and `u-boot.img` from the user-area FAT filesystem. Other boards use filesystem boot from the user area. Third-party images still require compatible filesystem bootloaders; inclusion in the catalog does not establish hardware boot compatibility with this loader. See [BeaglePlay boot documentation](https://docs.beagleboard.org/boards/beagleplay/demos-and-tutorials/understanding-boot.html).

If eMMC preparation reports a warning, the installer stops with an error even after a completed image write. The successful eMMC completion screen does not start the ten-second automatic reboot countdown and instructs the user to power off and remove the SD card; BeaglePlay powers on with USR released, while DIP-switch boards use the filesystem boot setting above. Enter remains available for a manual reboot. Verify both the error path and these completion instructions during hardware testing.

Before marking a board/image combination as hardware verified, record its board revision, SoC security type, image checksum, boot-mode settings, and UART log. Test a cold boot from the installed eMMC, warm reboot, recovery startup, and startup with the SD removed where the board's ROM configuration permits it. Build completion and software tests alone do not establish those results. The BeaglePlay UART/HDMI recovery menu is board-specific; other machine outputs do not acquire that menu merely by including the recovery payload.

## License

TOBI is licensed under GPL v2 only (`GPL-2.0-only`).
