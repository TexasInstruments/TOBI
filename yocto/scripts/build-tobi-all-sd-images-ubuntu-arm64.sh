#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MACHINES="${MACHINES:-am62pxx-evm am62xx-evm am62xx-lp-evm am62xxsip-evm beagleplay-ti am62axx-evm am62lxx-evm am64xx-evm am68-sk am69-sk}"

if [[ "${SKIP_TOBI_APP_BUILD:-0}" != "1" || ! -x "$REPO_ROOT/out/aarch64-linux/tobi" ]]; then
  "$REPO_ROOT/yocto/scripts/build-tobi-ubuntu-arm64.sh"
fi

first=1
for machine in $MACHINES; do
  echo "==> Building TOBI SD image for MACHINE=$machine"
  if [[ "$first" == "1" ]]; then
    MACHINE="$machine" BITBAKE_TARGET=tobi-sd-image \
      "$REPO_ROOT/yocto/scripts/build-tobi-sd-image-ubuntu-arm64.sh"
    first=0
  else
    SKIP_DOCKER_BUILD=1 SKIP_VOLUME_CHOWN=1 MACHINE="$machine" BITBAKE_TARGET=tobi-sd-image \
      "$REPO_ROOT/yocto/scripts/build-tobi-sd-image-ubuntu-arm64.sh"
  fi
done
