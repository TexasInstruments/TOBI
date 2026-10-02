# TOBI Hardware Image Workspace

**TOBI** is the **TI Out of Box Installer** for Texas Instruments Sitara starter kit evaluation modules. It boots a small RAM-resident Linux environment, presents a terminal UI, downloads a selected OS image, streams/decompresses it directly to target media, and reboots into the installed image.

The first hardware target was **SK-AM62P-LP** / Yocto machine `am62pxx-evm`.
TOBI now carries board definitions and catalog entries for these starter kits and EVMs:

| Board | Yocto `MACHINE` | Catalog SDK |
| --- | --- | --- |
| SK-AM62P-LP | `am62pxx-evm` | `PROCESSOR-SDK-LINUX-AM62P` |
| SK-AM62-LP | `am62xx-lp-evm` | `PROCESSOR-SDK-LINUX-AM62X` |
| SK-AM62-SIP | `am62xxsip-evm` | `PROCESSOR-SDK-LINUX-AM62X` |
| SK-AM62B | `am62xx-evm` | `PROCESSOR-SDK-LINUX-AM62X` |
| BeaglePlay | `beagleplay-ti` | `PROCESSOR-SDK-LINUX-AM62X` |
| SK-AM62A-LP | `am62axx-evm` | `PROCESSOR-SDK-LINUX-AM62A` |
| TMDS62LEVM | `am62lxx-evm` | `AM62L-LINUX-SDK` |
| SK-AM64B | `am64xx-evm` | `PROCESSOR-SDK-LINUX-AM64X` |
| TMDS64EVM | `am64xx-evm` | `PROCESSOR-SDK-LINUX-AM64X` |
| SK-AM68 | `am68-sk` | `PROCESSOR-SDK-LINUX-AM68` |
| SK-AM69 | `am69-sk` | `PROCESSOR-SDK-LINUX-AM69` |

The catalog also includes Armbian community downloads for supported boards that have matching Armbian board pages as a separate Community section.

## Layout

```text
tobi/       standalone Rust TUI application; can become its own git repo
meta-tobi/  Yocto layer for packaging TOBI into a RAM installer image
yocto/      build notes and helper scripts for TI Processor SDK Linux
```

The Rust app can be split out later with:

```sh
git subtree split --prefix=tobi -b tobi-app
```

## Hosted Catalog

The public OS catalog is hosted from this GitHub repository:

```text
https://raw.githubusercontent.com/Grippy98/TOBI/master/tobi/sample/catalog.json
```

TOBI uses that URL by default. For local testing or private catalogs, pass another source:

```sh
cargo run --manifest-path tobi/Cargo.toml -- --manifest /path/to/catalog.json --mode mock
cargo run --manifest-path tobi/Cargo.toml -- --manifest https://example.com/catalog.json --mode mock
```

The Yocto initramfs also uses the hosted catalog by default. It can be overridden at boot with:

```text
tobi.manifest=https://example.com/catalog.json
```

or by setting `TOBI_MANIFEST_URL` in the initramfs environment.

The production image does not embed a downloadable-image catalog. If the board cannot reach the hosted catalog, TOBI falls back to local-image flashing only and asks the user to attach FAT32 media with a compatible image file.

When a proxy is needed, TOBI prompts for UTC system time first so TLS validation can succeed even if automatic time sync failed. It then lets the user choose the TI proxy (`http://webproxy.ext.ti.com:80`) or enter a manual proxy URL.

## License

TOBI is licensed under GPL v2 only (`GPL-2.0-only`). See [LICENSE](LICENSE).

## Build The TUI On Linux

Ubuntu 22.04 is the recommended Linux host baseline for this project.

```sh
sudo apt-get update
sudo apt-get install -y build-essential ca-certificates curl git pkg-config libssl-dev liblzma-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
cd tobi
cargo test
cargo build --release
cargo run -- --mode mock
```

Running TOBI with no arguments starts the production path: `--mode live` with write permissions enabled. Use `--mode mock` for local UI testing; mock mode never writes to a real block device. Use `--no-allow-write` to inspect live device detection without allowing writes.

## Build With Docker

The TUI app can be built and run in Docker:

```sh
cd tobi
docker build -t tobi .
docker run --rm -it tobi
```

For quick local UI testing from this workspace, use the helper script. It runs
the app in mock mode, attaches it to your host terminal, and uses the same
x86_64 Docker builder/cache used by the Yocto helper scripts:

```sh
./scripts/run-tobi-local.sh tui
./scripts/run-tobi-local.sh serial
./scripts/run-tobi-local.sh tui --test-proxy-setup
```

The `tui` mode exercises the HDMI-style crossterm UI. The `serial` mode
exercises the line-oriented UART UI that the initramfs starts with
`--serial-ui`. The `--test-proxy-setup` flag simulates a board with DHCP and
a valid local IP where the online catalog is unreachable, then opens the UTC
time and proxy setup screen.

On an x86_64 Linux host, build natively and let Yocto cross-compile the ARM image:

```sh
./yocto/scripts/build-tobi-ubuntu-x86_64.sh
./yocto/scripts/build-tobi-sd-image-ubuntu-x86_64.sh
```

Build a specific board image by setting `MACHINE`:

```sh
MACHINE=am64xx-evm ./yocto/scripts/build-tobi-sd-image-ubuntu-x86_64.sh
```

Build all currently defined board images:

```sh
./yocto/scripts/build-tobi-all-sd-images-ubuntu-x86_64.sh
```

The x86_64 flow cross-compiles the standalone `tobi` binary to AArch64, then runs BitBake as a native x86_64 process. This is the preferred Docker flow for x86_64 hosts.

On Apple silicon, both the Rust app and the TI Yocto image can be built in native ARM64 Ubuntu Docker without Rosetta:

```sh
./yocto/scripts/build-tobi-ubuntu-arm64.sh
./yocto/scripts/build-tobi-sd-image-ubuntu-arm64.sh
```

Build a specific board image by setting `MACHINE`:

```sh
MACHINE=am64xx-evm ./yocto/scripts/build-tobi-sd-image-ubuntu-arm64.sh
```

Build all currently defined board images:

```sh
./yocto/scripts/build-tobi-all-sd-images-ubuntu-arm64.sh
```

The Yocto helper uses Docker named volumes for the TI SDK checkout, downloads, sstate cache, and build home so BitBake runs on a Linux filesystem instead of macOS' default case-insensitive filesystem.

## Build The Yocto Image On Linux

Build or provide an AArch64 Linux `tobi` binary first. On an ARM64 Linux host:

```sh
cd tobi
cargo build --release
mkdir -p ../out/aarch64-linux
cp target/release/tobi ../out/aarch64-linux/tobi
```

Then set up TI Processor SDK Linux and add `meta-tobi`:

```sh
git clone https://git.ti.com/git/arago-project/oe-layersetup.git tisdk
cd tisdk
./oe-layertool-setup.sh -f configs/processor-sdk/processor-sdk-master-12.00.00.07.04-config.txt
cd build
. conf/setenv
bitbake-layers add-layer /absolute/path/to/meta-tobi
echo 'TOBI_PREBUILT = "/absolute/path/to/out/aarch64-linux/tobi"' >> conf/local.conf
MACHINE=am62pxx-evm bitbake tobi-sd-image
```

The expected deploy artifacts are:

```text
tobi-initramfs-am62pxx-evm.rootfs.cpio.xz
tobi-sd-image-am62pxx-evm.rootfs.wic.xz
tobi-sd-image-am62pxx-evm.rootfs.wic.bmap
```

Replace `am62pxx-evm` with another supported machine to generate that board's TOBI image.

The SD image is user-flashable. It boots TOBI into RAM and leaves the target eMMC free to be overwritten by the installer.

When flashing to eMMC, TOBI runs a post-flash boot patcher before reboot. It mounts the installed boot partition, updates `uEnv.txt` for recognized TI Yocto, TI Debian, and Armbian layouts so U-Boot selects the eMMC MMC index and rootfs partition, and adds an `extlinux/extlinux.conf` eMMC bootflow fallback for Armbian-style images whose built-in U-Boot environment starts on SD. The TUI shows this as an explicit install phase, and the success popup includes the patch result and changed boot settings.

## BeaglePlay U-Boot Menu And Recovery Bundle

The `dev` branch patches TI U-Boot 2026.01 for `MACHINE=beagleplay-ti` with a
centered TI-red splash for three seconds followed by a ten-second menu on both
the debug UART and HDMI:

1. Boot an OS from the SD card (`mmc1`, default).
2. Boot an OS from eMMC (`mmc0`).
3. Start TOBI Recovery, searching SD and then eMMC.

The regular OS entries support TI's legacy `uEnv.txt` path followed by standard U-Boot bootflow discovery for `boot.scr`, extlinux, and EFI. A missing device, filesystem, boot file, or recovery component reports the error and returns to the menu instead of dropping out of the boot flow.

TOBI SD images place the kernel, initramfs, and board DTB under the boot partition's `/recovery` directory. That directory normally appears as `/boot/recovery` after Linux mounts the boot partition. The root `uEnv.txt` remains compatible with an unmodified TI U-Boot and points it at the recovery payload.

To add the same payload to another WIC image from a layer that depends on `meta-tobi`, inherit the opt-in class in that image or its `.bbappend`:

```bitbake
inherit tobi-recovery
```

The class adds `recovery/Image`, `recovery/uInitrd`, and the machine DTBs to `IMAGE_BOOT_FILES`. Check the target WKS boot-partition size before enabling it. It is intentionally not injected into every TI image by default yet: TOBI is a write-capable recovery environment, adds meaningful image size, and needs a defined signing and update policy for secure production systems.

Build the initial BeaglePlay test image with:

```sh
MACHINE=beagleplay-ti ./yocto/scripts/build-tobi-sd-image-ubuntu-x86_64.sh
```

The layer carries a video-only IT66121 bridge port and extends TI's TIDSS driver to activate the AM625 DPI pipeline. U-Boot reads the monitor EDID and falls back to 1280x720 at 60 Hz when EDID is unavailable. It emits DVI-compatible TMDS video over the HDMI connector; HDMI audio, HDCP, and runtime hot-plug handling are out of scope. Output remains multiplexed to the UART, so a missing or unsupported display does not remove serial access. Linux uses its normal DRM/TIDSS and IT66121 drivers after boot.

Directly chain-loading a second K3 `u-boot.img` is deliberately not part of this first version. On AM62x, ROM, `tiboot3.bin`, `tispl.bin`, TF-A/OP-TEE, and A53 U-Boot form a staged handoff, and a second U-Boot can depend on state supplied by the earlier stages. The supported path here is to let TOBI U-Boot boot the selected distro's normal OS configuration. Keeping a fully separate stock TI U-Boot should instead use a board-supported alternate boot source or bootloader slot and reboot into that chain.

## Optional TOBI-lite AM62-SIP Test Image

`SK-AM62-SIP` / `am62xxsip-evm` has 512 MiB of integrated LPDDR4. The normal
all-board build and release matrix therefore use the regular TOBI image for
this board. TI's AM6254ATL U-Boot and Linux device trees both describe the full
512 MiB region at `0x80000000`.

The branch retains **TOBI-lite** as an optional constrained-memory diagnostic
target. It trims the kernel module set, enables RAM-only zram swap, and relaxes
the `.wic.xz` memory guard for stress testing. It is not required for normal
AM62-SIP operation and is not selected by the all-board build scripts.

Build the TOBI-lite SD image with:

```sh
./yocto/scripts/build-tobi-lite-sd-image-ubuntu-x86_64.sh
```

On Apple silicon:

```sh
./yocto/scripts/build-tobi-lite-sd-image-ubuntu-arm64.sh
```

Expected copied artifacts use the `tobi-lite-*` basename, for example:

```text
out/yocto/tobi-lite-initramfs-am62xxsip-evm.rootfs.cpio.xz
out/yocto/tobi-lite-sd-image-am62xxsip-evm.rootfs.wic.xz
out/yocto/tobi-lite-sd-image-am62xxsip-evm.rootfs.wic.bmap
```
