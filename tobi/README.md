# TOBI

**TI Out of Box Installer**: terminal OS installer prototype for Texas Instruments Sitara starter kit evaluation modules.

The first target board was **SK-AM62P-LP** using TI's Yocto machine name `am62pxx-evm`.
The catalog now includes SK-AM62P-LP, SK-AM62-LP, SK-AM62-SIP, SK-AM62B, BeaglePlay, SK-AM62A-LP, TMDS62LEVM, SK-AM64B, TMDS64EVM, SK-AM68, and SK-AM69 entries. Supported boards that also have Armbian board pages include Armbian Community images from the Armbian board downloads; SK-AM62A-LP and TMDS64EVM currently remain TI-only because Armbian does not publish matching board pages.

## Catalog

TOBI uses the public GitHub-hosted catalog by default:

```text
https://raw.githubusercontent.com/Grippy98/TOBI/master/tobi/sample/catalog.json
```

Use `--manifest` to test a local or alternate catalog:

```sh
cargo run -- --manifest sample/catalog.json --mode mock
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

## TOBI-lite Mode

`--lite` labels the app as **TOBI-lite** and enables the low-memory xz test policy used by the AM62-SIP Yocto image:

```sh
cargo run -- --mode mock --lite
```

In lite mode, `.wic.xz` images use measured catalog decoder-memory values when available, otherwise they fall back to the gzip-sized RAM estimate, and the xz guard is not enforced. This exists so 256 MiB AM62-SIP hardware can prove whether current TI `.wic.xz` images actually stream successfully. The production low-memory answer may still be `.wic.gz` or low-window `.wic.zst` catalog entries if xz is unstable.

## Run In Docker

```sh
docker build -t tobi .
docker run --rm -it tobi
```

## Live Write Mode

Live mode is the default production mode. The TUI still requires explicit target selection and confirmation before writing:

```sh
sudo tobi \
  --manifest https://raw.githubusercontent.com/Grippy98/TOBI/master/tobi/sample/catalog.json \
  --proxy http://proxy.example.com:8080 \
  --target /dev/mmcblk0
```

The production Yocto image should run fully from initramfs before this mode is used.

After a successful eMMC flash, TOBI runs a post-flash boot patcher before showing the success screen. The patcher mounts the installed boot partition, updates `uEnv.txt` when the image is recognized as TI Yocto, TI Debian, or Armbian media, then unmounts it before reboot. This fixes SD-card-oriented defaults by selecting the eMMC MMC index and rootfs partition. The install UI shows the patching phase and the final success popup lists exactly what was changed or warns if patching could not be completed.

## License

TOBI is licensed under GPL v2 only (`GPL-2.0-only`). See [LICENSE](LICENSE).
