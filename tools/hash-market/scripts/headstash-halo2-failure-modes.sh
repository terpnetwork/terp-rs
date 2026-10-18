#!/bin/bash
# One pass of the headstash failure modes on a real 4-validator chain.
# Campaign instantiate is headstash-halo2-campaign.sh (vk_id 1 and the blob hash).
# Four loopback lanes verify one Halo2 claim. Binds stay on 127.0.0.1.
# Does not print keys, seeds, proofs, or the vk hash.
# After the replay is rejected, the owner calls SweepUnclaimed. The
# recipient was stored on the campaign. Leftover uterp must leave the
# contract. A lying lane sets both HEADSTASH_LIE_ROOT and HEADSTASH_LAB_LIE.
set -euo pipefail

export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"
export ICT_PUBLISH_HOST=127.0.0.1

WT="${WT:-/Users/returniflost/abstract/terp-core/crates/terp-rs-headstash-epoch}"
ICT="${ICT:-/Users/returniflost/abstract/terp-core/crates/ict-rs}"
TERPD="${TERPD:-/Users/returniflost/abstract/terp-core/build/terpd-v6.4.0-darwin-arm64}"
LANE="${LANE:-$WT/tools/hash-market/target/release/headstash-lane}"
WASM="${WASM:-/tmp/cw_headstash-sweep.wasm}"
WAVS="${WAVS:-/tmp/headstash-epoch-chain/wavs.json}"
WORK="${WORK:-$(mktemp -d /tmp/headstash-fm-halo2.XXXXXX)}"
CHAIN_ID=headstash-fm-1
PORTS=(18521 18522 18523 18524)

mkdir -p "$WORK"
chmod 700 "$WORK"
REPORT="$WORK/report.txt"
: > "$REPORT"
STOP="$WORK/stop"
CHAIN_FILE="$WORK/chain.env"
: > "$WORK/pids"

lane_pids=()
chain_pid=""

cleanup() {
  touch "$STOP" 2>/dev/null || true
  if [ -f "$WORK/pids" ]; then
    while read -r pid; do
      kill "$pid" 2>/dev/null || true
    done < "$WORK/pids"
  fi
  if [ -n "${chain_pid}" ]; then
    wait "$chain_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

report() {
  local name="$1" ok="$2" ms="$3" detail="$4"
  local word="FAIL"
  if [ "$ok" = "1" ]; then word="PASS"; fi
  printf '%s %s %sms %s\n' "$word" "$name" "$ms" "$detail" | tee -a "$REPORT"
}

now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }

start_lane() {
  local i="$1" lie="$2"
  local port="${PORTS[$i]}"
  local data="$WORK/lane$i"
  rm -rf "$data"
  mkdir -p "$data"
  chmod 700 "$data"
  if [ "$lie" = "1" ]; then
    HASH_MARKET_BIND="127.0.0.1:${port}" \
    HASH_MARKET_DATA="$data" \
    HEADSTASH_CUSTODY_SEED="$WORK/custody.seed" \
    HEADSTASH_OPERATOR="$HEADSTASH_OPERATOR" \
    HASHMERCHANT_CHAIN_UID=headstash-claim \
    HEADSTASH_LIE_ROOT=1 \
    HEADSTASH_LAB_LIE=1 \
    "$LANE" > "$WORK/lane$i.log" 2>&1 &
  else
    HASH_MARKET_BIND="127.0.0.1:${port}" \
    HASH_MARKET_DATA="$data" \
    HEADSTASH_CUSTODY_SEED="$WORK/custody.seed" \
    HEADSTASH_OPERATOR="$HEADSTASH_OPERATOR" \
    HASHMERCHANT_CHAIN_UID=headstash-claim \
    "$LANE" > "$WORK/lane$i.log" 2>&1 &
  fi
  lane_pids[$i]=$!
  echo "$!" >> "$WORK/pids"
}

wait_lane() {
  local i="$1"
  local port="${PORTS[$i]}"
  local n=0
  while [ "$n" -lt 180 ]; do
    if curl -fsS --max-time 1 "http://127.0.0.1:${port}/health" >/dev/null 2>&1; then
      return 0
    fi
    if [ -n "${lane_pids[$i]:-}" ] && ! kill -0 "${lane_pids[$i]}" 2>/dev/null; then
      echo "lane $i exited" >&2
      tail -20 "$WORK/lane$i.log" >&2 || true
      exit 1
    fi
    n=$((n + 1))
    sleep 2
  done
  echo "lane $i did not come up" >&2
  tail -20 "$WORK/lane$i.log" >&2 || true
  exit 1
}

stop_lane() {
  local i="$1"
  if [ -n "${lane_pids[$i]:-}" ]; then
    kill "${lane_pids[$i]}" 2>/dev/null || true
    wait "${lane_pids[$i]}" 2>/dev/null || true
    lane_pids[$i]=""
  fi
  local n=0
  while [ "$n" -lt 30 ]; do
    if ! curl -fsS --max-time 1 "http://127.0.0.1:${PORTS[$i]}/health" >/dev/null 2>&1; then
      return 0
    fi
    n=$((n + 1))
    sleep 0.25
  done
  echo "lane $i did not stop" >&2
  exit 1
}

post_json() {
  local port="$1" path="$2" body="$3" out="$4" auth="${5:-public}"
  local -a auth_hdr=()
  if [ "$auth" = "operator" ]; then
    local sig pubhex
    sig=$(python3 "$WT/tools/hash-market/scripts/headstash-operator-sign.py" \
      --key "$WORK/op.seed" --method POST --path "$path" --body "$body" \
      --domain "${HEADSTASH_AUTH_DOMAIN:-hash-market.tree-write.v1}")
    pubhex=$(cat "$WORK/op.pub")
    auth_hdr=(-H "x-hash-market-pubkey: ${pubhex}" -H "x-hash-market-signature: ${sig}")
  fi
  curl -sS --max-time 900 -o "$out" -w "%{http_code}" \
    -H 'content-type: application/json' \
    "${auth_hdr[@]}" \
    -X POST "http://127.0.0.1:${port}${path}" \
    --data-binary @"$body"
}

install_claim_on() {
  local i="$1"
  local port="${PORTS[$i]}"
  local code
  code=$(post_json "$port" /ve/headstash/campaign "$WORK/sidecar-campaign.json" "$WORK/camp-$i.json" operator)
  if [ "$code" != "200" ]; then
    echo "campaign lane $i http $code" >&2
    return 1
  fi
  code=$(post_json "$port" /ve/headstash/claims "$WORK/claim-post.json" "$WORK/claim-$i.json")
  if [ "$code" != "200" ]; then
    echo "claim lane $i http $code" >&2
    return 1
  fi
  python3 - "$WORK/claim-$i.json" << 'PY'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("accepted") != [0] or body.get("rejected"):
    raise SystemExit("claim not accepted")
PY
  code=$(post_json "$port" /ve/headstash/seal "$WORK/seal.json" "$WORK/seal-$i.json" operator)
  if [ "$code" != "200" ]; then
    echo "seal lane $i http $code" >&2
    return 1
  fi
  python3 - "$WORK/seal-$i.json" << 'PY'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("algo") != "headstash-accepted" or body.get("chain_uid") != "headstash-claim":
    raise SystemExit("seal shape")
PY
}

query_count() {
  "$TERPD" query hashmerchant root headstash-claim headstash-accepted \
    --node "$NODE" -o json 2>/dev/null | python3 -c '
import json,sys
try:
    text=json.dumps(json.load(sys.stdin))
except Exception:
    print("")
    raise SystemExit
key="\"attestation_count\":"
i=text.find(key)
if i<0:
    print("")
    raise SystemExit
rest=text[i+len(key):].lstrip().lstrip(":").lstrip().lstrip("\"")
n=""
for ch in rest:
    if ch.isdigit():
        n+=ch
    else:
        break
print(n)
'
}

chain_height() {
  curl -fsS --max-time 3 "http://127.0.0.1:${RPC}/status" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["sync_info"]["latest_block_height"])'
}

wait_count() {
  local want="$1" secs="$2"
  local start got=""
  start=$(date +%s)
  while true; do
    got=$(query_count || true)
    if [ "$got" = "$want" ]; then
      echo "$got"
      return 0
    fi
    if [ $(( $(date +%s) - start )) -ge "$secs" ]; then
      echo "${got:-none}"
      return 1
    fi
    sleep 2
  done
}

contract_amount() {
  "$TERPD" query bank balances "$CONTRACT" --node "$NODE" -o json | python3 -c '
import json,sys
raw=json.load(sys.stdin)
for coin in raw.get("balances") or []:
    if coin.get("denom")=="uterp":
        print(coin.get("amount","0"))
        raise SystemExit
print("0")
'
}

tx_wait() {
  local gas="$1"
  shift
  local out="$WORK/tx-last.json"
  "$TERPD" "$@" \
    --from validator \
    --home "$HOME_CHAIN" \
    --keyring-backend test \
    --chain-id "$CHAIN_ID" \
    --node "$NODE" \
    --gas "$gas" \
    --gas-prices 0.025uterp \
    -y -b sync -o json > "$out"
  local hash
  hash=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("txhash",""))' "$out")
  case "$hash" in
    [0-9A-Fa-f]*) ;;
    *) echo "no txhash" >&2; return 1 ;;
  esac
  if [ "${#hash}" -lt 64 ]; then
    echo "short txhash" >&2
    return 1
  fi
  local i=0
  while [ "$i" -lt 40 ]; do
    if "$TERPD" query tx "$hash" --node "$NODE" -o json > "$WORK/tx-query.json" 2>/dev/null; then
      return 0
    fi
    i=$((i + 1))
    sleep 2
  done
  echo "tx not indexed" >&2
  return 1
}

if [ ! -x "$LANE" ] || [ ! -x "$TERPD" ] || [ ! -s "$WASM" ] || [ ! -s "$WAVS" ]; then
  echo "lane, terpd, wasm, or wavs missing" >&2
  exit 1
fi

dd if=/dev/urandom of="$WORK/custody.seed" bs=32 count=1 status=none
chmod 600 "$WORK/custody.seed"
dd if=/dev/urandom of="$WORK/op.seed" bs=32 count=1 status=none
chmod 600 "$WORK/op.seed"
python3 "$WT/tools/hash-market/scripts/headstash-operator-sign.py" --key "$WORK/op.seed" --pubkey-only > "$WORK/op.pub"
HEADSTASH_OPERATOR=$(python3 "$WT/tools/hash-market/scripts/headstash-operator-sign.py" --key "$WORK/op.seed" --address-only)
export HEADSTASH_OPERATOR
case "$HEADSTASH_OPERATOR" in
  terp1*) ;;
  *) echo "operator missing" >&2; exit 1 ;;
esac
chmod 600 "$WORK/op.pub"
printf '%s' '{"height":1}' > "$WORK/seal.json"

for i in 0 1 2 3; do
  start_lane "$i" 0
  wait_lane "$i"
done
PUB=$(cat "$WORK/lane0/custody.pub")
if [ "${#PUB}" != "64" ]; then
  echo "pubkey missing" >&2
  exit 1
fi

export HEADSTASH_EXTERNAL=1
export HEADSTASH_CUSTODY_PUB_HEX="$PUB"
export HEADSTASH_CHAIN_FILE="$CHAIN_FILE"
export HEADSTASH_STOP_FILE="$STOP"
export HASHMERCHANT_SIDECAR_URLS="http://host.docker.internal:18521,http://host.docker.internal:18522,http://host.docker.internal:18523,http://host.docker.internal:18524"

(
  cd "$ICT"
  cargo run -q -p ict-rs --example headstash_failure_modes --features "docker,testing,terp"
) > "$WORK/chain.log" 2>&1 &
chain_pid=$!

for _ in $(seq 1 120); do
  if [ -s "$CHAIN_FILE" ]; then
    break
  fi
  if ! kill -0 "$chain_pid" 2>/dev/null; then
    echo "chain process exited" >&2
    tail -50 "$WORK/chain.log" >&2 || true
    exit 1
  fi
  sleep 2
done
if [ ! -s "$CHAIN_FILE" ]; then
  echo "chain file missing" >&2
  tail -50 "$WORK/chain.log" >&2 || true
  exit 1
fi
# shellcheck disable=SC1090
source "$CHAIN_FILE"
NODE="tcp://127.0.0.1:${rpc}"
RPC="$rpc"
case "$NODE" in
  tcp://127.0.0.1:[0-9]*) ;;
  *) echo "refusing node" >&2; exit 1 ;;
esac

HOME_CHAIN="$WORK/home"
rm -rf "$HOME_CHAIN"
mkdir -p "$HOME_CHAIN"
chmod 700 "$HOME_CHAIN"
docker cp "${container}:${home}/keyring-test" "$HOME_CHAIN/keyring-test"
chmod -R go-rwx "$HOME_CHAIN" || true

OWNER=$("$TERPD" keys show validator -a --keyring-backend test --home "$HOME_CHAIN")
case "$OWNER" in
  terp1*) ;;
  *) echo "owner missing" >&2; exit 1 ;;
esac

HEADSTASH_RECIPIENT="$OWNER" HEADSTASH_PROVE_OUT="$WORK/claim.json" "$LANE" --prove > "$WORK/prove.log"

python3 - "$WORK/claim.json" "$WORK" "$PUB" << 'PY'
import base64, json, pathlib, sys
claim_path, work, pub = sys.argv[1:]
work = pathlib.Path(work)
claim = json.loads(pathlib.Path(claim_path).read_text())
proof = claim["proof_hex"]
if claim["vk_id"] != 1 or claim["amount"] != 15:
    raise SystemExit("claim shape")
if claim["ctx_hex"] != "07" * 32:
    raise SystemExit("ctx")
if claim["denom_hex"] != "1d56a5879c78333d517942a36e7ea0932e7ffa1a1027e439279e8517580ea706":
    raise SystemExit("denom")
if claim["destination"].count(":") or not claim["destination"].startswith("terp1"):
    raise SystemExit("destination")
vk_b64 = pathlib.Path(claim_path).with_suffix(".vk").read_text().strip()
raw = base64.b64decode(vk_b64, validate=True)
if len(raw) != 32:
    raise SystemExit("vk")
bad = bytearray(raw)
bad[0] ^= 0x5A
common = {
    "backend": "halo2",
    "ctx_hex": claim["ctx_hex"],
    "allocation_root_hex": claim["root_hex"],
    "bech32_hrp": "terp",
    "denom_hex": claim["denom_hex"],
    "proof_len": len(proof) // 2,
    "cutoff_height": 1,
    "amounts": [{"amount": 15, "cap": 1}],
    "trusted_hex": [pub],
    "trusted_verify_threshold": 1,
    "trusted_sign_threshold": 1,
    "max_rows": 4,
    "reserved_hex": [],
    "vk_id": 1,
}
bad_body = dict(common)
bad_body["vk_hash_hex"] = bad.hex()
good = dict(common)
good["vk_hash_hex"] = raw.hex()
claim_post = {
    "claims": [{
        "proof_hex": proof,
        "message": claim["message"],
        "denom_hex": claim["denom_hex"],
        "vk_id": 1,
    }]
}
(work / "sidecar-bad.json").write_text(json.dumps(bad_body))
(work / "sidecar-campaign.json").write_text(json.dumps(good))
(work / "claim-post.json").write_text(json.dumps(claim_post))
(work / "public.env").write_text(
    "\n".join([
        "ALLOCATION_ROOT_HEX=" + claim["root_hex"],
        "DENOM_HEX=" + claim["denom_hex"],
        "CTX_HEX=" + claim["ctx_hex"],
        "NULLIFIER_HEX=" + claim["nullifier_hex"],
        "DEST=" + claim["destination"],
    ]) + "\n"
)
(work / "vk.b64").write_text(vk_b64)
PY
chmod 600 "$WORK"/sidecar-bad.json "$WORK"/sidecar-campaign.json "$WORK"/claim-post.json "$WORK"/vk.b64 "$WORK"/public.env "$WORK"/claim.json

t0=$(now_ms)
bad_code=$(post_json 18521 /ve/headstash/campaign "$WORK/sidecar-bad.json" "$WORK/bad-campaign.json")
claim_code=$(post_json 18521 /ve/headstash/claims "$WORK/claim-post.json" "$WORK/bad-claim.json")
bad_err=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("error",""))' "$WORK/bad-campaign.json")
claim_acc=$(python3 -c 'import json,sys
try:
    b=json.load(open(sys.argv[1]))
except Exception:
    print("rejected"); raise SystemExit
print("accepted" if b.get("accepted") else "rejected")' "$WORK/bad-claim.json")
ms=$(( $(now_ms) - t0 ))
if [ "$bad_code" = "400" ] && [ "$bad_err" = "vk hash" ] && [ "$claim_acc" = "rejected" ]; then
  report mismatched_vk_hash_rejected 1 "$ms" "campaign 400 vk hash; claim http ${claim_code}"
else
  report mismatched_vk_hash_rejected 0 "$ms" "campaign ${bad_code} ${bad_err} claim ${claim_code} ${claim_acc}"
fi

# shellcheck disable=SC1090
source "$WORK/public.env"
export NODE CHAIN_ID HOME_CHAIN WAVS ALLOCATION_ROOT_HEX DENOM_HEX CTX_HEX
export KEY=validator
export TRUSTED_HEX="$PUB"
export CUTOFF=1
export AMOUNT=15
export WASM="${WASM}"
export SWEEP_RECIPIENT="$OWNER"
VK_HASH_B64=$(cat "$WORK/vk.b64")
export VK_HASH_B64 WORK
bash "$WT/tools/hash-market/scripts/headstash-halo2-campaign.sh" | tee "$WORK/campaign.out"
CONTRACT=$(awk '/^contract /{print $2}' "$WORK/campaign.out")
case "$CONTRACT" in
  terp1*) ;;
  *) echo "contract missing" >&2; exit 1 ;;
esac

python3 - "$WORK/public.env" "$WORK/stage.json" "$WORK/replay.json" << 'PY'
import base64, json, pathlib, sys
env_path, stage_path, replay_path = sys.argv[1:]
env = {}
for line in pathlib.Path(env_path).read_text().splitlines():
    if not line.strip():
        continue
    k, v = line.split("=", 1)
    env[k] = v
def b64hex(name):
    return base64.b64encode(bytes.fromhex(env[name])).decode()
row = {
    "nullifier": b64hex("NULLIFIER_HEX"),
    "denom": b64hex("DENOM_HEX"),
    "destination": env["DEST"],
    "amount": 15,
}
stage = {
    "stage_epoch_batch": {
        "digests": [base64.b64encode(bytes(32)).decode()],
        "rows": [row],
    }
}
replay = {
    "apply_accepted_batch": {
        "allocation_root": b64hex("ALLOCATION_ROOT_HEX"),
        "root_id": 0,
        "rows": [row],
    }
}
pathlib.Path(stage_path).write_text(json.dumps(stage, separators=(",", ":")))
pathlib.Path(replay_path).write_text(json.dumps(replay, separators=(",", ":")))
PY
STAGE_MSG=$(cat "$WORK/stage.json")
tx_wait 5000000 tx wasm execute "$CONTRACT" "$STAGE_MSG"

operator_routes() {
  local t0 code ok
  t0=$(now_ms)
  code=$(post_json 18521 /ve/headstash/campaign "$WORK/sidecar-campaign.json" "$WORK/noauth-camp.json" public)
  ok=0
  if [ "$code" = "401" ] || [ "$code" = "403" ]; then ok=1; fi
  report unauthenticated_campaign_refused "$ok" "$(( $(now_ms) - t0 ))" "http ${code}"
  t0=$(now_ms)
  code=$(post_json 18521 /ve/headstash/campaign "$WORK/sidecar-campaign.json" "$WORK/auth-camp.json" operator)
  ok=0
  if [ "$code" = "200" ]; then ok=1; fi
  report operator_campaign_accepted "$ok" "$(( $(now_ms) - t0 ))" "http ${code}"
  t0=$(now_ms)
  code=$(post_json 18521 /ve/headstash/claims "$WORK/claim-post.json" "$WORK/public-claim.json" public)
  ok=0
  if [ "$code" = "200" ] && python3 -c 'import json,sys; b=json.load(open(sys.argv[1])); raise SystemExit(0 if b.get("accepted")==[0] and not b.get("rejected") else 1)' "$WORK/public-claim.json"; then
    ok=1
  fi
  report public_claim_succeeds "$ok" "$(( $(now_ms) - t0 ))" "http ${code}"
  t0=$(now_ms)
  code=$(post_json 18521 /ve/headstash/seal "$WORK/seal.json" "$WORK/noauth-seal.json" public)
  ok=0
  if [ "$code" = "401" ] || [ "$code" = "403" ]; then ok=1; fi
  report unauthenticated_seal_refused "$ok" "$(( $(now_ms) - t0 ))" "http ${code}"
  t0=$(now_ms)
  code=$(post_json 18521 /ve/headstash/seal "$WORK/seal.json" "$WORK/auth-seal.json" operator)
  ok=0
  if [ "$code" = "200" ]; then ok=1; fi
  report operator_seal_accepted "$ok" "$(( $(now_ms) - t0 ))" "http ${code}"
}

stop_lane 3
operator_routes
t0=$(now_ms)
install_claim_on 1
install_claim_on 2
paid=0
count=""
bal=""
for _ in $(seq 1 40); do
  count=$(query_count || true)
  bal=$(contract_amount || echo 0)
  if [ "$count" = "3" ] && [ -n "$bal" ] && [ "$bal" != "1000000" ] && [ "$bal" != "0" ]; then
    paid=1
    break
  fi
  sleep 2
done
report one_sidecar_down_still_pays "$paid" "$(( $(now_ms) - t0 ))" "count=${count:-none} escrow=${bal:-none}"

bal_hold=$(contract_amount)
h0=$(chain_height)
t0=$(now_ms)
stop_lane 2
sleep 12
h1=$(chain_height)
count2=$(query_count || true)
bal2=$(contract_amount)
moved=0
if [ "$h1" -gt "$h0" ]; then moved=1; fi
ok=0
if [ "$moved" = "1" ] && [ "$count2" = "3" ] && [ "$bal2" = "$bal_hold" ]; then ok=1; fi
report two_sidecars_down_no_quorum "$ok" "$(( $(now_ms) - t0 ))" "count=${count2:-none} height_moved=${moved}"

t0=$(now_ms)
start_lane 2 0
start_lane 3 0
wait_lane 2
wait_lane 3
install_claim_on 2
install_claim_on 3
set +e
count4=$(wait_count 4 45)
rc=$?
set -e
ms=$(( $(now_ms) - t0 ))
if [ "$rc" = "0" ]; then
  report recovers_next_epoch 1 "$ms" "count=4"
  report baseline_four_up 1 "$ms" "count=4"
else
  report recovers_next_epoch 0 "$ms" "count=${count4}"
  report baseline_four_up 0 "$ms" "count=${count4}"
fi

t0=$(now_ms)
stop_lane 1
sleep 6
start_lane 1 0
wait_lane 1
install_claim_on 1
set +e
countr=$(wait_count 4 45)
rc=$?
set -e
ms=$(( $(now_ms) - t0 ))
if [ "$rc" = "0" ]; then
  report sidecar_restart_mid_epoch 1 "$ms" "count=4"
else
  report sidecar_restart_mid_epoch 0 "$ms" "count=${countr}"
fi

t0=$(now_ms)
stop_lane 3
start_lane 3 1
wait_lane 3
install_claim_on 3
sleep 12
countw=$(query_count || true)
ms=$(( $(now_ms) - t0 ))
ok=0
if [ "$countw" = "3" ]; then ok=1; fi
report wrong_root_outvoted "$ok" "$ms" "count=${countw:-none}"

t0=$(now_ms)
python3 - "$WORK/replay.json" "$WORK/vk.b64" "$WORK/payout-mismatch.json" << 'PY'
import base64, json, pathlib, sys
replay, vk_path, dest = sys.argv[1:]
body = json.loads(pathlib.Path(replay).read_text())
vk = bytearray(base64.b64decode(pathlib.Path(vk_path).read_text().strip()))
vk[0] ^= 0x5A
body["apply_accepted_batch"]["vk_hash"] = base64.b64encode(bytes(vk)).decode()
pathlib.Path(dest).write_text(json.dumps(body, separators=(",", ":")))
PY
held_pay=$(contract_amount)
MISMATCH_MSG=$(cat "$WORK/payout-mismatch.json")
set +e
tx_wait 5000000 tx wasm execute "$CONTRACT" "$MISMATCH_MSG"
set -e
ok=0
after_pay=$(contract_amount || echo missing)
if grep -q "vk hash" "$WORK/tx-query.json" 2>/dev/null && [ "$after_pay" = "$held_pay" ]; then ok=1; fi
report payout_vk_hash_rejected "$ok" "$(( $(now_ms) - t0 ))" "escrow stayed ${after_pay}"

t0=$(now_ms)
REPLAY_MSG=$(cat "$WORK/replay.json")
set +e
tx_wait 5000000 tx wasm execute "$CONTRACT" "$REPLAY_MSG"
set -e
ok=0
if grep -q "nullifier already exists" "$WORK/tx-query.json" 2>/dev/null; then ok=1; fi
report replayed_nullifier_across_epochs "$ok" "$(( $(now_ms) - t0 ))" "second spend rejected"

t0=$(now_ms)
held=$(contract_amount || echo missing)
height=$(chain_height || echo 0)
SWEEP_MSG='{"sweep_unclaimed":{}}'
set +e
tx_wait 5000000 tx wasm execute "$CONTRACT" "$SWEEP_MSG"
sweep_rc=$?
set -e
code=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d.get("code",""))' "$WORK/tx-query.json" 2>/dev/null || echo missing)
left=$(contract_amount || echo missing)
ok=0
if [ "$sweep_rc" = "0" ] && [ "$code" = "0" ] && [ "$left" = "0" ] && [ "$height" -gt 1 ]; then
  ok=1
fi
report unclaimed_funds "$ok" "$(( $(now_ms) - t0 ))" "height ${height} escrow ${held} -> ${left} code ${code}"

cp "$REPORT" /tmp/headstash-fm-halo2-report.txt
echo "report $REPORT"
