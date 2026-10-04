#!/usr/bin/env bash
# Build cw-ics08-wasm-terp via the Terp Docker optimizer (rustc 1.91).
# Standard: crates/terp-rs/docs/wasm-artifacts.md.
#
# Published CosmWasm / 08-wasm artifacts must not use host cargo wasm.
# Store on-chain with `terpd tx ibc-wasm store-code` (08-wasm module), not
# `tx wasm store`.
#
# Usage (from terp-rs/):
#   ./scripts/build-ics08-wasm-terp.sh
#   OPTIMIZER_IMAGE=terpnetwork/optimizer-arm64:0.17.0 ./scripts/build-ics08-wasm-terp.sh
set -euo pipefail

if [[ "${FORCE_HOST_WASM:-0}" == "1" ]]; then
  echo "error: FORCE_HOST_WASM is not supported; 08-wasm artifacts must use Docker optimizer (rustc 1.91)" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CRATES_ROOT="$(cd "${CRATE_ROOT}/.." && pwd)"
CRATE_NAME="$(basename "${CRATE_ROOT}")"

ARCH="$(uname -m)"
case "${ARCH}" in
  arm64|aarch64) DEFAULT_IMAGE="terpnetwork/optimizer-arm64:0.17.0" ;;
  x86_64|amd64)  DEFAULT_IMAGE="terpnetwork/optimizer:0.17.0" ;;
  *)             DEFAULT_IMAGE="terpnetwork/optimizer-arm64:0.17.0" ;;
esac
OPTIMIZER_IMAGE="${OPTIMIZER_IMAGE:-${DEFAULT_IMAGE}}"

echo "==> optimizer image: ${OPTIMIZER_IMAGE}"
echo "    volume: ${CRATES_ROOT} → /workspace"
echo "    project: /workspace/${CRATE_NAME}"
echo "    package: cw-ics08-wasm-terp"

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker is required" >&2
  exit 1
fi
if ! docker image inspect "${OPTIMIZER_IMAGE}" >/dev/null 2>&1; then
  echo "error: image ${OPTIMIZER_IMAGE} not found." >&2
  echo "Build from optimizer crate: cd <terp-core>/optimizer && make build-arm64" >&2
  exit 1
fi

mkdir -p "${CRATE_ROOT}/artifacts"

# Optimizer image rustc 1.91. Override ENTRYPOINT (bob/workspace) so we only
# emit this LC. Path deps resolve because crates/ is mounted at /workspace.
docker run --rm \
  --entrypoint /bin/ash \
  -v "${CRATES_ROOT}:/workspace" \
  -v "${CRATE_ROOT}/artifacts:/artifacts_out" \
  -w "/workspace/${CRATE_NAME}" \
  -e CARGO_TARGET_DIR=/target \
  -e RUSTFLAGS="-C link-arg=-s -C target-feature=-simd128" \
  --mount type=volume,source="${CRATE_NAME}_optimizer_cache",target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  "${OPTIMIZER_IMAGE}" \
  -c "set -e
      rustc --version
      cargo build --release --target wasm32-unknown-unknown --lib -p cw-ics08-wasm-terp -p terp-iavl-lc
      ls -la /target/wasm32-unknown-unknown/release/cw_ics08_wasm_terp.wasm
      cp -f /target/wasm32-unknown-unknown/release/cw_ics08_wasm_terp.wasm /artifacts_out/
     "

ARTIFACT="${CRATE_ROOT}/artifacts/cw_ics08_wasm_terp.wasm"
if [[ ! -f "${ARTIFACT}" ]]; then
  echo "error: expected artifact missing: ${ARTIFACT}" >&2
  exit 1
fi

# Size-optimize only. Keep bulk-memory (wasmvm CAP_BULK_MEMORY / 08-wasm VM).
WASM_OPT_BIN="$(command -v wasm-opt || true)"
if [[ -n "${WASM_OPT_BIN}" ]]; then
  echo "==> wasm-opt -Os (bulk-memory kept): cw_ics08_wasm_terp.wasm"
  TMP="$(mktemp "${TMPDIR:-/tmp}/cw-opt.XXXXXX.wasm")"
  "${WASM_OPT_BIN}" -Os --enable-bulk-memory --signext-lowering "${ARTIFACT}" -o "${TMP}"
  mv "${TMP}" "${ARTIFACT}"
fi

(
  cd "${CRATE_ROOT}/artifacts"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum cw_ics08_wasm_terp.wasm | tee -a checksums.txt
  else
    shasum -a 256 cw_ics08_wasm_terp.wasm | tee -a checksums.txt
  fi
  ls -la cw_ics08_wasm_terp.wasm
  file cw_ics08_wasm_terp.wasm || true
)

echo "==> 08-wasm LC artifact ready (store with: terpd tx ibc-wasm store-code)"
