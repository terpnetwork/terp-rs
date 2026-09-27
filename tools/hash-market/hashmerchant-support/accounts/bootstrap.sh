#!/usr/bin/env bash
# Bootstrap default demo accounts from a members template.
# - Creates missing keyring keys (test backend)
# - Writes resolved members-out.json with addresses
# - Optionally funds dust from a funder key
# - Optionally runs feegrant-batch
#
# Usage:
#   bootstrap.sh --file members.json --out members.resolved.json
#   bootstrap.sh --file members.json --out out.json --funder <key> --dust 1000uterp --feegrant --granter <key>
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib.sh
source "${SCRIPT_DIR}/../lib.sh"

MEMBERS_FILE=""
OUT_FILE=""
FUNDER=""
DUST=""
DO_FEEGRANT=false
GRANTER=""
DRY_RUN=false

usage() {
  cat <<'EOF'
bootstrap.sh — create keys + resolve members for demos

  --file PATH         Input members.json
  --out PATH          Output resolved members.json (addresses filled)
  --funder KEY        Optional: send dust from this key
  --dust COIN         Amount per member (default: 1000uterp)
  --feegrant          Also run feegrant-batch
  --granter KEY       Required if --feegrant
  --dry-run           Print plan only
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --file) MEMBERS_FILE="${2:-}"; shift 2 ;;
    --out) OUT_FILE="${2:-}"; shift 2 ;;
    --funder) FUNDER="${2:-}"; shift 2 ;;
    --dust) DUST="${2:-}"; shift 2 ;;
    --feegrant) DO_FEEGRANT=true; shift ;;
    --granter) GRANTER="${2:-}"; shift 2 ;;
    --dry-run) DRY_RUN=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) hm_die "unknown arg: $1" ;;
  esac
done

[[ -n "$MEMBERS_FILE" && -f "$MEMBERS_FILE" ]] || hm_die "--file required"
[[ -n "$OUT_FILE" ]] || hm_die "--out required"
: "${DUST:=1000uterp}"

hm_need_cmds jq

if [[ "$DRY_RUN" != true ]]; then
  hm_require_bind
  hm_guard_mainnet
fi

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

# Normalize to array
jq '(if type=="array" then . else .members end)' "$MEMBERS_FILE" > "$tmp.arr"
mv "$tmp.arr" "$tmp"

resolved='[]'
n="$(jq 'length' "$tmp")"
hm_info "members=${n}"

for i in $(seq 0 $((n - 1))); do
  row="$(jq -c ".[$i]" "$tmp")"
  name="$(echo "$row" | jq -r --argjson i "$i" '.name // .key // ("member-" + ($i|tostring))')"
  addr="$(echo "$row" | jq -r '.address // .addr // empty')"
  key="$(echo "$row" | jq -r '.key // .name // empty')"

  if [[ -z "$addr" ]]; then
    key="${key:-member-$i}"
    if [[ "$DRY_RUN" == true ]]; then
      hm_info "dry-run: keys add $key"
      addr="terp1dryrun${i}xxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    else
      if ! hm_bind keys show "$key" -a >/dev/null 2>&1; then
        hm_info "creating key $key"
        # non-interactive test keyring
        printf 'y\n' | hm_bind keys add "$key" --output json >/tmp/hm-key-"$key".json 2>&1 || \
          hm_bind keys add "$key" --output json >/tmp/hm-key-"$key".json
      fi
      addr="$(hm_bind keys show "$key" -a)"
    fi
  fi

  hm_info "member name=$name key=${key:-} addr=$addr"
  resolved="$(jq -c --arg name "$name" --arg key "${key:-}" --arg addr "$addr" \
    '. + [{name:$name, key:(if $key=="" then null else $key end), address:$addr}]' \
    <<<"$resolved")"

  if [[ -n "$FUNDER" && "$DRY_RUN" != true ]]; then
    # shellcheck disable=SC2046
    hm_bind tx bank send "$FUNDER" "$addr" "$DUST" $(hm_tx_flags) --from "$FUNDER" >/dev/null \
      && hm_ok "funded $addr $DUST" \
      || hm_warn "fund failed $addr"
  elif [[ -n "$FUNDER" && "$DRY_RUN" == true ]]; then
    hm_info "dry-run: bank send $FUNDER -> $addr $DUST"
  fi
done

jq -n --argjson members "$resolved" '{version:"loyalty-members/v1", members:$members}' > "$OUT_FILE"
hm_ok "wrote $OUT_FILE"

if [[ "$DO_FEEGRANT" == true ]]; then
  [[ -n "$GRANTER" ]] || hm_die "--granter required with --feegrant"
  extra=()
  [[ "$DRY_RUN" == true ]] && extra+=(--dry-run)
  "${SCRIPT_DIR}/feegrant-batch.sh" --granter "$GRANTER" --file "$OUT_FILE" "${extra[@]}"
fi

hm_summary
