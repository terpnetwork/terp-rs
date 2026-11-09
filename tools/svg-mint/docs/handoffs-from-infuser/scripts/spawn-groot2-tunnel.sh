#!/bin/sh
# Spawn svg-mint-fed on groot2 and publish via Cloudflare tunnel.
# Usage on groot2:  DREGG_NODE_BIN=... CLOUDFLARED_TOKEN=... ./spawn-groot2-tunnel.sh
set -eu
BIND="${SVG_MINT_FED_BIND:-0.0.0.0:8421}"
BIN="${DREGG_NODE_BIN:-$HOME/abstract/terp-core/crates/dregg/svg-mint/target/release/svg-mint-fed}"
STORE="${SVG_MINT_FED_STORE:-$HOME/.terp/svg-mint-fed-cell.json}"
export SVG_MINT_FED_BIND="$BIND"
export SVG_MINT_FED_STORE="$STORE"
if [ ! -x "$BIN" ]; then
  echo "missing svg-mint-fed ELF at $BIN" >&2
  exit 1
fi
echo "starting $BIN on $BIND"
nohup "$BIN" >"$HOME/.terp/svg-mint-fed.log" 2>&1 &
echo $! >"$HOME/.terp/svg-mint-fed.pid"
if command -v cloudflared >/dev/null 2>&1 && [ -n "${CLOUDFLARED_TOKEN:-}" ]; then
  nohup cloudflared tunnel run --token "$CLOUDFLARED_TOKEN" \
    >"$HOME/.terp/cloudflared-dregg.log" 2>&1 &
  echo $! >"$HOME/.terp/cloudflared-dregg.pid"
  echo "cloudflared started (token tunnel). Point hostname at http://127.0.0.1:8421"
else
  echo "cloudflared skipped: set CLOUDFLARED_TOKEN and install cloudflared"
fi
echo "DREGG health: curl -sS http://127.0.0.1:8421/health"
