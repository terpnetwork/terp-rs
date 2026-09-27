#!/usr/bin/env bash
# Build cw-ics08-wasm-crosslink via the Terp Docker optimizer.
#
# MSRV for CosmWasm artifacts: Rust **1.86** only (pinned in optimizer image +
# workspace rust-version). No host rustc / older toolchains for published wasm.
#
# Run from the terp-rs crate root. Volume path is mounted at /workspace so path
# deps resolve (../cosmwasm, ../cw-minus, ../ibc-proto-rs, …).
#
# Usage (from terp-rs/):
#   ./scripts/build-crosslink-wasm.sh
#   ./scripts/build-crosslink-wasm.sh /path/to/crates          # explicit volume
#   OPTIMIZER_IMAGE=terpnetwork/optimizer-arm64:0.17.0 ./scripts/build-crosslink-wasm.sh
set -euo pipefail

# Refuse accidental host cargo wasm publish paths.
if [[ "${FORCE_HOST_WASM:-0}" == "1" ]]; then
  echo "error: FORCE_HOST_WASM is not supported; CosmWasm artifacts must use Docker optimizer (rustc 1.86)" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CRATE_NAME="$(basename "${CRATE_ROOT}")"

# Volume host path (mounted at /workspace). Optional first arg.
VOLUME_PATH="${1:-}"
if [[ -z "${VOLUME_PATH}" ]]; then
  VOLUME_PATH="$(cd "${CRATE_ROOT}/.." && pwd)"
else
  VOLUME_PATH="$(cd "${VOLUME_PATH}" && pwd)"
fi

# Crate must live under the volume path as VOLUME_PATH/<CRATE_NAME>
if [[ ! -d "${VOLUME_PATH}/${CRATE_NAME}" ]]; then
  echo "error: expected crate at ${VOLUME_PATH}/${CRATE_NAME}" >&2
  echo "  volume path: ${VOLUME_PATH}" >&2
  echo "  crate name:  ${CRATE_NAME}" >&2
  exit 1
fi

ARCH="$(uname -m)"
case "${ARCH}" in
  arm64|aarch64) DEFAULT_IMAGE="terpnetwork/optimizer-arm64:0.17.0" ;;
  x86_64|amd64)  DEFAULT_IMAGE="terpnetwork/optimizer:0.17.0" ;;
  *)             DEFAULT_IMAGE="terpnetwork/optimizer-arm64:0.17.0" ;;
esac
OPTIMIZER_IMAGE="${OPTIMIZER_IMAGE:-${DEFAULT_IMAGE}}"

PROJECT_DIR="/workspace/${CRATE_NAME}"

echo "==> Docker CosmWasm optimizer (run from crate, volume as path)"
echo "    crate:    ${CRATE_ROOT}"
echo "    volume:   ${VOLUME_PATH}  →  /workspace"
echo "    project:  ${PROJECT_DIR}"
echo "    image:    ${OPTIMIZER_IMAGE}"
echo

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker is required" >&2
  exit 1
fi

if ! docker image inspect "${OPTIMIZER_IMAGE}" >/dev/null 2>&1; then
  echo "error: image ${OPTIMIZER_IMAGE} not found." >&2
  echo "Build from optimizer crate:" >&2
  echo "  cd <terp-core>/optimizer && make build-arm64   # or build-amd64" >&2
  exit 1
fi

# Workdir is the crate inside the mounted volume (path-dep layout preserved).
docker run --rm \
  -v "${VOLUME_PATH}:/workspace" \
  -w "${PROJECT_DIR}" \
  -e "PROJECT_DIR=${PROJECT_DIR}" \
  --mount type=volume,source="${CRATE_NAME}_optimizer_cache",target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  "${OPTIMIZER_IMAGE}"

ARTIFACT="${CRATE_ROOT}/artifacts/cw_ics08_wasm_crosslink.wasm"
if [[ ! -f "${ARTIFACT}" ]]; then
  echo "error: expected artifact missing: ${ARTIFACT}" >&2
  ls -la "${CRATE_ROOT}/artifacts" 2>/dev/null || true
  exit 1
fi

# Host post-process: image ships wasm-opt 116; host binaryen ≥120 can re-lower
# bulk-memory for wasmvm gatekeepers that still reject those ops.
WASM_OPT_BIN="$(command -v wasm-opt || true)"
if [[ -n "${WASM_OPT_BIN}" ]] \
  && "${WASM_OPT_BIN}" --help 2>&1 | grep -qE 'llvm-memory-copy-fill-lowering|memory-copy-fill-lowering'; then
  echo "==> host ${WASM_OPT_BIN}: lower bulk-memory for CosmWasm gatekeeper"
  TMP="$(mktemp "${TMPDIR:-/tmp}/cw-opt.XXXXXX.wasm")"
  "${WASM_OPT_BIN}" -Os \
    --llvm-memory-copy-fill-lowering \
    --signext-lowering \
    --disable-bulk-memory \
    "${ARTIFACT}" -o "${TMP}"
  mv "${TMP}" "${ARTIFACT}"
  (
    cd "${CRATE_ROOT}/artifacts"
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum -- *.wasm | tee checksums.txt
    else
      shasum -a 256 -- *.wasm | tee checksums.txt
    fi
  )
fi

mkdir -p "${CRATE_ROOT}/contracts/light-clients/cw-ics08-wasm-crosslink/artifacts"
cp -f "${ARTIFACT}" "${CRATE_ROOT}/contracts/light-clients/cw-ics08-wasm-crosslink/artifacts/"
if [[ -f "${CRATE_ROOT}/artifacts/checksums.txt" ]]; then
  cp -f "${CRATE_ROOT}/artifacts/checksums.txt" \
    "${CRATE_ROOT}/contracts/light-clients/cw-ics08-wasm-crosslink/artifacts/"
fi

echo
echo "==> published"
ls -la "${ARTIFACT}"
( cd "${CRATE_ROOT}/artifacts" && (command -v sha256sum >/dev/null && sha256sum cw_ics08_wasm_crosslink.wasm || shasum -a 256 cw_ics08_wasm_crosslink.wasm) )
echo "Update tests/tests/crosslink_light_client.rs WASM_CHECKSUM_HEX if the digest changed."
