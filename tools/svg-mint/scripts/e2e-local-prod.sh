#!/bin/sh
# groot-wan multi-chain suite for the svg mint.
# Spawns Terp, Zakura, ztreamer, Anvil, Bitcoin, and the Linux fed ELF.
# The ZEC wallet checkpoint is svg-mint-zrt in a temp directory.
set -eu
ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

cargo build -p svg-mint-zrt
export SVG_MINT_ZRT_BIN="$ROOT/target/debug/svg-mint-zrt"

if [ -z "${DREGG_NODE_BIN:-}" ]; then
  for cand in \
    "$ROOT/target/aarch64-unknown-linux-musl/release/svg-mint-fed" \
    "$ROOT/target/aarch64-unknown-linux-gnu/release/svg-mint-fed" \
    "$ROOT/target/linux-gnu/svg-mint-fed"
  do
    if [ -f "$cand" ]; then
      DREGG_NODE_BIN="$cand"
      break
    fi
  done
fi
if [ -z "${DREGG_NODE_BIN:-}" ] || [ ! -f "$DREGG_NODE_BIN" ]; then
  echo "DREGG_NODE_BIN must be a Linux ELF of svg-mint-fed built from this repo"
  exit 1
fi
export DREGG_NODE_BIN

if [ -z "${ZTREAMER_BIN:-}" ]; then
  for cand in \
    "$ROOT/runtimes/ztreamer/target/release/ztreamer" \
    "$HOME/abstract/terp-core/crates/ztreamer/target/release/ztreamer"
  do
    if [ -f "$cand" ]; then
      ZTREAMER_BIN="$cand"
      break
    fi
  done
fi
export ZTREAMER_BIN="${ZTREAMER_BIN:-}"
export ZTREAMER_BIND="${ZTREAMER_BIND:-127.0.0.1:19067}"
export ZTREAMER_GRPC="$ZTREAMER_BIND"

if [ -z "${ZAKURAD_BIN:-}" ] && [ -f "$HOME/abstract/terp-core/crates/zakura/target/zakura-regtest-e2e-linux/zakurad" ]; then
  ZAKURAD_BIN="$HOME/abstract/terp-core/crates/zakura/target/zakura-regtest-e2e-linux/zakurad"
fi
export ZAKURAD_BIN="${ZAKURAD_BIN:-}"

export SVG_MINT_ROOT="$ROOT"
export TERP_CRATES="$HOME/abstract/terp-core/crates"
export SVG_MINT_ARTIFACTS="${SVG_MINT_ARTIFACTS:-$TERP_CRATES/cw-infuser/artifacts}"
export SVG_MINT_REQUIRE_ZEC_WALLET=1
export PATH="/usr/local/bin:/Applications/Docker.app/Contents/Resources/bin:$HOME/.cargo/bin:$PATH"
export DOCKER_HOST="${DOCKER_HOST:-unix:///Users/returniflost/.docker/run/docker.sock}"

cd "$HOME/abstract/terp-core/crates/ict-rs"
exec cargo run --example svg_mint_local_prod -p ict-rs-cw-orch --features docker,zakura,dregg-node,ethereum,bitcoin,svg-mint-product
