#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TISDK_DIR="${TISDK_DIR:-$REPO_ROOT/tisdk}"
SDK_CONFIG="${SDK_CONFIG:-processor-sdk-master-12.00.00.07.04-config.txt}"
MACHINE="${MACHINE:-am62pxx-evm}"
BITBAKE_TARGET="${1:-${BITBAKE_TARGET:-tobi-initramfs}}"

if [[ -z "${TOBI_PREBUILT:-}" ]]; then
  echo "TOBI_PREBUILT must point to a target-architecture tobi binary" >&2
  exit 1
fi

if [[ ! -x "$TISDK_DIR/oe-layertool-setup.sh" ]]; then
  mkdir -p "$TISDK_DIR"
  find "$TISDK_DIR" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  git clone https://git.ti.com/git/arago-project/oe-layersetup.git "$TISDK_DIR"
fi

cd "$TISDK_DIR"
if [[ ! -d build ]]; then
  ./oe-layertool-setup.sh -f "configs/processor-sdk/$SDK_CONFIG"
fi

cd build
# shellcheck disable=SC1091
. conf/setenv

grep -q '^TOBI_PREBUILT' conf/local.conf || \
  echo "TOBI_PREBUILT = \"$TOBI_PREBUILT\"" >> conf/local.conf
if [[ -n "${DL_DIR:-}" ]]; then
  grep -q '^DL_DIR' conf/local.conf || \
    echo "DL_DIR = \"$DL_DIR\"" >> conf/local.conf
fi
if [[ -n "${SSTATE_DIR:-}" ]]; then
  grep -q '^SSTATE_DIR' conf/local.conf || \
    echo "SSTATE_DIR = \"$SSTATE_DIR\"" >> conf/local.conf
fi

# The layer checks TOBI_PREBUILT while parsing its recipes. Set it first.
bitbake-layers show-layers | grep -q "meta-tobi" || \
  bitbake-layers add-layer "$REPO_ROOT/meta-tobi"

if [[ "${FORCE_TOBI_PREBUILT_REBUILD:-0}" == "1" ]]; then
  MACHINE="$MACHINE" bitbake -c cleansstate tobi-prebuilt
  MACHINE="$MACHINE" bitbake -c clean tobi-initramfs tobi-sd-image
fi

MACHINE="$MACHINE" bitbake "$BITBAKE_TARGET"
