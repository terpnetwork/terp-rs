#!/bin/bash
# Fresh loopback terpd for the headstash membership-vk e2e, plus the headstash lane.
# All listeners on 127.0.0.1. Keys stay in the chain home (keyring-test); this
# script prints addresses only. The custody seed is created by path and only
# the lane process reads it.
#   RESET=1 bootstrap-chain.sh   -> wipe and re-init the home
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TERPD="${TERPD:-/tmp/terpd-b2339a9c-build/terpd}"
LANE_BIN="${LANE_BIN:-/tmp/hm-build-target/release/headstash-lane}"
CHAIN_HOME="${CHAIN_HOME:-/tmp/headstash-mvk-chain}"
CHAIN_ID="${CHAIN_ID:-headstash-mvk-e2e}"
RPC="http://127.0.0.1:46667"
LANE_BIND="127.0.0.1:19180"
OPSIGN="$HERE/../../hash-market/scripts/headstash-operator-sign.py"
case "$CHAIN_HOME" in /tmp/headstash-mvk-*) ;; *) echo "refusing home $CHAIN_HOME" >&2; exit 1;; esac
"$TERPD" version --long 2>/dev/null | grep -q "${TERPD_EXPECT:-b2339a9c}" || { echo "terpd is not ${TERPD_EXPECT:-b2339a9c}" >&2; exit 1; }

stop_pidfile() { local f="$1"; if [ -f "$f" ] && kill -0 "$(cat "$f")" 2>/dev/null; then kill "$(cat "$f")"; sleep 2; fi; rm -f "$f"; }

start_lane() {
  stop_pidfile "$CHAIN_HOME/lane.pid"
  local data="$CHAIN_HOME/lane-$(date +%s)"
  mkdir -p "$data"; chmod 700 "$data"; rm -f "$CHAIN_HOME/lane.log"
  HASH_MARKET_BIND="$LANE_BIND" HASH_MARKET_DATA="$data" \
  HEADSTASH_CUSTODY_SEED="$CHAIN_HOME/custody.seed" \
  HEADSTASH_OPERATOR="$(python3 "$OPSIGN" --key "$CHAIN_HOME/operator.key" --address-only)" \
    nohup "$LANE_BIN" >"$CHAIN_HOME/lane.log" 2>&1 &
  echo $! >"$CHAIN_HOME/lane.pid"
  for _ in $(seq 1 ${LANE_UP_TIMEOUT:-300}); do grep -q lane_up "$CHAIN_HOME/lane.log" 2>/dev/null && curl -fsS --max-time 2 "http://$LANE_BIND/ve/health" >/dev/null 2>&1 && break; sleep 1; done
  grep -q lane_up "$CHAIN_HOME/lane.log" || { echo "lane did not start" >&2; tail -5 "$CHAIN_HOME/lane.log" >&2; exit 1; }
  echo "$data" >"$CHAIN_HOME/lane.data"
  echo "lane up $LANE_BIND pid $(cat "$CHAIN_HOME/lane.pid") data $data"
}
if [ "${1:-}" = "lane" ]; then start_lane; exit 0; fi

if [ "${RESET:-0}" = 1 ]; then
  stop_pidfile "$CHAIN_HOME/terpd.pid"; stop_pidfile "$CHAIN_HOME/lane.pid"
  rm -rf "$CHAIN_HOME"
fi
mkdir -p "$CHAIN_HOME"; chmod 700 "$CHAIN_HOME"
umask 077

if [ ! -f "$CHAIN_HOME/config/genesis.json" ]; then
  "$TERPD" init mvk-e2e --chain-id "$CHAIN_ID" --default-denom uterp --home "$CHAIN_HOME" >"$CHAIN_HOME/init.log" 2>&1
  : >"$CHAIN_HOME/addresses.env"
  for name in owner relayer recipient1 recipient2 stranger proposer; do
    "$TERPD" keys add "$name" --keyring-backend test --home "$CHAIN_HOME" --output json >/dev/null 2>&1 </dev/null
    echo "$(echo "$name" | tr a-z A-Z)_ADDR=$("$TERPD" keys show "$name" -a --keyring-backend test --home "$CHAIN_HOME")" >>"$CHAIN_HOME/addresses.env"
  done
  source "$CHAIN_HOME/addresses.env"
  for a in "$OWNER_ADDR:${OWNER_UTERP:-100000000000}" "$RELAYER_ADDR:1000000000" "$RECIPIENT1_ADDR:1000000" "$RECIPIENT2_ADDR:1000000" "$STRANGER_ADDR:100000000" "$PROPOSER_ADDR:50000000000"; do
    "$TERPD" genesis add-genesis-account "${a%%:*}" "${a##*:}uterp" --home "$CHAIN_HOME" --keyring-backend test >>"$CHAIN_HOME/init.log" 2>&1
  done
  "$TERPD" genesis gentx owner 1000000uterp --chain-id "$CHAIN_ID" --home "$CHAIN_HOME" --keyring-backend test >>"$CHAIN_HOME/init.log" 2>&1
  "$TERPD" genesis collect-gentxs --home "$CHAIN_HOME" >>"$CHAIN_HOME/init.log" 2>&1
  # custody seed + lane operator key: written by path, never printed or read here
  python3 -c 'import os,sys; open(sys.argv[1],"wb").write(os.urandom(32)); os.chmod(sys.argv[1],0o600)' "$CHAIN_HOME/custody.seed"
  python3 -c 'import os,sys; open(sys.argv[1],"wb").write(os.urandom(32)); os.chmod(sys.argv[1],0o600)' "$CHAIN_HOME/operator.key"
  start_lane
  CUSTODY_PUB_HEX="$(cat "$(cat "$CHAIN_HOME/lane.data")/custody.pub")"
  python3 "$HERE/patch_genesis.py" "$CHAIN_HOME" "$CUSTODY_PUB_HEX" "http://$LANE_BIND"
  "$TERPD" genesis validate --home "$CHAIN_HOME" >>"$CHAIN_HOME/init.log" 2>&1
  # browser e2e: the page (served from 127.0.0.1:3921) reads REST directly; loopback-only test chain
  if [ "${REST_CORS:-0}" = 1 ]; then python3 - "$CHAIN_HOME/config/app.toml" <<'PY'
import re,sys; p=sys.argv[1]; s=open(p).read()
s=re.sub(r'(?m)^enabled-unsafe-cors = false', 'enabled-unsafe-cors = true', s); open(p,'w').write(s)
PY
  fi
fi

if ! curl -fsS --max-time 2 "$RPC/status" >/dev/null 2>&1; then
  HASHMERCHANT_SIDECAR_URL="http://$LANE_BIND" nohup "$TERPD" start --home "$CHAIN_HOME" \
    --log_level "*:info,x/hashmerchant:debug" >>"$CHAIN_HOME/terpd.log" 2>&1 &
  echo $! >"$CHAIN_HOME/terpd.pid"
fi
curl -fsS --max-time 2 "http://$LANE_BIND/ve/health" >/dev/null 2>&1 || start_lane
for _ in $(seq 1 90); do
  h=$(curl -fsS --max-time 2 "$RPC/status" 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["sync_info"]["latest_block_height"])' 2>/dev/null || echo 0)
  [ "$h" -ge 3 ] && break; sleep 1
done
[ "$h" -ge 3 ] || { echo "chain not producing blocks" >&2; tail -20 "$CHAIN_HOME/terpd.log" >&2; exit 1; }
echo "chain $CHAIN_ID height $h rpc $RPC rest http://127.0.0.1:41327 terpd pid $(cat "$CHAIN_HOME/terpd.pid")"
cat "$CHAIN_HOME/addresses.env"
echo CHAIN_READY
