#!/usr/bin/env bash
# Rebuild the Headstash K=11 membership + note (vk_id 3) browser prover (wasm-bindgen, --target web)
# and copy zk_headstash.js + zk_headstash_bg.wasm into the claim page's
# public/headstash-prover/membership/ directory.
#
# Usage: build-browser-prover.sh [DEST_DIR]
#   DEST_DIR default: <terp-core>/websites/dao-dao-ui/apps/dapp/public/headstash-prover/membership
#   Set DEST_DIR=- to build only (artifacts stay in $OUT).
# Needs: wasm-pack, nightly rustup toolchain with wasm32-unknown-unknown
# (alloc_error_hook is nightly), wasm-opt (binaryen) optional.
#
# After copying, update MEMBERSHIP_JS_SHA256 / MEMBERSHIP_WASM_SHA256 in
# public/headstash-prover/membership/worker.js with the printed hashes.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$HERE/../circuits/halo2"
TERP_CORE="$(cd "$HERE/../../../../.." && pwd)"
DEST="${1:-$TERP_CORE/websites/dao-dao-ui/apps/dapp/public/headstash-prover/membership}"
OUT="${OUT:-${TMPDIR:-/tmp}/headstash-browser-prover}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/headstash-browser-target}"
# rand 0.10 ThreadRng -> getrandom 0.3/0.4: select the crypto.getRandomValues backend.
export RUSTFLAGS="${RUSTFLAGS:-} --cfg getrandom_backend=\"wasm_js\""

rm -rf "$OUT"
wasm-pack build "$CRATE" --release --target web --out-dir "$OUT" --out-name zk_headstash \
  -- --no-default-features --features browser

for f in zk_headstash.js zk_headstash_bg.wasm; do
  test -s "$OUT/$f" || { echo "missing $OUT/$f" >&2; exit 1; }
done
# vk_id 2 (membership, depth 16) and vk_id 3 (denomination notes, depth 17).
for sym in prove_membership_witness_browser prove_note_witness_browser; do
  grep -q "$sym" "$OUT/zk_headstash.js" || { echo "export $sym missing" >&2; exit 1; }
done

echo "artifacts in $OUT:"
( cd "$OUT" && shasum -a 256 zk_headstash.js zk_headstash_bg.wasm 2>/dev/null || sha256sum zk_headstash.js zk_headstash_bg.wasm )
wc -c "$OUT/zk_headstash.js" "$OUT/zk_headstash_bg.wasm"

if [ "$DEST" != "-" ]; then
  mkdir -p "$DEST"
  cp "$OUT/zk_headstash.js" "$OUT/zk_headstash_bg.wasm" "$DEST/"
  echo "copied to $DEST (update the pinned hashes in worker.js)"
fi
