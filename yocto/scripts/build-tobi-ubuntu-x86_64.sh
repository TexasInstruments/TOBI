#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="${IMAGE:-tobi-ubuntu-x86_64-tobi-cross-builder:22.04}"
OUT_DIR="${OUT_DIR:-$REPO_ROOT/out/aarch64-linux}"
CACHE_DIR="${CACHE_DIR:-$REPO_ROOT/out/.cache/ubuntu-x86_64-tobi-cross}"
TARGET="${TARGET:-aarch64-unknown-linux-gnu}"

docker build \
  -f "$REPO_ROOT/docker/ubuntu-x86_64-tobi-cross-builder/Dockerfile" \
  -t "$IMAGE" \
  "$REPO_ROOT"

mkdir -p "$OUT_DIR" "$CACHE_DIR/cargo" "$CACHE_DIR/target"

docker run --rm \
  --user "$(id -u):$(id -g)" \
  -e CARGO_HOME=/cargo \
  -e CARGO_TARGET_DIR=/target \
  -e CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
  -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  -v "$REPO_ROOT:/workspace:ro" \
  -v "$CACHE_DIR/cargo:/cargo" \
  -v "$CACHE_DIR/target:/target" \
  -v "$OUT_DIR:/out" \
  -w /workspace/tobi \
  "$IMAGE" \
  sh -lc "cargo test && cargo build --release --target '$TARGET' && cp \"/target/$TARGET/release/tobi\" /out/tobi"

file "$OUT_DIR/tobi"
