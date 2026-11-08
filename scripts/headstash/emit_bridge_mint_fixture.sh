#!/usr/bin/env bash
# Thin wrapper → spectrum fixtures emit (Tacit anvil → Domain B mint packet).
# Prefer: `just demo-bridge-fixture` from crates/headstash.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
exec bash "$ROOT/docs/plans/spectrum/fixtures/emit_bridge_mint_fixture.sh" "$@"
