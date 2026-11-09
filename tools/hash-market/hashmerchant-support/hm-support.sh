#!/usr/bin/env bash
# hm-support — HashMerchant support façade + demo automation
#
# Modes:
#   prepare | install | authenticate | configure | deploy | integrate | smoke | monitor
#   accounts … | whitelist … | demo … | package-usb | lab | help
#
# Env: BIND, HM_HOME, HM_NODE, HM_CHAIN_ID, HM_KEYRING, HASHMERCHANT_SIDECAR_URL, HM_MAINNET_OK
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "${ROOT}/lib.sh"

usage() {
  cat <<'EOF'
hm-support — wire HashMerchant support + demos

Core (Path B wire):
  prepare              Check host prerequisites
  install              Show artifact layout / verify checksums if present
  authenticate         Check keyring / print auth checklist
  configure            Validate sidecar-url + optional write instructions
  deploy               Hints for wasm store/instantiate (test contract)
  integrate            register-contract + escrow helpers
  smoke                Sidecar health + optional root wait
  monitor              One-shot health snapshot
  lab                  Run tests/tsh/hashmerchant/a.sh (full e2e)

Accounts / demos:
  accounts bootstrap   Create keys + resolve members.json
  accounts feegrant    Batch feegrant from members file
  accounts verify      Query allowances; optional --fee-granter smoke
  whitelist build      members.json → merkle tree + proofs
  whitelist proof      Print proof for address
  whitelist self-test  Golden codec test
  demo nft-help        Oracle-priced NFT mint demo docs/recipe
  package-usb          Build offline kit tarball

Examples:
  ./hm-support.sh prepare
  ./hm-support.sh accounts feegrant --granter ops --file whitelist/fixtures/members.json --dry-run
  ./hm-support.sh accounts verify --file whitelist/fixtures/members.json --granter ops --dry-run
  ./hm-support.sh whitelist build --in whitelist/fixtures/members.json --out /tmp/wl
  HASHMERCHANT_SIDECAR_URL=http://127.0.0.1:18888 ./hm-support.sh smoke
EOF
}

cmd_prepare() {
  hm_log "=== prepare ==="
  local missing=0
  for c in jq curl python3; do
    if command -v "$c" >/dev/null 2>&1; then hm_ok "$c"; else hm_fail "missing $c"; missing=1; fi
  done
  if command -v "$BIND" >/dev/null 2>&1; then hm_ok "$BIND"; else hm_warn "$BIND not in PATH (use terp-installer for node ops)"; fi
  if command -v anvil >/dev/null 2>&1; then hm_ok "anvil (lab)"; else hm_info "anvil optional (lab e2e only)"; fi
  hm_info "threat: never put mainnet mnemonics on USB; PII stays off-chain; oracle never mints value"
  hm_info "node install: websites/terp.network/get (terp-installer) — not this tool"
  (( missing == 0 ))
  hm_summary
}

cmd_install() {
  hm_log "=== install (layout) ==="
  local paths=(
    "$ROOT/lib.sh"
    "$ROOT/hm-support.sh"
    "$ROOT/accounts/bootstrap.sh"
    "$ROOT/accounts/feegrant-batch.sh"
    "$ROOT/accounts/verify.sh"
    "$ROOT/accounts/README.md"
    "$ROOT/whitelist/build.py"
    "$ROOT/demo/nft-oracle-mint/README.md"
    "$ROOT/sidecar-api.md"
  )
  local p
  for p in "${paths[@]}"; do
    if [[ -f "$p" ]]; then hm_ok "$(basename "$p")"; else hm_fail "missing $p"; fi
  done
  if [[ -f "$ROOT/usb/SHASUMS" ]]; then
    hm_info "verifying usb/SHASUMS"
    (cd "$ROOT" && shasum -c usb/SHASUMS) && hm_ok "checksums" || hm_fail "checksums"
  else
    hm_info "no usb/SHASUMS yet — run package-usb to generate"
  fi
  # e2e reference
  local e2e="$ROOT/../../tests/tsh/hashmerchant/a.sh"
  if [[ -f "$e2e" ]]; then hm_ok "e2e a.sh present"; else hm_warn "e2e a.sh not found at $e2e"; fi
  hm_summary
}

cmd_authenticate() {
  hm_log "=== authenticate ==="
  hm_info "checklist:"
  hm_info "  1. Ops granter key only on secure host (not on media after bootstrap)"
  hm_info "  2. HM_KEYRING=$HM_KEYRING"
  hm_info "  3. Feegrant caps + expiry on demos"
  hm_info "  4. HM_MAINNET_OK required for mainnet-like chain ids"
  if command -v "$BIND" >/dev/null 2>&1; then
    if hm_bind keys list --output json 2>/dev/null | jq -e 'length >= 0' >/dev/null 2>&1; then
      hm_ok "keyring reachable"
    else
      hm_warn "could not list keys (set HM_HOME / keyring)"
    fi
  else
    hm_warn "$BIND not available"
  fi
  hm_summary
}

cmd_configure() {
  hm_log "=== configure ==="
  local url="${HASHMERCHANT_SIDECAR_URL:-}"
  if [[ -z "$url" ]]; then
    hm_info "HASHMERCHANT_SIDECAR_URL unset — empty vote extensions (safe default)"
    hm_info "set app.toml [hashmerchant] sidecar-url OR export HASHMERCHANT_SIDECAR_URL=http://HOST:PORT"
  else
    hm_info "HASHMERCHANT_SIDECAR_URL=$url"
    if hm_sidecar_health "$url" 2>/dev/null; then
      hm_ok "sidecar /health"
    else
      hm_fail "sidecar /health failed"
    fi
  fi
  hm_info "see sidecar-api.md for /vote-extension JSON contract"
  hm_summary
}

cmd_deploy() {
  hm_log "=== deploy (hints) ==="
  cat <<EOF
Deploy test/claim wasm (lab):
  1. Build: tests/tsh/hashmerchant/build-contract.sh
  2. Store:  $BIND tx wasm store hashmerchant_test.wasm --from <key> ...
  3. Init:   $BIND tx wasm instantiate <code_id> '{}' --label hashmerchant-test ...
  4. Record code_id + contract address as artifacts

NFT demo: see demo/nft-oracle-mint/README.md
EOF
  hm_ok "printed deploy hints"
}

cmd_integrate() {
  hm_log "=== integrate ==="
  local contract="${1:-}"
  local chain_uid="${2:-ethereum-mainnet}"
  local substore="${3:-state}"
  local escrow="${4:-1000000uterp}"
  local from="${5:-}"
  if [[ -z "$contract" || -z "$from" ]]; then
    cat <<EOF
Usage: hm-support integrate <contract> [chain_uid] [substore] [escrow] --from <key>
  $BIND tx hashmerchant register-contract <contract> <chain_uid> <substore> <escrow> --from <key>
  $BIND tx hashmerchant refill-escrow <contract> <amount> --from <key>
EOF
    return 0
  fi
  shift 5 || true
  hm_require_bind
  hm_guard_mainnet
  # shellcheck disable=SC2046
  hm_bind tx hashmerchant register-contract "$contract" "$chain_uid" "$substore" "$escrow" \
    $(hm_tx_flags) --from "$from" "$@"
  hm_ok "register-contract submitted"
}

cmd_smoke() {
  hm_log "=== smoke ==="
  local url="${HASHMERCHANT_SIDECAR_URL:-}"
  if [[ -n "$url" ]]; then
    if hm_sidecar_health "$url"; then
      hm_ok "sidecar health"
      if ve="$(hm_sidecar_vote_extension "$url" 2>/dev/null)"; then
        echo "$ve" | jq -e '.root // .chain_uid // .attestations' >/dev/null && hm_ok "vote-extension JSON" || hm_warn "unexpected VE body"
      else
        hm_fail "vote-extension fetch"
      fi
    else
      hm_fail "sidecar health"
    fi
  else
    hm_warn "no HASHMERCHANT_SIDECAR_URL — skip sidecar smoke"
  fi
  if command -v "$BIND" >/dev/null 2>&1 && [[ -n "${HM_HOME}${HM_NODE}" ]]; then
    if hm_wait_for_height 1 5 2>/dev/null; then hm_ok "chain reachable"; else hm_warn "chain status not ready"; fi
  fi
  hm_summary
}

cmd_monitor() {
  cmd_smoke
  if command -v "$BIND" >/dev/null 2>&1; then
    local chain_uid="${1:-ethereum-mainnet}"
    local algo="${2:-keccak256}"
    local resp
    resp="$(hm_bind q hashmerchant root "$chain_uid" "$algo" --output json 2>/dev/null || echo '{}')"
    hm_info "root query: $(echo "$resp" | jq -c . 2>/dev/null || echo "$resp")"
  fi
}

cmd_lab() {
  local e2e="$ROOT/../../tests/tsh/hashmerchant/a.sh"
  [[ -f "$e2e" ]] || hm_die "lab script missing: $e2e"
  hm_log "=== lab: $e2e ==="
  exec bash "$e2e"
}

cmd_package_usb() {
  hm_log "=== package-usb ==="
  local dest="${1:-$ROOT/usb/hm-support-kit}"
  rm -rf "$dest"
  mkdir -p "$dest"
  rsync -a --exclude usb/hm-support-kit --exclude 'usb/*.tar.gz' \
    "$ROOT/" "$dest/" 2>/dev/null || cp -R "$ROOT/." "$dest/"
  # strip nested copy if any
  rm -rf "$dest/usb/hm-support-kit"
  (
    cd "$dest"
    find . -type f ! -name SHASUMS ! -name '*.tar.gz' -print0 | sort -z | xargs -0 shasum -a 256 > SHASUMS
  )
  mkdir -p "$ROOT/usb"
  cp "$dest/SHASUMS" "$ROOT/usb/SHASUMS"
  tar -C "$(dirname "$dest")" -czf "${dest}.tar.gz" "$(basename "$dest")"
  hm_ok "kit ${dest}.tar.gz"
  hm_info "verify: tar xzf … && shasum -c SHASUMS"
}

cmd_accounts() {
  local sub="${1:-}"
  shift || true
  case "$sub" in
    bootstrap) exec bash "$ROOT/accounts/bootstrap.sh" "$@" ;;
    feegrant) exec bash "$ROOT/accounts/feegrant-batch.sh" "$@" ;;
    verify) exec bash "$ROOT/accounts/verify.sh" "$@" ;;
    *) hm_die "accounts subcommands: bootstrap | feegrant | verify" ;;
  esac
}

cmd_whitelist() {
  local sub="${1:-}"
  shift || true
  case "$sub" in
    build)
      local inp="" out=""
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --in) inp="${2:-}"; shift 2 ;;
          --out) out="${2:-}"; shift 2 ;;
          *) hm_die "unknown: $1" ;;
        esac
      done
      [[ -n "$inp" && -n "$out" ]] || hm_die "whitelist build --in FILE --out DIR"
      exec python3 "$ROOT/whitelist/build.py" --in "$inp" --out "$out"
      ;;
    proof)
      local inp="" addr=""
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --in) inp="${2:-}"; shift 2 ;;
          --address) addr="${2:-}"; shift 2 ;;
          --out) shift 2 ;; # ignore, use tmp
          *) hm_die "unknown: $1" ;;
        esac
      done
      [[ -n "$inp" && -n "$addr" ]] || hm_die "whitelist proof --in members.json --address ADDR"
      tmp="$(mktemp -d)"
      python3 "$ROOT/whitelist/build.py" --in "$inp" --out "$tmp" --proof-for "$addr"
      ;;
    self-test) exec python3 "$ROOT/whitelist/build.py" --self-test ;;
    *) hm_die "whitelist subcommands: build | proof | self-test" ;;
  esac
}

cmd_demo() {
  local sub="${1:-nft-help}"
  case "$sub" in
    nft-help|nft|help)
      cat "$ROOT/demo/nft-oracle-mint/README.md"
      ;;
    nft-recipe)
      exec bash "$ROOT/demo/nft-oracle-mint/recipe.sh" "${@:2}"
      ;;
    *) hm_die "demo subcommands: nft-help | nft-recipe" ;;
  esac
}

main() {
  local cmd="${1:-help}"
  shift || true
  case "$cmd" in
    prepare) cmd_prepare "$@" ;;
    install) cmd_install "$@" ;;
    authenticate|auth) cmd_authenticate "$@" ;;
    configure|config) cmd_configure "$@" ;;
    deploy) cmd_deploy "$@" ;;
    integrate) cmd_integrate "$@" ;;
    smoke) cmd_smoke "$@" ;;
    monitor) cmd_monitor "$@" ;;
    lab) cmd_lab "$@" ;;
    package-usb) cmd_package_usb "$@" ;;
    accounts) cmd_accounts "$@" ;;
    whitelist) cmd_whitelist "$@" ;;
    demo) cmd_demo "$@" ;;
    help|-h|--help) usage ;;
    *) usage; hm_die "unknown command: $cmd" ;;
  esac
}

main "$@"
