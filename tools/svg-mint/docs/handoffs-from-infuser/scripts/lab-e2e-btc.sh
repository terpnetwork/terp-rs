#!/bin/sh
# Live lab e2e for ultra-light BTC (ict-rs bitcoind in Docker). Run ON groot-wan.
# Does not start groot2 house or Akash porch. DREGG must not query public SaaS.
set -eu
export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"
export HOME="${HOME:-/Users/returniflost}"
ROOT="${TERP_CORE:-$HOME/abstract/terp-core/crates}"
export ZAKURAD_BIN="${ZAKURAD_BIN:-$ROOT/zakura/target/zakura-regtest-e2e-linux/zakurad}"
export DREGG_NODE_BIN="${DREGG_NODE_BIN:-$ROOT/dregg/svg-mint/target/linux-gnu/svg-mint-fed}"

if ! command -v docker >/dev/null; then
  echo '{"live":false,"error":"docker not in PATH (need /usr/local/bin)"}'
  exit 2
fi
if ! docker info >/dev/null 2>&1; then
  echo '{"live":false,"error":"docker daemon not up"}'
  exit 2
fi
if [ ! -x "$ZAKURAD_BIN" ]; then
  echo "{\"live\":false,\"error\":\"ZAKURAD_BIN missing: $ZAKURAD_BIN\"}"
  exit 2
fi
if [ ! -x "$DREGG_NODE_BIN" ]; then
  echo "{\"live\":false,\"error\":\"DREGG_NODE_BIN missing: $DREGG_NODE_BIN\"}"
  exit 2
fi

cd "$ROOT/ict-rs"
cargo run --example svg_mint_local_prod -p ict-rs-cw-orch \
  --features docker,zakura,dregg-node,ethereum,bitcoin
