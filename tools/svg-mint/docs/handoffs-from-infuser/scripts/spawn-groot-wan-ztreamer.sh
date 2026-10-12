#!/bin/sh
# Lab (groot-wan / groot.local): ztreamer CompactTxStreamer + svg-mint-fed for Zallet.
# Does not start electriccoinco/lightwalletd. Does not copy ELFs to groot2.
set -eu
export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"
ROOT="${TERP_CORE:-$HOME/abstract/terp-core/crates}"
ZTREAMER_DIR="${ZTREAMER_DIR:-$ROOT/ztreamer}"
MINT="$ROOT/dregg/svg-mint"
BIND="${ZTREAMER_BIND:-127.0.0.1:19067}"
FED_BIND="${SVG_MINT_FED_BIND:-127.0.0.1:8421}"
INDEX_DIR="${ZTREAMER_INDEX_DIR:-$HOME/.terp/ztreamer-index}"
BIND_PORT="${BIND##*:}"
FED_PORT="${FED_BIND##*:}"

if docker ps --format '{{.Image}} {{.Names}}' 2>/dev/null | grep -qi lightwalletd; then
  echo '{"ok":false,"error":"lightwalletd container running; stop it"}'
  exit 2
fi
if [ ! -f "$ZTREAMER_DIR/Cargo.toml" ]; then
  echo "{\"ok\":false,\"error\":\"ztreamer missing at $ZTREAMER_DIR\"}"
  exit 2
fi

mkdir -p "$INDEX_DIR" "$HOME/.terp"
(cd "$ZTREAMER_DIR" && cargo build --release --bin ztreamer)
ZTREAMER_BIN="$ZTREAMER_DIR/target/release/ztreamer"
(cd "$MINT" && cargo build --release --bin svg-mint-fed)
FED_BIN="$MINT/target/release/svg-mint-fed"

if command -v lsof >/dev/null; then
  old=$(lsof -nP -iTCP:"$BIND_PORT" -sTCP:LISTEN -t 2>/dev/null || true)
  if [ -n "$old" ]; then echo "$old" | xargs kill 2>/dev/null || true; sleep 1; fi
fi
nohup "$ZTREAMER_BIN" --bind "$BIND" --index-dir "$INDEX_DIR" --chain-name regtest \
  >"$HOME/.terp/ztreamer.log" 2>&1 &
echo $! >"$HOME/.terp/ztreamer.pid"
sleep 1

if [ -f "$HOME/.terp/dregg-svg.env" ]; then
  set -a
  # shellcheck disable=SC1090
  . "$HOME/.terp/dregg-svg.env"
  set +a
fi
export ZCASH_DEVTOOL_SERVER="$BIND"
export SVG_MINT_FED_BIND="$FED_BIND"

if command -v lsof >/dev/null; then
  fold=$(lsof -nP -iTCP:"$FED_PORT" -sTCP:LISTEN -t 2>/dev/null || true)
  if [ -n "$fold" ]; then echo "$fold" | xargs kill 2>/dev/null || true; sleep 1; fi
fi
nohup env ZCASH_DEVTOOL_SERVER="$BIND" SVG_MINT_FED_BIND="$FED_BIND" "$FED_BIN" \
  >"$HOME/.terp/svg-mint-fed.log" 2>&1 &
echo $! >"$HOME/.terp/svg-mint-fed.pid"
sleep 1

port_ok=false
if command -v nc >/dev/null && nc -z 127.0.0.1 "$BIND_PORT" 2>/dev/null; then
  port_ok=true
fi
health=$(curl -sS --max-time 3 "http://${FED_BIND}/health" 2>/dev/null || echo '{}')

export PORT_OK="$port_ok" BIND="$BIND" FED_BIND="$FED_BIND" HEALTH="$health"
export ZTREAMER_BIN FED_BIN
python3 - <<'PY'
import json, os
h = os.environ.get("HEALTH", "{}")
try:
    d = json.loads(h)
except Exception:
    d = {}
print(json.dumps({
    "ok": os.environ.get("PORT_OK") == "true",
    "compact_grpc": os.environ.get("BIND"),
    "compact_vendor": "Ztreamer",
    "lwd_absent": True,
    "fed_bind": os.environ.get("FED_BIND"),
    "health_ok": d.get("ok"),
    "checkout_ready": d.get("checkout_ready"),
    "service": d.get("service"),
    "ztreamer_bin": os.environ.get("ZTREAMER_BIN"),
    "fed_bin": os.environ.get("FED_BIN"),
}))
PY
