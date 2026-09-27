#!/usr/bin/env bash
# Verify feegrant allowances for members; optional sponsored smoke tx.
#
# Usage:
#   verify.sh --file members.json [--granter KEY|ADDR] [--dry-run]
#   verify.sh --file members.json --granter ops --smoke [--from KEY]
#
# Smoke (when not dry-run): each grantee sends a tiny self bank MsgSend with
#   --fee-granter <granter> so gas is paid by the allowance.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib.sh
source "${SCRIPT_DIR}/../lib.sh"

MEMBERS_FILE=""
GRANTER=""
DRY_RUN=false
SMOKE=false
FROM_KEY=""
SMOKE_AMOUNT="${HM_FEEGRANT_SMOKE_AMOUNT:-1uterp}"

usage() {
  cat <<'EOF'
verify.sh — query feegrant allowances; optional --fee-granter smoke

Required:
  --file PATH         members.json (array or {members:[...]})

Options:
  --granter KEY|ADDR  Prefer pair query granter→grantee; also fee-granter for smoke
  --smoke             Broadcast a tiny bank send as grantee with --fee-granter
  --from KEY          Key name for smoke when grantee has a keyring entry
  --amount COIN       Smoke amount (default: 1uterp / HM_FEEGRANT_SMOKE_AMOUNT)
  --dry-run           Print plan / skip chain (no terpd required)
  -h, --help

Env: BIND, HM_HOME, HM_NODE, HM_CHAIN_ID, HM_KEYRING, HM_MAINNET_OK, HM_FEES
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --file) MEMBERS_FILE="${2:-}"; shift 2 ;;
    --granter) GRANTER="${2:-}"; shift 2 ;;
    --smoke) SMOKE=true; shift ;;
    --from) FROM_KEY="${2:-}"; shift 2 ;;
    --amount) SMOKE_AMOUNT="${2:-}"; shift 2 ;;
    --dry-run) DRY_RUN=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) hm_die "unknown arg: $1" ;;
  esac
done

[[ -n "$MEMBERS_FILE" ]] || hm_die "--file required"
[[ -f "$MEMBERS_FILE" ]] || hm_die "members file not found: $MEMBERS_FILE"
hm_need_cmds jq

if [[ "$DRY_RUN" != true ]]; then
  hm_require_bind
  hm_guard_mainnet
fi

if [[ "$SMOKE" == true && -z "$GRANTER" && "$DRY_RUN" != true ]]; then
  hm_die "--smoke requires --granter"
fi

GRANTER_ADDR=""
if [[ -n "$GRANTER" && "$DRY_RUN" != true ]]; then
  GRANTER_ADDR="$(hm_resolve_addr "$GRANTER")"
  hm_info "granter=${GRANTER} addr=${GRANTER_ADDR}"
elif [[ -n "$GRANTER" ]]; then
  GRANTER_ADDR="$GRANTER"
  hm_info "dry-run granter=${GRANTER}"
fi

count=0
ok=0
while IFS= read -r line; do
  [[ -n "$line" ]] || continue
  name="$(echo "$line" | jq -r '.name // .key // empty')"
  addr="$(echo "$line" | jq -r '.address // .addr // empty')"
  key="$(echo "$line" | jq -r '.key // .name // empty')"
  [[ -n "$addr" ]] || continue
  count=$((count + 1))
  label="${name:-$addr}"

  if [[ "$DRY_RUN" == true ]]; then
    if [[ -n "$GRANTER_ADDR" ]]; then
      hm_info "dry-run: q feegrant grant $GRANTER_ADDR $addr"
    else
      hm_info "dry-run: q feegrant grants-by-grantee $addr"
    fi
    if [[ "$SMOKE" == true ]]; then
      hm_info "dry-run: tx bank send $addr $addr $SMOKE_AMOUNT --fee-granter ${GRANTER_ADDR:-GRANTER}"
    fi
    ok=$((ok + 1))
    hm_ok "planned $label"
    continue
  fi

  # Query
  q_ok=false
  if [[ -n "$GRANTER_ADDR" ]]; then
    if resp="$(hm_bind q feegrant grant "$GRANTER_ADDR" "$addr" --output json 2>/dev/null)"; then
      # accept either allowance wrapper shapes
      if echo "$resp" | jq -e '.allowance // .grant // .' >/dev/null 2>&1; then
        hm_ok "allowance granter→$label"
        q_ok=true
        echo "$resp" | jq -c '{granter:(.allowance.granter // .grant.granter // .granter // empty), grantee:(.allowance.grantee // .grant.grantee // .grantee // empty), type:(.allowance.allowance."@type" // .allowance.allowance.type_url // .grant.allowance."@type" // "present")}' 2>/dev/null \
          | while read -r j; do hm_info "$j"; done || true
      fi
    fi
  fi
  if [[ "$q_ok" != true ]]; then
    if resp="$(hm_bind q feegrant grants-by-grantee "$addr" --output json 2>/dev/null)" \
      || resp="$(hm_bind q feegrant grants "$addr" --output json 2>/dev/null)"; then
      n="$(echo "$resp" | jq '(.allowances // .grants // []) | length' 2>/dev/null || echo 0)"
      if [[ "${n:-0}" =~ ^[0-9]+$ ]] && (( n > 0 )); then
        hm_ok "grantee $label has $n allowance(s)"
        q_ok=true
      else
        hm_fail "no allowances for $label ($addr)"
      fi
    else
      hm_fail "query failed for $label ($addr)"
    fi
  fi

  [[ "$q_ok" == true ]] || continue

  if [[ "$SMOKE" != true ]]; then
    ok=$((ok + 1))
    continue
  fi

  # Smoke: grantee-initiated bank send self with fee-granter
  # Prefer keyring key matching member; else --from override for all.
  from="${FROM_KEY:-}"
  if [[ -z "$from" && -n "$key" ]]; then
    if hm_bind keys show "$key" -a >/dev/null 2>&1; then
      from="$key"
    fi
  fi
  if [[ -z "$from" ]]; then
    hm_warn "smoke skip $label: no keyring key (set --from or member.key)"
    ok=$((ok + 1))
    continue
  fi

  # shellcheck disable=SC2046
  if hm_bind tx bank send "$from" "$addr" "$SMOKE_AMOUNT" \
      $(hm_tx_flags) --from "$from" --fee-granter "$GRANTER_ADDR" 2>&1; then
    hm_ok "smoke --fee-granter $label"
    ok=$((ok + 1))
  else
    hm_fail "smoke failed $label"
  fi
done < <(jq -c '(if type=="array" then . else .members end)[]' "$MEMBERS_FILE")

if (( count == 0 )); then
  hm_die "no members with address in $MEMBERS_FILE"
fi

hm_log "verify: ${ok}/${count} ok"
hm_summary
