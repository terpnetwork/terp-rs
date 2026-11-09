#!/bin/sh
# Daily Marain + Latent Benders pass on groot.
# After the session exists, pack it and open the factory create so the mint window starts.
# The cold key still signs that one page. A finished day is recorded in onchain.json and is not repeated.
set -eu
ROOT="$(CDPATH= cd "$(dirname "$0")/.." && pwd)"
REPO="$(CDPATH= cd "$ROOT/../.." && pwd)"
STATE="${SVGC_STATE:-$HOME/Library/Application Support/terp-svg-collections}"
mkdir -p "$STATE/days" "$HOME/Library/Logs"
export SVGC_MODEL="${SVGC_MODEL:-opencode/deepseek-v4.1-flash-free}"
export OPENCODE_BIN="${OPENCODE_BIN:-$HOME/.opencode/bin/opencode2}"
export SVGC_RUNTIME="${SVGC_RUNTIME:-opencode}"
export SVGC_BIN="${SVGC_BIN:-$REPO/target/debug/svgc}"
export PATH="/opt/homebrew/bin:/usr/local/bin:$REPO/target/debug:$PATH"
export TERP_SCRIPT_BROADCAST=1
export STATE_FILE="${STATE_FILE:-$STATE/cw-orch-state.json}"
PY="${PYTHON:-/usr/bin/python3}"
DAILY="$ROOT/harness/daily.py"
CACHE="$STATE/psychedex-cache-v1.json"
PERSONA="$ROOT/personas/latent-benders.json"
FACTORY="${SVGC_FACTORY:-terp1wg5z9rcwpw5kvrl4g78ees2l9ejptrhrx4t2k56nq6t27uly87zssujm80}"
GRANTER="${SVGC_GRANTER:-terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052}"
GRPC="${SVGC_GRPC:-http://127.0.0.1:19090}"
RPC="${SVGC_RPC:-http://127.0.0.1:26658}"
if [ ! -x "$SVGC_BIN" ]; then
  echo "svgc missing at $SVGC_BIN" >&2
  exit 1
fi
if [ ! -f "$CACHE" ]; then
  echo "building psychedex cache"
  "$PY" "$DAILY" build-cache --out "$CACHE" --days 7 --max-fetch 40
fi
while true; do
  DAY="$(date -u +%F)"
  OUT="$STATE/days/$DAY"
  mkdir -p "$OUT"
  echo "svg collections $DAY"
  "$PY" "$DAILY" commit --day "$DAY" --state "$STATE/state.json"
  if [ ! -f "$OUT/marain-draft.json" ]; then
    "$PY" "$DAILY" marain-draft --out "$OUT/marain-draft.json" --batch "$DAY"
  fi
  if [ ! -f "$OUT/day-record.partial.json" ]; then
    "$PY" "$DAILY" symbient-day \
      --persona "$PERSONA" --day "$DAY" --state "$STATE/state.json" \
      --out "$OUT" --source cache --cache "$CACHE" --frames 1
  fi
  if [ ! -f "$OUT/packed/packed.svp" ]; then
    "$SVGC_BIN" symbient "$OUT/day-build.json" "$OUT/packed"
  fi
  if [ ! -f "$OUT/day-record.json" ]; then
    cp "$OUT/packed/build-report.json" "$OUT/packed/sanitize-report.json" "$OUT/"
    "$PY" "$DAILY" finalize --out "$OUT" --state "$STATE/state.json"
  fi
  if [ ! -f "$OUT/onchain.json" ]; then
    "$PY" - "$OUT" "$GRANTER" << 'PY'
import base64, json, sys
from pathlib import Path
out, granter = Path(sys.argv[1]), sys.argv[2]
rec = json.loads((out / "day-record.json").read_text())
packed = (out / "packed" / "packed.svp").read_bytes()
seed = bytes.fromhex(rec["record_sha256"])
msg = {
  "create_series_collection": {
    "series": "latent-benders",
    "label": f"latent-benders-{rec['persona']}-{rec['day']}",
    "instantiate_msg": {
      "name": f"Latent Benders: A Study — Day {rec['day_index']} ({rec['day']})",
      "symbol": "BENDERS",
      "minter": None,
      "creator": granter,
      "withdraw_address": granter,
      "collection_info_extension": {
        "seed": base64.b64encode(seed).decode(),
        "packed": base64.b64encode(packed).decode(),
        "price_tiers": [],
        "total": 100,
        "mint_start_time": None,
        "mint_end_time": "86400000000000",
        "whitelist": None,
        "require_auth": False,
      },
    },
  }
}
(out / "create-latent-benders.json").write_text(json.dumps(msg, indent=1) + "\n")
print("factory message", out / "create-latent-benders.json")
PY
    if lsof -nP -iTCP:8741 -sTCP:LISTEN >/dev/null 2>&1; then
      echo "signer page busy; will retry this day before sleeping until tomorrow"
      sleep 600
      continue
    fi
    echo "chain grpc $GRPC rpc $RPC"
    if ! NEXT="$("$PY" - "$GRPC" "$FACTORY" << 'PY'
import base64, json, subprocess, sys
grpc, factory = sys.argv[1], sys.argv[2]
host = grpc.split("://", 1)[-1]
query = base64.b64encode(b'{"series":{"series":"latent-benders"}}').decode()
proc = subprocess.run(
    ["grpcurl", "-plaintext", "-d", json.dumps({"address": factory, "query_data": query}),
     host, "cosmwasm.wasm.v1.Query/SmartContractState"],
    capture_output=True, text=True)
if proc.returncode != 0:
    sys.stderr.write(proc.stderr or proc.stdout)
    sys.exit(1)
info = json.loads(base64.b64decode(json.loads(proc.stdout)["data"]))
print(int(info.get("next_allowed_at") or 0))
PY
    )"; then
      echo "series query on $GRPC failed; signer stays closed" >&2
      sleep 60
      continue
    fi
    NOW="$(date +%s)"
    if [ "$NEXT" -gt "$NOW" ]; then
      echo "next collection allowed at $NEXT; waiting $((NEXT - NOW))s"
      sleep $((NEXT - NOW + 2))
    fi
    echo "opening factory create for $DAY"
    SVG_OPS="${SVG_OPS:-$REPO/target/debug/svg_ops}"
    # The runtime key is the operator. It broadcasts with the granter's authz and feegrant.
    if [ -z "${MAIN_MNEMONIC:-}" ] && [ -f "$HOME/.cw-orch/MAIN_MNEMONIC.mnemonic" ]; then
      MAIN_MNEMONIC="$(tr -d '\n' < "$HOME/.cw-orch/MAIN_MNEMONIC.mnemonic")"
      export MAIN_MNEMONIC
    fi
    set +e
    "$SVG_OPS" operator-exec \
      --signer env \
      --chain-id morocco-1 --network morocco-1 \
      --grpc "$GRPC" --rpc "$RPC" \
      --granter "$GRANTER" --contract "$FACTORY" \
      --msg-file "$OUT/create-latent-benders.json" --broadcast > "$OUT/broadcast.log" 2>&1
    code=$?
    set -e
    if [ "$code" -eq 0 ] && grep -q '^created ' "$OUT/broadcast.log"; then
      grep '^created ' "$OUT/broadcast.log" | tail -1 > "$OUT/onchain.json"
      echo "on chain: $(cat "$OUT/onchain.json")"
    else
      echo "factory create for $DAY did not land (exit $code); see $OUT/broadcast.log" >&2
      sleep 600
      continue
    fi
  fi
  # Writes need the collection key through MinIO on groot2, not the public edge.
  if ! nc -z -G 1 127.0.0.1 19100 2>/dev/null; then
    ssh -f -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 \
      -L 127.0.0.1:19100:127.0.0.1:9000 groot2 || echo "minio tunnel did not open" >&2
  fi
  # A created collection is not done until its study is on the live prefix.
  # A failed copy retries; it does not wait until the next UTC day.
  published=0
  for attempt in 1 2 3 4 5; do
    if SVGC_PUBLISH=1 \
      SVGC_LIVE_PREFIX="${SVGC_LIVE_PREFIX:-static/svg-collections/pm}" \
      "$PY" "$ROOT/harness/publish_study.py"; then
      published=1
      break
    fi
    echo "study publish failed (attempt $attempt); retrying" >&2
    sleep 20
  done
  if [ "$published" != 1 ]; then
    echo "study is still local; will try again before the next day" >&2
    sleep 120
    continue
  fi
  "$PY" -c 'import time; now=time.time(); nxt=(int(now)//86400+1)*86400; time.sleep(max(60, nxt-now+30))'
done
