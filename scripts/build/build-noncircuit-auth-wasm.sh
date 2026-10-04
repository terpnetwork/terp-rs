#!/usr/bin/env bash
# Rebuild passkey / recovery / ed25519 / eth wasm with the CosmWasm optimizer image
# (rustc 1.91). Terp wasmvm meters bulk-memory; do **not** strip memory.copy/fill.
# Standard: crates/terp-rs/docs/wasm-artifacts.md.
#
# Usage (from terp-rs/):
#   ./scripts/build-noncircuit-auth-wasm.sh
#   OPTIMIZER_IMAGE=terpnetwork/optimizer-arm64:0.17.0 ./scripts/build-noncircuit-auth-wasm.sh
set -euo pipefail

if [[ "${FORCE_HOST_WASM:-0}" == "1" ]]; then
  echo "error: FORCE_HOST_WASM is not supported; use the Docker optimizer image" >&2
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

PACKAGES=(terp-passkey terp-recovery terp-ed25519 terp-eth)
WASM_NAMES=(terp_passkey.wasm terp_recovery.wasm terp_ed25519.wasm terp_eth.wasm)

echo "==> optimizer image: ${OPTIMIZER_IMAGE}"
echo "    volume: ${CRATES_ROOT} → /workspace"
echo "    project: /workspace/${CRATE_NAME}"

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker is required" >&2
  exit 1
fi
if ! docker image inspect "${OPTIMIZER_IMAGE}" >/dev/null 2>&1; then
  echo "error: image ${OPTIMIZER_IMAGE} not found" >&2
  exit 1
fi

mkdir -p "${CRATE_ROOT}/artifacts"

PKG_ARGS=()
for p in "${PACKAGES[@]}"; do
  PKG_ARGS+=(-p "$p")
done

# Image ENTRYPOINT is terp-optimize.sh (bob over the whole workspace). Override
# to rustc 1.91 cargo -p so we only emit the four non-circuit authenticators.
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
      rustup show
      cargo build --release --target wasm32-unknown-unknown --lib ${PKG_ARGS[*]}
      ls -la /target/wasm32-unknown-unknown/release/*.wasm || true
      cp -f /target/wasm32-unknown-unknown/release/terp_passkey.wasm /artifacts_out/
      cp -f /target/wasm32-unknown-unknown/release/terp_recovery.wasm /artifacts_out/
      cp -f /target/wasm32-unknown-unknown/release/terp_ed25519.wasm /artifacts_out/
      cp -f /target/wasm32-unknown-unknown/release/terp_eth.wasm /artifacts_out/
      ls -la /artifacts_out/terp_passkey.wasm /artifacts_out/terp_recovery.wasm /artifacts_out/terp_ed25519.wasm /artifacts_out/terp_eth.wasm
     "

# Size-optimize only. Keep bulk-memory (wasmvm CAP_BULK_MEMORY).
WASM_OPT_BIN="$(command -v wasm-opt || true)"
for name in "${WASM_NAMES[@]}"; do
  ART="${CRATE_ROOT}/artifacts/${name}"
  if [[ ! -f "${ART}" ]]; then
    echo "error: missing ${ART}" >&2
    exit 1
  fi
  if [[ -n "${WASM_OPT_BIN}" ]]; then
    echo "==> wasm-opt -Os (bulk-memory kept): ${name}"
    TMP="$(mktemp "${TMPDIR:-/tmp}/cw-opt.XXXXXX.wasm")"
    "${WASM_OPT_BIN}" -Os --enable-bulk-memory --signext-lowering "${ART}" -o "${TMP}"
    mv "${TMP}" "${ART}"
  fi
done

(
  cd "${CRATE_ROOT}/artifacts"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${WASM_NAMES[@]}" | tee -a checksums.txt
  else
    shasum -a 256 "${WASM_NAMES[@]}" | tee -a checksums.txt
  fi
  ls -la "${WASM_NAMES[@]}"
)

echo "==> non-circuit authenticator wasm ready"
