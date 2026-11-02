#!/usr/bin/env bash
# Reproducible cw-headstash wasm build (the recipe for v0.0.1 40197344… and tf-denom 08f9b1ca…).
# usage: tools/headstash/deploy/build-cw-headstash.sh [OUT_DIR]
# Mounts the parent "crates" dir because the workspace has path deps on ../cosmwasm, ../zakura-common, and so on.
# --ignore-rust-version: zakura-common declares rust-version 1.97, but the pinned image is rustc 1.91.
set -euo pipefail
IMAGE=${IMAGE:-terpnetwork/optimizer-arm64:0.17.0-rust1.91}
REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
CRATES=$(dirname "$REPO"); NAME=$(basename "$REPO")
OUT=$(mkdir -p "${1:-$REPO/artifacts/build}" && cd "${1:-$REPO/artifacts/build}" && pwd)
docker run --rm -v "$CRATES":/code -v "$OUT":/out -v "hs-wasm-target-$NAME":/target \
  -w "/code/$NAME" --entrypoint sh "$IMAGE" -c '
set -e
CARGO_TARGET_DIR=/target cargo build --release --lib --target wasm32-unknown-unknown \
  -p cw-headstash --features zk-api --ignore-rust-version
wasm-opt -Os --enable-bulk-memory --enable-sign-ext --disable-simd \
  --one-caller-inline-max-function-size=1000 \
  /target/wasm32-unknown-unknown/release/cw_headstash.wasm -o /out/cw_headstash.wasm'
# Image RUSTFLAGS: -C link-arg=-s -C link-arg=--allow-undefined -C target-feature=-simd128 --cfg getrandom_backend="unsupported"
shasum -a 256 "$OUT/cw_headstash.wasm"; wc -c < "$OUT/cw_headstash.wasm"
