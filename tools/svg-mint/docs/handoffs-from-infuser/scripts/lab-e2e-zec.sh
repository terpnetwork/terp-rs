#!/bin/sh
# Live lab e2e for ztreamer CompactTxStreamer (ict-rs Zakura sibling). Run ON groot-wan.
# Does not start cookie-harness lightwalletd or groot2 house. DREGG must not query public LWD.
set -eu
export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"
export HOME="${HOME:-/Users/returniflost}"
ROOT="${TERP_CORE:-$HOME/abstract/terp-core/crates}"
export ZAKURAD_BIN="${ZAKURAD_BIN:-$ROOT/zakura/target/zakura-regtest-e2e-linux/zakurad}"
export DREGG_NODE_BIN="${DREGG_NODE_BIN:-$ROOT/dregg/svg-mint/target/linux-gnu/svg-mint-fed}"
ZTREAMER_DIR="${ZTREAMER_DIR:-$ROOT/ztreamer}"
if [ -z "${ZTREAMER_BIN:-}" ] && [ -f "$ZTREAMER_DIR/Cargo.toml" ]; then
  if [ ! -x "$ZTREAMER_DIR/target/release/ztreamer" ]; then
    (cd "$ZTREAMER_DIR" && cargo build --release --bin ztreamer)
  fi
  export ZTREAMER_BIN="$ZTREAMER_DIR/target/release/ztreamer"
fi
export ZTREAMER_BIND="${ZTREAMER_BIND:-127.0.0.1:19067}"
if [ -z "${ZCLI_BIN:-}" ]; then
  for c in "$ROOT/zcli/target/release/zcli" "$ROOT/zcli/bin/zcli/target/release/zcli"; do
    if [ -x "$c" ]; then export ZCLI_BIN="$c"; break; fi
  done
fi
export ZCLI_ENDPOINT="${ZCLI_ENDPOINT:-http://127.0.0.1:19067}"
export ZTREAMER_GRPC="${ZTREAMER_GRPC:-$ZTREAMER_BIND}"

if ! command -v docker >/dev/null; then
  echo '{"live":false,"error":"docker not in PATH (need /usr/local/bin)"}'
  exit 2
fi
if ! docker info >/dev/null 2>&1; then
  echo '{"live":false,"error":"docker daemon not up"}'
  exit 2
fi
if docker ps --format '{{.Image}} {{.Names}}' 2>/dev/null | grep -qi lightwalletd; then
  echo '{"live":false,"lwd_absent":false,"error":"lightwalletd container running; stop it for ztreamer prove"}'
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
out=$(cargo run --example svg_mint_local_prod -p ict-rs-cw-orch \
  --features docker,zakura,dregg-node,ethereum,bitcoin)
echo "$out"

python3 -c '
import json,sys,re
text=sys.argv[1]
objs=[]
dec=json.JSONDecoder()
i=0
while i < len(text):
    while i < len(text) and text[i] not in "{[":
        i+=1
    if i>=len(text):
        break
    try:
        obj,end=dec.raw_decode(text,i)
        objs.append(obj)
        i=end
    except json.JSONDecodeError:
        i+=1
blob=None
for o in reversed(objs):
    if isinstance(o,dict) and "live" in o:
        blob=o
        break
if blob is None:
    raise SystemExit("no live JSON object in cargo stdout")
if blob.get("live") is not True:
    raise SystemExit("live!=true")
vendor=str(blob.get("compact_vendor") or "")
if "ztreamer" not in vendor.lower():
    raise SystemExit("compact_vendor is not Ztreamer: %r" % vendor)
wal=str(blob.get("wallet") or "")
if wal != "zcli":
    raise SystemExit("wallet is not zcli: %r" % wal)
if blob.get("lwd_absent") is not True:
    raise SystemExit("lwd_absent!=true")
circuit=blob.get("zec_tip_circuit") or {}
if circuit.get("ok") is not True:
    raise SystemExit("zec_tip_circuit not exercised: %r" % circuit)
if circuit.get("pay_targets_closed") is not True:
    raise SystemExit("zec_tip_circuit did not close pay-targets: %r" % circuit)
grpc=str(blob.get("compact_grpc") or "")
if not grpc:
    raise SystemExit("compact_grpc missing")
if "8232" in grpc:
    raise SystemExit("compact_grpc must not be Zakura JSON-RPC :8232")
walk=blob.get("pweb_walk") or {}
if walk.get("ok") is not True:
    raise SystemExit("pweb_walk not ok: %r" % walk)
if walk.get("unpaid_grant_blocked") is not True:
    raise SystemExit("unpaid passkey-grant must be blocked: %r" % walk)
if walk.get("paid_grant_ok") is not True:
    raise SystemExit("paid passkey-grant not ok: %r" % walk)
if walk.get("admin_stats_ok") is not True:
    raise SystemExit("admin stats not ok: %r" % walk)
if not walk.get("paid_c"):
    raise SystemExit("pweb_walk missing paid Pedersen c: %r" % walk)
' "$out"
