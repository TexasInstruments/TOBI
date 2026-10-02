#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="${IMAGE:-tobi-ubuntu-arm64-yocto-builder:22.04}"
OUT_DIR="${OUT_DIR:-$REPO_ROOT/out/aarch64-linux}"
TISDK_VOLUME="${TISDK_VOLUME:-tobi-yocto-tisdk}"
DOWNLOADS_VOLUME="${DOWNLOADS_VOLUME:-tobi-yocto-downloads}"
SSTATE_VOLUME="${SSTATE_VOLUME:-tobi-yocto-sstate}"
HOME_VOLUME="${HOME_VOLUME:-tobi-yocto-home}"
MACHINE="${MACHINE:-am62pxx-evm}"
BITBAKE_TARGET="${BITBAKE_TARGET:-tobi-sd-image}"
SDK_CONFIG="${SDK_CONFIG:-processor-sdk-master-12.00.00.07.04-config.txt}"

if [[ ! -x "$OUT_DIR/tobi" ]]; then
  echo "Missing $OUT_DIR/tobi; run yocto/scripts/build-tobi-ubuntu-arm64.sh first." >&2
  exit 1
fi

if [[ "${SKIP_DOCKER_BUILD:-0}" != "1" ]]; then
  docker build \
    --platform linux/arm64 \
    -f "$REPO_ROOT/docker/ubuntu-arm64-yocto-builder/Dockerfile" \
    -t "$IMAGE" \
    "$REPO_ROOT"
fi

docker volume create "$TISDK_VOLUME" >/dev/null
docker volume create "$DOWNLOADS_VOLUME" >/dev/null
docker volume create "$SSTATE_VOLUME" >/dev/null
docker volume create "$HOME_VOLUME" >/dev/null

if [[ "${SKIP_VOLUME_CHOWN:-0}" != "1" ]]; then
  docker run --rm \
    --platform linux/arm64 \
    -e HOST_UID="$(id -u)" \
    -e HOST_GID="$(id -g)" \
    -v "$TISDK_VOLUME:/yocto/tisdk" \
    -v "$DOWNLOADS_VOLUME:/yocto/downloads" \
    -v "$SSTATE_VOLUME:/yocto/sstate-cache" \
    -v "$HOME_VOLUME:/yocto/home" \
    "$IMAGE" \
    bash -lc '
      set -euo pipefail
      chown -R "$HOST_UID:$HOST_GID" /yocto/tisdk /yocto/downloads /yocto/sstate-cache /yocto/home
    '
fi

docker run --rm \
  --platform linux/arm64 \
  --user "$(id -u):$(id -g)" \
  -e HOME=/yocto/home \
  -e MACHINE="$MACHINE" \
  -e BITBAKE_TARGET="$BITBAKE_TARGET" \
  -e SDK_CONFIG="$SDK_CONFIG" \
  -e TISDK_DIR=/yocto/tisdk \
  -e DL_DIR=/yocto/downloads \
  -e SSTATE_DIR=/yocto/sstate-cache \
  -e TOBI_PREBUILT=/workspace/out/aarch64-linux/tobi \
  -e FORCE_TOBI_PREBUILT_REBUILD="${FORCE_TOBI_PREBUILT_REBUILD:-1}" \
  -v "$REPO_ROOT:/workspace" \
  -v "$TISDK_VOLUME:/yocto/tisdk" \
  -v "$DOWNLOADS_VOLUME:/yocto/downloads" \
  -v "$SSTATE_VOLUME:/yocto/sstate-cache" \
  -v "$HOME_VOLUME:/yocto/home" \
  -w /workspace \
  "$IMAGE" \
  bash -lc '
    set -euo pipefail
    mkdir -p "$HOME" /workspace/out/yocto
    git config --global user.email "tobi-builder@example.invalid"
    git config --global user.name "TOBI Builder"
    ./yocto/scripts/bootstrap-tobi-yocto.sh "$BITBAKE_TARGET"
    find "$TISDK_DIR/build" -path "*deploy*images*" \
      \( -name "tobi-sd-image*.wic.xz" -o -name "tobi-sd-image*.wic.bmap" \
         -o -name "tobi-lite-sd-image*.wic.xz" -o -name "tobi-lite-sd-image*.wic.bmap" \
         -o -name "tobi-initramfs*.cpio.xz" -o -name "tobi-lite-initramfs*.cpio.xz" \) \
      -exec cp -f {} /workspace/out/yocto/ \;
  '
