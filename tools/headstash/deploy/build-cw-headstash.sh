#!/usr/bin/env bash
# Hermetic, reproducible cw-headstash wasm build.
# usage: build-cw-headstash.sh [SRC_TREE] [OUT_DIR]
#   SRC_TREE  a checkout of this repo at the commit to build (default: this repo). It is mounted at the
#             canonical path $CRATES/terp-rs-consolidate, because source paths are embedded in the wasm.
#   SNAP      pinned sibling snapshot (default ~/hs-backup-20261026/build-snapshot-20261102, see its
#             MANIFEST.txt). Each sibling copy is mounted over $CRATES/<name>, so the live siblings
#             (cosmwasm, cw-minus, cw-plus-plus, cw-storage-plus, zakura-common) are never read.
# Recipe (the one that produced v0.0.1 40197344… and, after the tf-denom fold, dd40ded5…): optimizer-arm64:0.17.0-rust1.91 (its RUSTFLAGS),
#   host ~/.cargo registry and git caches, -j 4, a fresh target dir, --ignore-rust-version (zakura-common
#   declares 1.97), then wasm-opt -Os --one-caller-inline-max-function-size=1000 --enable-bulk-memory
#   --enable-sign-ext --disable-simd.
# Mounting at /code (or any other path) gives a different hash, so always use this script.
set -euo pipefail
IMAGE=${IMAGE:-terpnetwork/optimizer-arm64:0.17.0-rust1.91}
HERE=$(cd "$(dirname "$0")" && pwd)
SRC=$(cd "${1:-$(git -C "$HERE" rev-parse --show-toplevel)}" && pwd)
CRATES=${CRATES:-$HOME/abstract/terp-core/crates}
CANON=$CRATES/terp-rs-consolidate
SNAP=${SNAP:-$HOME/hs-backup-20261026/build-snapshot-20261102}
OUT=$(mkdir -p "${2:-$SRC/artifacts/build}" && cd "${2:-$SRC/artifacts/build}" && pwd)
TGT=$(mktemp -d /tmp/hs-wasm-target.XXXXXX)
[ -f "$SNAP/MANIFEST.txt" ] || { echo "missing $SNAP/MANIFEST.txt" >&2; exit 1; }
MOUNTS=(-v "$CRATES:$CRATES:ro")
while read -r name _; do MOUNTS+=(-v "$SNAP/$name:$CRATES/$name:ro"); done < "$SNAP/MANIFEST.txt"
MOUNTS+=(-v "$SRC:$CANON")
docker run --rm -m 6g "${MOUNTS[@]}" \
  -v "$HOME/.cargo/registry:/usr/local/cargo/registry" -v "$HOME/.cargo/git:/usr/local/cargo/git" \
  -v "$TGT:/target" -v "$OUT:/out" -w "$CANON" -e CARGO_TARGET_DIR=/target --entrypoint sh "$IMAGE" -c '
set -e
cargo build -j 4 --ignore-rust-version --release --lib --target wasm32-unknown-unknown -p cw-headstash --features zk-api
wasm-opt -Os --one-caller-inline-max-function-size=1000 --enable-bulk-memory --enable-sign-ext --disable-simd \
  /target/wasm32-unknown-unknown/release/cw_headstash.wasm -o /out/cw_headstash.wasm'
rm -rf "$TGT"
{ echo "image $IMAGE"; echo "src $(git -C "$SRC" rev-parse HEAD) dirty=$(git -C "$SRC" status --porcelain --untracked-files=no | wc -l | tr -d ' ')"
  echo "snapshot $SNAP"; cat "$SNAP/MANIFEST.txt"
  echo "$(shasum -a 256 "$OUT/cw_headstash.wasm" | cut -c1-64)  cw_headstash.wasm  $(wc -c < "$OUT/cw_headstash.wasm" | tr -d ' ') bytes"; } > "$OUT/BUILD-INFO.txt"
tail -1 "$OUT/BUILD-INFO.txt"
