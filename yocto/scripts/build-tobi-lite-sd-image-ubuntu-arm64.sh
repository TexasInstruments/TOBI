#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

MACHINE="${MACHINE:-am62xxsip-evm}" BITBAKE_TARGET=tobi-lite-sd-image \
  "$REPO_ROOT/yocto/scripts/build-tobi-sd-image-ubuntu-arm64.sh"
