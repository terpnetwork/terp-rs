#!/usr/bin/env bash
# Collab-safe HashMerchant field ladder: R0 → R1 → R3-mock.
# Forces ICT_HM_RUNTIME=mock. Never builds hash-market Docker images.
#
# Usage:
#   ./crates/ict-rs/scripts/field-ladder-mock.sh
#   cd crates/ict-rs && just field-ladder-mock
#   SKIP_R3=1 ./crates/ict-rs/scripts/field-ladder-mock.sh   # R0+R1 only
#
# Docs: docs/hashmerchant/FIELD-E2E.md
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Resolve monorepo root (support both scripts/ and tools/hashmerchant-support/ install paths)
ROOT=""
for cand in \
  "$(cd "$SCRIPT_DIR/../../.." 2>/dev/null && pwd)" \
  "$(cd "$SCRIPT_DIR/../.." 2>/dev/null && pwd)" \
  "$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"; do
  if [[ -n "$cand" && -f "$cand/tools/hashmerchant-support/hm-support.sh" ]]; then
    ROOT="$cand"
    break
  fi
done

if [[ -z "$ROOT" ]]; then
  echo "ERROR: monorepo root not found (need tools/hashmerchant-support/hm-support.sh)"
  echo "  script dir: $SCRIPT_DIR"
  exit 1
fi

export ICT_HM_RUNTIME=mock
# Do not inherit full-image overrides into a field mock run
unset ICT_HM_IMAGE_MINT ICT_HM_IMAGE_SIDECAR 2>/dev/null || true

HM_DIR="$ROOT/crates/terp-rs/tools/hash-market"
ICT_DIR="$ROOT/crates/ict-rs"
REPORT="${ICT_L3_REPORT:-$ICT_DIR/hashmerchant_l3_report.json}"

echo "═══════════════════════════════════════════════════════════"
echo " Field ladder (mock) — root=$ROOT"
echo " ICT_HM_RUNTIME=$ICT_HM_RUNTIME  (forced)"
echo "═══════════════════════════════════════════════════════════"

echo ""
echo "=== R0: hash-market lib suite (no Docker) ==="
(
  cd "$HM_DIR"
  cargo test -p hash-market --lib tests::suite
)

echo ""
echo "=== R1: modular matrix (ICT_MOCK, no Docker) ==="
(
  cd "$ICT_DIR"
  if command -v just >/dev/null 2>&1; then
    just test-hashmerchant-matrix
  else
    ICT_MOCK=1 cargo test -p ict-rs --features "testing,hashmerchant,terp" --test hashmerchant_modular_matrix
  fi
)

if [[ "${SKIP_R3:-0}" == "1" ]]; then
  echo ""
  echo "=== R3-mock: SKIPPED (SKIP_R3=1) ==="
  echo "Field partial green: R0+R1 only. Run R3 when Docker is free."
  exit 0
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "ERROR: docker not in PATH (required for R3-mock). Set SKIP_R3=1 for R0+R1 only."
  exit 1
fi
if ! docker info >/dev/null 2>&1; then
  echo "ERROR: docker daemon not reachable. Set SKIP_R3=1 for R0+R1 only."
  exit 1
fi

echo ""
echo "=== R3-mock: L3 Ubuntu bootstrap (mock_runtime.py; no hash-market image build) ==="
(
  cd "$ICT_DIR"
  export ICT_L3_REPORT="$REPORT"
  if command -v just >/dev/null 2>&1; then
    just l3-bootstrap-mock
  else
    ICT_HM_RUNTIME=mock ICT_L3_REPORT="$REPORT" \
      cargo run -p ict-rs --example hashmerchant_bootstrap_ubuntu --features docker
  fi
)

echo ""
echo "═══════════════════════════════════════════════════════════"
echo " Field ladder mock GREEN (R0 + R1 + R3-mock)"
echo " Report: $REPORT"
echo " Full runtime is optional — see docs/hashmerchant/FIELD-E2E.md"
echo "═══════════════════════════════════════════════════════════"
