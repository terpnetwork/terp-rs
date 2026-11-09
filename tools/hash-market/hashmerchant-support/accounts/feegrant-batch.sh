#!/usr/bin/env bash
# Batch feegrant grants for demo/default accounts.
# Usage:
#   feegrant-batch.sh --granter <key> --file members.json [--limit 1000000uterp] [--expiry-days 30]
#   feegrant-batch.sh --dry-run --file members.json
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib.sh
source "${SCRIPT_DIR}/../lib.sh"

GRANTER=""
MEMBERS_FILE=""
LIMIT="${HM_FEEGRANT_LIMIT:-1000000uterp}"
EXPIRY_DAYS="${HM_FEEGRANT_EXPIRY_DAYS:-30}"
DRY_RUN=false
ALLOWED_MSGS_DEFAULT="/cosmwasm.wasm.v1.MsgExecuteContract,/cosmwasm.wasm.v1.MsgInstantiateContract,/cosmos.bank.v1beta1.MsgSend"
ALLOWED_MSGS="${HM_FEEGRANT_MSGS:-$ALLOWED_MSGS_DEFAULT}"
BASIC_ONLY=false

usage() {
  cat <<'EOF'
feegrant-batch.sh — grant fee allowances to members

Required:
  --granter KEY       Key name of fee granter (ops/treasury)
  --file PATH         members.json (array or {members:[...]})

Options:
  --limit COIN        Spend limit (default: 1000000uterp or HM_FEEGRANT_LIMIT)
  --expiry-days N     Allowance lifetime (default: 30; 0 = no expiry)
  --allowed-msgs CSV  Allowed msg type URLs (default: wasm execute/instantiate + bank send)
  --basic             Use BasicAllowance only (no msg allowlist)
  --dry-run           Print actions without broadcasting
  -h, --help          Show help

Env: BIND, HM_HOME, HM_NODE, HM_CHAIN_ID, HM_KEYRING, HM_MAINNET_OK
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --granter) GRANTER="${2:-}"; shift 2 ;;
    --file) MEMBERS_FILE="${2:-}"; shift 2 ;;
    --limit) LIMIT="${2:-}"; shift 2 ;;
    --expiry-days) EXPIRY_DAYS="${2:-}"; shift 2 ;;
    --allowed-msgs) ALLOWED_MSGS="${2:-}"; shift 2 ;;
    --basic) BASIC_ONLY=true; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) hm_die "unknown arg: $1" ;;
  esac
done

[[ -n "$MEMBERS_FILE" ]] || hm_die "--file required"
[[ -f "$MEMBERS_FILE" ]] || hm_die "members file not found: $MEMBERS_FILE"
if [[ "$DRY_RUN" != true ]]; then
  [[ -n "$GRANTER" ]] || hm_die "--granter required (unless --dry-run)"
  hm_require_bind
  hm_guard_mainnet
fi

hm_need_cmds jq

EXPIRY_FLAG=()
if [[ "$EXPIRY_DAYS" != "0" ]]; then
  # RFC3339 UTC
  if date -u -v+"${EXPIRY_DAYS}"d +%Y-%m-%dT%H:%M:%SZ >/dev/null 2>&1; then
    EXPIRY="$(date -u -v+"${EXPIRY_DAYS}"d +%Y-%m-%dT%H:%M:%SZ)"
  else
    EXPIRY="$(date -u -d "+${EXPIRY_DAYS} days" +%Y-%m-%dT%H:%M:%SZ)"
  fi
  EXPIRY_FLAG=(--expiration "$EXPIRY")
  hm_info "expiration=${EXPIRY}"
fi

GRANTER_ADDR=""
if [[ "$DRY_RUN" != true ]]; then
  GRANTER_ADDR="$(hm_resolve_addr "$GRANTER")"
  hm_info "granter=${GRANTER} addr=${GRANTER_ADDR}"
fi

count=0
ok=0
while IFS= read -r addr; do
  [[ -n "$addr" ]] || continue
  count=$((count + 1))
  hm_log "[$count] grantee=$addr limit=$LIMIT"

  if [[ "$DRY_RUN" == true ]]; then
    if [[ "$BASIC_ONLY" == true ]]; then
      hm_info "dry-run: feegrant grant $GRANTER $addr --spend-limit $LIMIT ${EXPIRY_FLAG[*]:-}"
    else
      hm_info "dry-run: feegrant grant $GRANTER $addr --spend-limit $LIMIT --allowed-messages $ALLOWED_MSGS ${EXPIRY_FLAG[*]:-}"
    fi
    ok=$((ok + 1))
    continue
  fi

  # Cosmos SDK feegrant CLI:
  #   terpd tx feegrant grant [granter_key_or_addr] [grantee] [flags]
  grant_args=(tx feegrant grant "$GRANTER" "$addr" --spend-limit "$LIMIT" "${EXPIRY_FLAG[@]}")
  if [[ "$BASIC_ONLY" != true && -n "$ALLOWED_MSGS" ]]; then
    # Multiple --allowed-messages flags or comma-separated depending on SDK version;
    # pass as repeated flags for maximum compatibility.
    IFS=',' read -ra msgs <<< "$ALLOWED_MSGS"
    for m in "${msgs[@]}"; do
      m="$(echo "$m" | xargs)"
      [[ -n "$m" ]] || continue
      grant_args+=(--allowed-messages "$m")
    done
  fi
  # shellcheck disable=SC2046
  if hm_bind "${grant_args[@]}" $(hm_tx_flags) --from "$GRANTER" 2>&1; then
    hm_ok "granted $addr"
    ok=$((ok + 1))
  else
    hm_fail "grant failed for $addr"
  fi
done < <(hm_members_addresses "$MEMBERS_FILE")

if (( count == 0 )); then
  # also accept keys-only members: resolve later
  while IFS= read -r key; do
    [[ -n "$key" ]] || continue
    count=$((count + 1))
    if [[ "$DRY_RUN" == true ]]; then
      hm_info "dry-run: resolve key=$key then grant"
      ok=$((ok + 1))
      continue
    fi
    addr="$(hm_resolve_addr "$key")"
    grant_args=(tx feegrant grant "$GRANTER" "$addr" --spend-limit "$LIMIT" "${EXPIRY_FLAG[@]}")
    if [[ "$BASIC_ONLY" != true && -n "$ALLOWED_MSGS" ]]; then
      IFS=',' read -ra msgs <<< "$ALLOWED_MSGS"
      for m in "${msgs[@]}"; do
        m="$(echo "$m" | xargs)"
        [[ -n "$m" ]] || continue
        grant_args+=(--allowed-messages "$m")
      done
    fi
    # shellcheck disable=SC2046
    if hm_bind "${grant_args[@]}" $(hm_tx_flags) --from "$GRANTER" 2>&1; then
      hm_ok "granted $key ($addr)"
      ok=$((ok + 1))
    else
      hm_fail "grant failed for $key"
    fi
  done < <(jq -r '(if type=="array" then . else .members end)[] | select(.address==null and .addr==null) | (.key // .name // empty)' "$MEMBERS_FILE")
fi

if (( count == 0 )); then
  hm_die "no members with address or key found in $MEMBERS_FILE"
fi
hm_log "feegrant-batch: ${ok}/${count} ok"
if (( ok != count )); then
  exit 1
fi
exit 0
