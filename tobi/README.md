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

## TOBI Updates

Before showing the OS list, TOBI compares its running version with the newest stable TOBI image for the detected board. The check runs after Welcome and any proxy configuration. **Install** downloads and writes the new full image to the verified current boot media; the prompt names the device and warns that all its data will be erased and a reboot is required. **Skip** continues to the OS list without another prompt that session.

Updates finish on the completion screen and require a manual reboot or power cycle. TOBI uses the boot card's hardware CID supplied by U-Boot, with a unique `root=PARTUUID` fallback for older boot environments. Automatic installation is unavailable when the source cannot be proven, the system is not RAM-resident, or the target is mounted. Mock mode simulates an update to its mock SD card without writing storage. Images need to be rebuilt to include this feature and the boot-source marker.

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

Live mode is the default production mode. Normal OS installations require explicit target selection and confirmation; installing a TOBI update instead writes the verified current boot media as described above:

```sh
sudo tobi \
  --manifest https://raw.githubusercontent.com/TexasInstruments/TOBI/master/catalog.json \
  --proxy http://proxy.example.com:8080 \
  --target /dev/mmcblk0
```

The production Yocto image should run fully from initramfs before this mode is used.

After an eMMC image write, TOBI runs a post-flash boot patcher before showing the success screen. The patcher mounts the installed boot partition, updates `uEnv.txt` when the image is recognized as TI Yocto, TI Debian, or Armbian media, then unmounts it. This fixes SD-card-oriented defaults by selecting the eMMC MMC index and rootfs partition. For a TOBI image, it retains the `/recovery` boot paths and TOBI arguments so the installed image starts the RAM installer again. The install UI shows the patching phase and the final success popup lists exactly what was changed. A boot-preparation warning stops the install with an error, even when the disk image was written; correct the reported problem and keep the SD card for recovery.

After a successful eMMC flash, the completion screen shows board-specific instructions from `../docs/boot-guide.json`. Press **G** for a full-screen QR code to the illustrated GitHub Pages guide; press G, Esc or Enter to return to instructions. Enter in the QR view does not reboot. Serial consoles print the settings and URL. The guide also identifies SK-AM64B and SK-AM68 as having no onboard eMMC. See the [Pages build and publishing instructions](../docs/README.md); a repository admin must enable Pages before the QR links are live.

The default eMMC layout is a disk image in the user data area, with bootloaders in its filesystem boot partition. AM62 starter kits, TMDS62LEVM and TMDS64EVM use **MMCSD boot from eMMC port 0 in filesystem mode**. Power off before changing switches or removing the SD card. ROM's separate **eMMC boot** mode reads Boot0/Boot1 and requires a different first-stage layout. Check the board revision, printed switch numbers and ON marking; see the shared guide's official references and [TI's eMMC UDA explanation](https://software-dl.ti.com/processor-sdk-linux/esd/AM62X/latest/exports/docs/linux/How_to_Guides/Target/How_to_mmcsd_boot_emmc_uda.html).

BeaglePlay's released-USR straps and SK-AM69's stock eMMC switch mode require Boot0. When installing a TOBI recovery image on either board, TOBI copies its `tiboot3.bin` to Boot0, verifies the copy by reading it back, and configures the eMMC to start that loader. The patched SPL then loads the remaining stages from the user-area filesystem. After success, power off and remove SD; leave BeaglePlay's USR released, or set SK-AM69 SW2.1 OFF, SW2.2/SW2.3 ON and leave SW2.4 unchanged. Third-party images are not automatically prepared with this recovery-only bootstrap. Keep the SD recovery image available while verifying startup.

Successful eMMC installs do not start the ten-second automatic reboot countdown. The completion screen provides the power-off and boot-mode instructions above; Enter remains available for a manual reboot.

Mock tests, file-backed writes, and successful image builds verify software behavior. They do not verify ROM boot, eMMC controller configuration, cold boot, or recovery operation on each physical board.

## License

TOBI is licensed under GPL v2 only (`GPL-2.0-only`). See [LICENSE](LICENSE).
