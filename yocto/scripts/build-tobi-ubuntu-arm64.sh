#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="${IMAGE:-tobi-ubuntu-arm64-builder:22.04}"
OUT_DIR="${OUT_DIR:-$REPO_ROOT/out/aarch64-linux}"
CACHE_DIR="${CACHE_DIR:-$REPO_ROOT/out/.cache/ubuntu-arm64-tobi}"

docker build \
  -f "$REPO_ROOT/docker/ubuntu-arm64-tobi-builder/Dockerfile" \
  -t "$IMAGE" \
  "$REPO_ROOT"

mkdir -p "$OUT_DIR" "$CACHE_DIR/cargo" "$CACHE_DIR/target"

docker run --rm \
  --platform linux/arm64 \
  --user "$(id -u):$(id -g)" \
  -e CARGO_HOME=/cargo \
  -e CARGO_TARGET_DIR=/target \
  -v "$REPO_ROOT:/workspace:ro" \
  -v "$CACHE_DIR/cargo:/cargo" \
  -v "$CACHE_DIR/target:/target" \
  -v "$OUT_DIR:/out" \
  -w /workspace/tobi \
  "$IMAGE" \
  sh -lc 'cargo test && cargo build --release && cp /target/release/tobi /out/tobi'

file "$OUT_DIR/tobi"
