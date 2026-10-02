# TOBI

**TI Out of Box Installer**: terminal OS installer for Texas Instruments Sitara starter kit evaluation modules.

The catalog includes SK-AM62P-LP, SK-AM62-LP, SK-AM62-SIP, SK-AM62B, BeaglePlay, SK-AM62A-LP, TMDS62LEVM, SK-AM64B, TMDS64EVM, SK-AM68, and SK-AM69 entries. All listed boards except SK-AM62A-LP have Armbian Community images from their matching Armbian board downloads. BeaglePlay also has official BeagleBoard Debian IoT and XFCE images.

## Catalog

Board definitions and downloadable OS images are listed in the repository-root `catalog.json` (`../catalog.json` from this directory).

TOBI uses the public GitHub-hosted catalog by default:

```text
https://raw.githubusercontent.com/TexasInstruments/TOBI/master/catalog.json
```

Use `--manifest` to test a local or alternate catalog:

```sh
cargo run -- --manifest ../catalog.json --mode mock
```

Mock mode defaults to SK-AM62P-LP. To preview another board's filtered OS list:

```sh
TOBI_MOCK_BOARD=sk-am64b cargo run -- --mode mock
```

The app intentionally does not embed a downloadable-image catalog. If the online catalog cannot be reached, TOBI shows a warning and continues with the local custom-image option only.

## Run Locally

No arguments match the production appliance behavior: live mode with write permissions enabled.

```sh
cargo run --
```

For local UI testing, use mock mode:

```sh
cargo run -- --mode mock
```

`mock` mode never writes to a real block device. Use `--no-allow-write` if you need live device detection without permitting writes.

## Build On Linux

Ubuntu 22.04 host example:

```sh
sudo apt-get update
sudo apt-get install -y build-essential ca-certificates curl git pkg-config libssl-dev liblzma-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
cargo test
cargo build --release
```

## Custom Images

TOBI includes a built-in **Custom image from attached media** option. It scans mounted media for:

```text
.wic.xz, .img.xz, .wic.zst, .img.zst, .wic.gz, .img.gz, .wic, .img, .raw, .bin
```

Default scan roots:

```text
macOS: /Volumes
Linux: /run/media, /media, /mnt, /var/run/media
```

For local testing or unusual mount layouts:

```sh
TOBI_CUSTOM_IMAGE_ROOTS="/path/to/images:/another/root" cargo run -- --mode mock
```

If the online catalog cannot be reached, TOBI stays open, warns the user, and still allows flashing a custom local image. Press `P` from the warning to set the UTC system time, enter a proxy URL, and retry the catalog. The time prompt matters because HTTPS catalog and image downloads can fail when the board clock is wrong.

TOBI streams images directly to the target media. The full downloaded or local image does not need to fit into RAM; only the installer runtime, decompressor, and write buffers do. Before installing, TOBI checks the available RAM against an estimated working set and blocks the install if that working set cannot fit.

## Run In Docker

From this directory, use the repository root as the build context:

```sh
docker build -f Dockerfile -t tobi ..
docker run --rm -it tobi
```

## Live Write Mode

Live mode is the default production mode. The TUI still requires explicit target selection and confirmation before writing:

```sh
sudo tobi \
  --manifest https://raw.githubusercontent.com/TexasInstruments/TOBI/master/catalog.json \
  --proxy http://proxy.example.com:8080 \
  --target /dev/mmcblk0
```

The production Yocto image should run fully from initramfs before this mode is used.

After an eMMC image write, TOBI runs a post-flash boot patcher before showing the success screen. The patcher mounts the installed boot partition, updates `uEnv.txt` when the image is recognized as TI Yocto, TI Debian, or Armbian media, then unmounts it. This fixes SD-card-oriented defaults by selecting the eMMC MMC index and rootfs partition. For a TOBI image, it retains the `/recovery` boot paths and TOBI arguments so the installed image starts the RAM installer again. The install UI shows the patching phase and the final success popup lists exactly what was changed. A boot-preparation warning stops the install with an error, even when the disk image was written; correct the reported problem and keep the SD card for recovery.

The default eMMC layout is a disk image in the user data area, with bootloaders in its filesystem boot partition. After success, power off, remove the SD card, select **MMCSD boot from eMMC port 0 in filesystem mode** on boards whose boot pins can select it, then power on. ROM's separate **eMMC boot** mode requires bootloaders in Boot0/Boot1. The install result cannot change physical switches; use the board manual and [TI's eMMC UDA guide](https://software-dl.ti.com/processor-sdk-linux/esd/AM62X/latest/exports/docs/linux/How_to_Guides/Target/How_to_mmcsd_boot_emmc_uda.html).

BeaglePlay's fixed released-USR boot straps select Boot0. When installing a TOBI recovery image, TOBI copies its `tiboot3.bin` to Boot0, verifies the copy by reading it back, and configures the eMMC to start that loader. Only this first stage uses Boot0; its patched SPL loads the remaining bootloader stages from the user-area filesystem. After success, power off, remove the SD card, and power on with USR released. Third-party images must provide compatible filesystem bootloaders. Keep the SD recovery image available while verifying eMMC startup without the SD card.

Successful eMMC installs do not start the ten-second automatic reboot countdown. The completion screen provides the power-off and boot-mode instructions above; Enter remains available for a manual reboot.

Mock tests, file-backed writes, and successful image builds verify software behavior. They do not verify ROM boot, eMMC controller configuration, cold boot, or recovery operation on each physical board.

## License

TOBI is licensed under GPL v2 only (`GPL-2.0-only`). See [LICENSE](LICENSE).
