#!/usr/bin/env bash
# Reproducible build of the Headstash K=11 note (vk_id 3) + membership (vk_id 2) browser prover.
#   cargo (wasm32-unknown-unknown, feature `browser`) -> wasm-bindgen --target web -> binaryen wasm-opt -Oz
#
# Usage: build-browser-prover.sh [DEST_DIR]
#   DEST_DIR default: <terp-core>/websites/dao-dao-ui/apps/dapp/public/headstash-prover/membership
#   DEST_DIR=- builds only; artifacts stay in $OUT (default ${TMPDIR:-/tmp}/headstash-browser-prover).
#
# Pins:
#   - Rust deps: tools/headstash/Cargo.lock is the seed. Cargo resolves into a scratch copy
#     (resolver.lockfile-path), so the committed lock is never rewritten. The resolved lock's sha256 is printed.
#   - wasm-bindgen CLI: must equal the wasm-bindgen crate version in the resolved lock (checked).
#     Override the binary with WASM_BINDGEN=/path/to/wasm-bindgen.
#   - wasm-opt: binaryen 132.0.0 from npm, pinned with an sha512 integrity in scripts/wasm-opt/package-lock.json
#     (npm ci verifies it). The system wasm-opt is never used.
# Needs: nightly toolchain with wasm32-unknown-unknown, node+npm, wasm-bindgen-cli.
# Prints raw/gzip sizes and sha256 of zk_headstash.js and zk_headstash_bg.wasm (gzip -9n, no name/mtime).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
WS="$HERE/.."                      # tools/headstash cargo workspace (member circuits/halo2 = zk-headstash)
TERP_CORE="$(cd "$HERE/../../../../.." && pwd)"
DEST="${1:-$TERP_CORE/websites/dao-dao-ui/apps/dapp/public/headstash-prover/membership}"
OUT="${OUT:-${TMPDIR:-/tmp}/headstash-browser-prover}"
SCRATCH="${SCRATCH:-${TMPDIR:-/tmp}/headstash-browser-prover-scratch}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/headstash-browser-target}"
# rand 0.10 ThreadRng -> getrandom 0.3/0.4: select the crypto.getRandomValues backend.
export RUSTFLAGS="${RUSTFLAGS:-} --cfg getrandom_backend=\"wasm_js\""
BINARYEN_VERSION=132
WASM_OPT_FLAGS=(-Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext
  --enable-mutable-globals --enable-reference-types --enable-multivalue)
sha() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1 || sha256sum "$1" | cut -d' ' -f1; }
report() { local f="$1" gz; gz="$(mktemp)"; gzip -9nc "$f" >"$gz"
  printf '%-22s raw %9d sha256 %s\n%-22s gz  %9d sha256 %s\n' "$(basename "$f")" "$(wc -c <"$f")" "$(sha "$f")" "" "$(wc -c <"$gz")" "$(sha "$gz")"; rm -f "$gz"; }

rm -rf "$OUT" "$SCRATCH"; mkdir -p "$OUT" "$SCRATCH/lock" "$SCRATCH/binaryen"

# 1. binaryen wasm-opt, pinned (npm ci fails on an integrity mismatch)
cp "$HERE/wasm-opt/package.json" "$HERE/wasm-opt/package-lock.json" "$SCRATCH/binaryen/"
( cd "$SCRATCH/binaryen" && npm ci --no-audit --no-fund --ignore-scripts --silent )
WASM_OPT="$SCRATCH/binaryen/node_modules/.bin/wasm-opt"
"$WASM_OPT" --version | grep -q "version $BINARYEN_VERSION\b" || { echo "wasm-opt is not binaryen $BINARYEN_VERSION: $("$WASM_OPT" --version)" >&2; exit 1; }

# 2. cargo, resolving into a scratch lock seeded from the committed one
cp "$WS/Cargo.lock" "$SCRATCH/lock/Cargo.lock"
( cd "$WS" && cargo build ${CARGO_OFFLINE:+--offline} --config "resolver.lockfile-path=\"$SCRATCH/lock/Cargo.lock\"" \
    -j "${JOBS:-4}" --release --lib --target wasm32-unknown-unknown -p zk-headstash --no-default-features --features browser )
RAW="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/zk_headstash.wasm"
test -s "$RAW" || { echo "missing $RAW" >&2; exit 1; }

# 3. wasm-bindgen, version must match the resolved crate
WBG_LOCK="$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/version = |"/,""); print; exit}' "$SCRATCH/lock/Cargo.lock")"
WASM_BINDGEN="${WASM_BINDGEN:-wasm-bindgen}"
WBG_CLI="$("$WASM_BINDGEN" --version | awk '{print $2}')"
[ "$WBG_CLI" = "$WBG_LOCK" ] || { echo "wasm-bindgen CLI $WBG_CLI != crate $WBG_LOCK (set WASM_BINDGEN=)" >&2; exit 1; }
"$WASM_BINDGEN" --target web --out-dir "$SCRATCH/bindgen" --out-name zk_headstash "$RAW"

# 4. wasm-opt -Oz
cp "$SCRATCH/bindgen/zk_headstash.js" "$OUT/"
"$WASM_OPT" "${WASM_OPT_FLAGS[@]}" "$SCRATCH/bindgen/zk_headstash_bg.wasm" -o "$OUT/zk_headstash_bg.wasm"

for f in zk_headstash.js zk_headstash_bg.wasm; do test -s "$OUT/$f" || { echo "missing $OUT/$f" >&2; exit 1; }; done
# vk_id 2 (membership, depth 16) and vk_id 3 (denomination notes, depth 17).
for sym in prove_membership_witness_browser prove_note_witness_browser; do
  grep -q "$sym" "$OUT/zk_headstash.js" || { echo "export $sym missing" >&2; exit 1; }
done

echo "pins: binaryen $("$WASM_OPT" --version | awk '{print $3}') ($(awk -F'"' '/"integrity"/{print $4; exit}' "$HERE/wasm-opt/package-lock.json")), wasm-bindgen $WBG_CLI, $(rustc --version)"
echo "resolved Cargo.lock sha256 $(sha "$SCRATCH/lock/Cargo.lock")"
echo "before wasm-opt:"; report "$SCRATCH/bindgen/zk_headstash_bg.wasm"
echo "artifacts in $OUT (wasm-opt ${WASM_OPT_FLAGS[*]}):"; report "$OUT/zk_headstash.js"; report "$OUT/zk_headstash_bg.wasm"

if [ "$DEST" != "-" ]; then
  mkdir -p "$DEST"
  cp "$OUT/zk_headstash.js" "$OUT/zk_headstash_bg.wasm" "$DEST/"
  echo "copied to $DEST"
fi
