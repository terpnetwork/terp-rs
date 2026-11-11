#!/usr/bin/env bash
# Copy runtime sources out of terp-core/crates/dregg/svg-mint into this workspace.
# Rewrites dregg_svg_mint:: → dregg:: so the fed crate imports the dregg library.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OLD="${SVG_MINT_OLD:-$HOME/abstract/terp-core/crates/dregg/svg-mint}"
if [[ ! -d "$OLD/src" ]]; then
  echo "missing $OLD/src" >&2
  exit 1
fi

mkdir -p "$ROOT/crates/dregg/src" \
  "$ROOT/crates/dregg/tests" \
  "$ROOT/crates/svg-mint-fed/src" \
  "$ROOT/crates/svg-mint-zrt/src" \
  "$ROOT/crates/cw-dregg-svg-gate" \
  "$ROOT/docs"

rsync -a --delete --exclude bin "$OLD/src/" "$ROOT/crates/dregg/src/"
if [[ -d "$OLD/tests" ]]; then
  rsync -a "$OLD/tests/" "$ROOT/crates/dregg/tests/"
fi
cp "$OLD/src/bin/svg-mint-fed.rs" "$ROOT/crates/svg-mint-fed/src/main.rs"
cp "$OLD/src/bin/svg-mint-zrt.rs" "$ROOT/crates/svg-mint-zrt/src/main.rs"

if [[ -d "$OLD/cw-gate/src" ]]; then
  mkdir -p "$ROOT/crates/cw-dregg-svg-gate/src"
  rsync -a --exclude target --exclude Cargo.toml --exclude Cargo.lock "$OLD/cw-gate/src/" "$ROOT/crates/cw-dregg-svg-gate/src/"
fi
if [[ -d "$OLD/docs" ]]; then
  rsync -a "$OLD/docs/" "$ROOT/docs/"
fi

# Library crate name is `dregg`.
find "$ROOT/crates" -name '*.rs' -print0 | xargs -0 sed -i.bak -e 's/dregg_svg_mint::/dregg::/g'
find "$ROOT/crates" -name '*.bak' -delete

echo "bootstrapped from $OLD"
