#!/usr/bin/env bash
# Vote yes on Foundation proposal-single from 3 members in parallel.
# Each mnemonic gets its own keyring-dir so sequences never collide.
#
# Env (scripts/release/.env, gitignored):
#   MAIN_MNEMONIC or DAO_MEMBER1_MNEMONIC
#   DAO_MEMBER2_MNEMONIC
#   DAO_MEMBER3_MNEMONIC
#
# Usage: vote_yes.sh [proposal_id ...]
# Default ids: 3 4 5 6

set -euo pipefail
# Never dump commands (mnemonics live in env).
set +x

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
ENV_FILE="${ENV_FILE:-$ROOT/scripts/release/.env}"
TERPD="${TERPD:-/Users/returniflost/go/bin/terpd}"
PROPOSAL_A="${PROPOSAL_A:-terp1p5582k3sw008jkmw0f48ucszn660nazqr9x3ut6h5aqj4frq8f0shaey4g}"

# Live cw4 members. Wrong mnemonic → abort that voter.
ALLOW='terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052
terp1fpdz820lcfze3alx2gukhp7yrnhshe4dv96wgj
terp14eygcfdmwpl00nw2amqpujg73nyy59azqu55em'

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

CHAIN_ID="${CHAIN_ID:-morocco-1}"
RPC_URL="${RPC_URL:-http://192.168.10.101:26657}"

unquote() {
  local s="${1-}"
  s="${s%\"}"
  s="${s#\"}"
  s="${s%\'}"
  s="${s#\'}"
  printf '%s' "$s"
}

placeholder() {
  local s="$1"
  [[ -z "$s" ]] && return 0
  [[ "$s" == *"word1 word2"* ]] && return 0
  [[ "$s" == *"…"* || "$s" == *"..."* ]] && return 0
  return 1
}

MEMBER1="$(unquote "${DAO_MEMBER1_MNEMONIC:-${MAIN_MNEMONIC:-}}")"
MEMBER2="$(unquote "${DAO_MEMBER2_MNEMONIC:-}")"
MEMBER3="$(unquote "${DAO_MEMBER3_MNEMONIC:-}")"

IDS=("${@:-3 4 5 6}")
if [[ $# -eq 0 ]]; then
  IDS=(3 4 5 6)
fi

vote_wallet() {
  local label="$1"
  local mnemonic="$2"
  shift 2
  local ids=("$@")

  if placeholder "$mnemonic"; then
    echo "[$label] skip: mnemonic unset or still a placeholder" >&2
    return 1
  fi

  local kr
  kr="$(mktemp -d "${TMPDIR:-/tmp}/fiab-vote.XXXXXX")"
  trap 'rm -rf "$kr"' RETURN

  if ! printf '%s\n' "$mnemonic" | "$TERPD" keys add voter --recover \
    --keyring-backend test --keyring-dir "$kr" --output json >/dev/null 2>&1; then
    echo "[$label] key recover failed" >&2
    return 1
  fi

  local addr
  addr="$("$TERPD" keys show voter -a --keyring-backend test --keyring-dir "$kr")"
  if ! grep -qx "$addr" <<<"$ALLOW"; then
    echo "[$label] $addr is not a Foundation cw4 member — not voting" >&2
    return 1
  fi
  echo "[$label] $addr"

  json_field() {
    python3 -c 'import json,sys; s=sys.stdin.read(); i=s.find("{");
print("" if i<0 else json.loads(s[i:]).get(sys.argv[1],""))' "$1"
  }

  local id hash code
  for id in "${ids[@]}"; do
    local msg
    msg=$(printf '{"vote":{"proposal_id":%s,"vote":"yes"}}' "$id")
    local out
    out="$("$TERPD" tx wasm execute "$PROPOSAL_A" "$msg" \
      --from voter --keyring-backend test --keyring-dir "$kr" \
      --chain-id "$CHAIN_ID" --node "$RPC_URL" \
      --gas auto --gas-adjustment 1.6 --gas-prices 0.05uthiol \
      --broadcast-mode sync --yes --output json 2>&1)" || true
    hash="$(json_field txhash <<<"$out")"
    if [[ -z "$hash" ]]; then
      echo "[$label] prop $id submit failed: ${out:0:350}" >&2
      sleep 3
      continue
    fi
    code=""
    for _ in $(seq 1 30); do
      local tx
      tx="$("$TERPD" q tx "$hash" --node "$RPC_URL" -o json 2>/dev/null)" || { sleep 2; continue; }
      code="$(json_field code <<<"$tx")"
      break
    done
    echo "[$label] prop $id $hash code=${code:-pending}"
    # same-account sequence: do not sign the next vote until this one is in a block
    if [[ "$code" != "0" ]]; then
      echo "[$label] stop this wallet (need inclusion before next sequence)" >&2
      return 1
    fi
  done
}

echo "env: $ENV_FILE"
echo "ids: ${IDS[*]}"
echo "node: $RPC_URL"

fail=0
vote_wallet member1 "$MEMBER1" "${IDS[@]}" &
p1=$!
vote_wallet member2 "$MEMBER2" "${IDS[@]}" &
p2=$!
vote_wallet member3 "$MEMBER3" "${IDS[@]}" &
p3=$!
for p in $p1 $p2 $p3; do
  if ! wait "$p"; then
    fail=1
  fi
done
exit "$fail"
