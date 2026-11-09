#!/usr/bin/env bash
# Dense parallel test packing for terp-rs CI (host just + GHA matrix).
#
# Packing rule (no historical per-package timings while lock is broken):
#   - Prefer multi-package `cargo test -p a -p b … --lib` over one process per package
#     (shared target/, one rustc graph, denser utilization).
#   - On GHA, split into a *small* number of balanced packs (matrix cells), not
#     one-job-per-package sparse matrices (setup/cache overhead dominates tiny libs).
#   - Balance by package class / expected compile weight.
#   - Serial exceptions: offline IBC preflight → offline tests → rebuild → validate.
#
# Env:
#   CI_DENSE_MODE=all|matrix   host default: all (single multi-p invocation)
#   CI_DENSE_PARALLEL=0|1      matrix-mode packs on host in parallel with isolated
#                              CARGO_TARGET_DIR (default 0 — safer shared target)
#   CI_DENSE_PACK=<name>       run only this pack (GHA matrix / debug)
#   CARGO_BUILD_JOBS           optional rustc job cap (act defaults to 2 via lib.sh)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

# ── Pack definitions: name|space-separated packages ─────────────────────────
CORE_LIB_PACKS=(
  "core-libs-a|terp-auth terp-account terp-rs"
  "core-libs-b|crosslink-light-client cw721-nips"
)

CONTRACT_CORE_PACKS=(
  "contracts-a|terp-ed25519 terp-passkey terp-vsck"
  "contracts-b|terp-eth terp-bls terp-recovery"
  "contracts-c|terp-authenticator-suite cw-ics08-wasm-crosslink"
)

CONTRACT_WASM_PKGS="terp-ed25519 terp-passkey terp-vsck cw-ics08-wasm-crosslink"
CONTRACT_HEAVY_PKGS="terp-zkjwt"
CORE_LIB_ALL="terp-auth terp-account crosslink-light-client cw721-nips terp-rs"
CONTRACT_CORE_ALL="terp-ed25519 terp-passkey terp-vsck terp-eth terp-bls terp-recovery terp-authenticator-suite cw-ics08-wasm-crosslink"

usage() {
  cat <<'U'
Usage: scripts/ci/dense-packs.sh <command> [args]

Commands:
  list                         Print pack names and membership
  cargo-lib <pkg> [pkg...]     cargo test -p … --lib (multi-package)
  core-libs                    Run core internal lib packs (host default: all multi-p)
  contracts-core               Run extended contract lib packs
  contracts-wasm               Wasm/lib check for selected contracts
  contracts-heavy              Optional zk contract libs
  pack <name>                  Run a single named pack (matrix cell)

Env: CI_DENSE_MODE=all|matrix  CI_DENSE_PARALLEL=0|1  CI_DENSE_PACK=<name>
U
}

cargo_lib() {
  if [[ "$#" -lt 1 ]]; then
    echo "cargo_lib: need at least one package" >&2
    return 2
  fi
  _args=()
  for _p in "$@"; do
    _args+=(-p "$_p")
  done
  echo ">>> cargo test ${_args[*]} --lib"
  cargo test "${_args[@]}" --lib
}

cargo_wasm_check() {
  for _p in "$@"; do
    echo ">>> wasm/lib check -p $_p"
    if ! cargo check -p "$_p" --target wasm32-unknown-unknown --lib; then
      cargo check -p "$_p" --lib
    fi
  done
}

# Sets global PACK_NAME and PACK_PKGS from "name|pkg pkg"
parse_pack_line() {
  PACK_NAME="${1%%|*}"
  PACK_PKGS="${1#*|}"
}

run_named_pack() {
  _want="$1"
  for _line in "${CORE_LIB_PACKS[@]}" "${CONTRACT_CORE_PACKS[@]}"; do
    parse_pack_line "$_line"
    if [[ "$PACK_NAME" == "$_want" ]]; then
      # shellcheck disable=SC2086
      cargo_lib $PACK_PKGS
      return 0
    fi
  done
  case "$_want" in
    core-libs-all)
      # shellcheck disable=SC2086
      cargo_lib $CORE_LIB_ALL
      ;;
    contracts-core-all)
      # shellcheck disable=SC2086
      cargo_lib $CONTRACT_CORE_ALL
      ;;
    contracts-heavy)
      # shellcheck disable=SC2086
      cargo_lib $CONTRACT_HEAVY_PKGS
      ;;
    contracts-wasm)
      # shellcheck disable=SC2086
      cargo_wasm_check $CONTRACT_WASM_PKGS
      ;;
    *)
      echo "unknown pack: $_want" >&2
      return 1
      ;;
  esac
}

# $1 = which group: core-libs | contracts-core
run_pack_group() {
  _group="$1"
  _mode="${CI_DENSE_MODE:-all}"
  _only="${CI_DENSE_PACK:-}"

  if [[ -n "$_only" ]]; then
    run_named_pack "$_only"
    return
  fi

  if [[ "$_group" == "core-libs" ]]; then
    _packs=("${CORE_LIB_PACKS[@]}")
    _all="$CORE_LIB_ALL"
  else
    _packs=("${CONTRACT_CORE_PACKS[@]}")
    _all="$CONTRACT_CORE_ALL"
  fi

  if [[ "$_mode" == "all" ]]; then
    # shellcheck disable=SC2086
    cargo_lib $_all
    return
  fi

  if [[ "${CI_DENSE_PARALLEL:-0}" == "1" ]]; then
    _pids=""
    _failures=0
    for _line in "${_packs[@]}"; do
      parse_pack_line "$_line"
      (
        export CARGO_TARGET_DIR="${ROOT}/target/dense-${PACK_NAME}"
        mkdir -p "${CARGO_TARGET_DIR}"
        # shellcheck disable=SC2086
        cargo_lib $PACK_PKGS
      ) &
      _pids="$_pids $!"
    done
    for _pid in $_pids; do
      if ! wait "$_pid"; then
        _failures=$((_failures + 1))
      fi
    done
    if [[ "$_failures" -ne 0 ]]; then
      echo "dense-packs: ${_failures} pack(s) failed" >&2
      return 1
    fi
    return
  fi

  for _line in "${_packs[@]}"; do
    parse_pack_line "$_line"
    echo "::group::pack ${PACK_NAME}"
    # shellcheck disable=SC2086
    cargo_lib $PACK_PKGS
    echo "::endgroup::"
  done
}

cmd_list() {
  echo "# Core lib packs (GHA: CI_DENSE_MODE=matrix)"
  for _line in "${CORE_LIB_PACKS[@]}"; do
    parse_pack_line "$_line"
    echo "  ${PACK_NAME}: ${PACK_PKGS}"
  done
  echo "# Contract core packs"
  for _line in "${CONTRACT_CORE_PACKS[@]}"; do
    parse_pack_line "$_line"
    echo "  ${PACK_NAME}: ${PACK_PKGS}"
  done
  echo "# Other"
  echo "  contracts-wasm: ${CONTRACT_WASM_PKGS}"
  echo "  contracts-heavy: ${CONTRACT_HEAVY_PKGS}"
  echo "  core-libs-all / contracts-core-all: multi-package collapse"
}

main() {
  _cmd="${1:-}"
  if [[ "$#" -gt 0 ]]; then shift; fi
  case "$_cmd" in
    list) cmd_list ;;
    cargo-lib) cargo_lib "$@" ;;
    core-libs) run_pack_group core-libs ;;
    contracts-core) run_pack_group contracts-core ;;
    contracts-wasm)
      # shellcheck disable=SC2086
      cargo_wasm_check $CONTRACT_WASM_PKGS
      ;;
    contracts-heavy)
      # shellcheck disable=SC2086
      cargo_lib $CONTRACT_HEAVY_PKGS
      ;;
    pack)
      if [[ -z "${1:-}" ]]; then echo "pack name required" >&2; exit 2; fi
      run_named_pack "$1"
      ;;
    -h|--help|help)
      usage
      ;;
    "")
      usage
      exit 2
      ;;
    *)
      echo "unknown command: $_cmd" >&2
      usage
      exit 2
      ;;
  esac
}

main "$@"
