#!/usr/bin/env bash
# Shared helpers for hashmerchant support tooling and (optionally) e2e.
# shellcheck disable=SC2034
set -euo pipefail

HM_SUPPORT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export HM_SUPPORT_ROOT

# Defaults (override via env)
: "${BIND:=terpd}"
: "${HM_CHAIN_ID:=}"
: "${HM_HOME:=}"
: "${HM_NODE:=}"
: "${HM_KEYRING:=test}"
: "${HM_DENOM:=uterp}"
: "${HM_FEES:=400000uterp}"
: "${HM_GAS:=auto}"
: "${HM_GAS_ADJUSTMENT:=1.3}"
: "${HM_YES:=false}"
: "${HM_MAINNET_OK:=false}"

PASS=0
FAIL=0
WARN=0

hm_log()  { printf '%s\n' "$*"; }
hm_info() { printf '  · %s\n' "$*"; }
hm_ok()   { printf '  PASS: %s\n' "$*"; PASS=$((PASS + 1)); }
hm_fail() { printf '  FAIL: %s\n' "$*" >&2; FAIL=$((FAIL + 1)); }
hm_warn() { printf '  WARN: %s\n' "$*" >&2; WARN=$((WARN + 1)); }
hm_die()  { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

hm_need_cmds() {
  local c
  for c in "$@"; do
    command -v "$c" >/dev/null 2>&1 || hm_die "required command not found: $c"
  done
}

hm_bind_flags() {
  local flags=()
  [[ -n "${HM_HOME}" ]] && flags+=(--home "$HM_HOME")
  [[ -n "${HM_NODE}" ]] && flags+=(--node "$HM_NODE")
  [[ -n "${HM_CHAIN_ID}" ]] && flags+=(--chain-id "$HM_CHAIN_ID")
  flags+=(--keyring-backend "$HM_KEYRING")
  printf '%s\n' "${flags[@]}"
}

# Run terpd with standard home/node/chain flags when set.
hm_bind() {
  local flags=()
  # shellcheck disable=SC2207
  flags=($(hm_bind_flags))
  "$BIND" "${flags[@]}" "$@"
}

hm_tx_flags() {
  printf '%s\n' --gas "$HM_GAS" --gas-adjustment "$HM_GAS_ADJUSTMENT" --fees "$HM_FEES" -y
}

hm_guard_mainnet() {
  case "${HM_CHAIN_ID}" in
    morocco-1|mainnet|terp-mainnet)
      if [[ "${HM_MAINNET_OK}" != "true" ]]; then
        hm_die "refusing mainnet-like chain-id=${HM_CHAIN_ID}; set HM_MAINNET_OK=true to override"
      fi
      ;;
  esac
}

hm_require_bind() {
  command -v "$BIND" >/dev/null 2>&1 || hm_die "$BIND not in PATH (install via terp-installer or build terpd)"
}

hm_wait_for_height() {
  local min_height="${1:-2}"
  local attempts="${2:-30}"
  local i height
  hm_require_bind
  for i in $(seq 1 "$attempts"); do
    height="$(hm_bind status 2>/dev/null | jq -r '.sync_info.latest_block_height // .SyncInfo.latest_block_height // empty' 2>/dev/null || true)"
    if [[ -n "$height" ]] && [[ "$height" =~ ^[0-9]+$ ]] && (( height >= min_height )); then
      hm_info "height=${height} (>= ${min_height})"
      return 0
    fi
    sleep 1
  done
  return 1
}

hm_wait_for_root() {
  local chain_uid="${1:?chain_uid}"
  local algo="${2:-keccak256}"
  local attempts="${3:-45}"
  local i resp root
  hm_require_bind
  for i in $(seq 1 "$attempts"); do
    resp="$(hm_bind q hashmerchant root "$chain_uid" "$algo" --output json 2>/dev/null || echo '{}')"
    root="$(echo "$resp" | jq -r '.root.root // empty' 2>/dev/null || true)"
    if [[ -n "$root" && "$root" != "null" ]]; then
      printf '%s\n' "$root"
      return 0
    fi
    sleep 2
  done
  return 1
}

hm_curl_json() {
  local url="${1:?url}"
  curl -sf --max-time "${2:-5}" "$url"
}

hm_sidecar_health() {
  local base="${1:?sidecar base url}"
  hm_curl_json "${base%/}/health" >/dev/null
}

hm_sidecar_vote_extension() {
  local base="${1:?sidecar base url}"
  hm_curl_json "${base%/}/vote-extension"
}

hm_resolve_addr() {
  local key_or_addr="${1:?}"
  if [[ "$key_or_addr" == terp1* ]] || [[ "$key_or_addr" == cosmos1* ]]; then
    printf '%s\n' "$key_or_addr"
    return 0
  fi
  hm_require_bind
  hm_bind keys show "$key_or_addr" -a
}

hm_summary() {
  printf '\n──────────────────────────\n'
  printf 'Results: %s passed, %s failed, %s warnings\n' "$PASS" "$FAIL" "$WARN"
  printf '──────────────────────────\n'
  (( FAIL == 0 ))
}

# JSON members file helpers: array of { "address"|"key", "name"? }
hm_members_count() {
  local file="${1:?}"
  jq 'if type=="array" then length else (.members|length) end' "$file"
}

hm_members_addresses() {
  local file="${1:?}"
  jq -r '
    (if type=="array" then . else .members end)
    | .[]
    | (.address // .addr // empty)
  ' "$file"
}
