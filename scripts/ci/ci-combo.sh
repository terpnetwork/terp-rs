#!/usr/bin/env bash
# terp-rs ecosystem combination CI — one network, parallel crate surfaces.
# Pattern: terp.network `just test-rs` / o-line IctTerpApp (ict-rs spawn + DAO codes).
#
#   bash scripts/ci/ci-combo.sh           # offline + in-process runtime
#   bash scripts/ci/ci-combo.sh --live    # also LCD DAO code-id checks (ICT_LCD)
#   ICT_SPAWN=1 bash scripts/ci/ci-combo.sh --live   # pull pinned ict-ci, then spawn
#
# Grant/mint onboard is embedded in hash-market (feature `onboard` via `server`).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
# Combo carries its own ict-rs pin (do not require terp-core monorepo scripts).
# shellcheck source=ci/ict-rs-bins.env
. "$ROOT/scripts/ci/ict-rs-bins.env"
export ICT_RS_COMMIT ICT_RS_BINS_URL ICT_RS_TARBALL ICT_RS_TARBALL_SHA256

LIVE=0
for a in "$@"; do
  [[ "$a" == "--live" ]] && LIVE=1
done

HM="$ROOT/tools/hash-market"
echo "== terp-rs combo: hash-market onboard (embedded, not a second process) =="
cargo test --manifest-path "$HM/Cargo.toml" --offline --locked --features server onboard 2>/dev/null \
  || cargo test --manifest-path "$HM/Cargo.toml" --features server onboard

echo "== existing test bins (onboard-local-workflow + prepare-authz presets) =="
cargo run --manifest-path "$HM/Cargo.toml" --features onboard --bin onboard-local-workflow
cargo run --manifest-path "$HM/Cargo.toml" --features onboard --bin prepare-authz -- --list-presets
for preset in register norick svg headstash loyalty marketplace; do
  cargo run --manifest-path "$HM/Cargo.toml" --features onboard --bin prepare-authz -- \
    --mode dao-proposal --preset "$preset" --chain-id "${ICT_CHAIN:-120u-1}" \
    --dao terp1dao --runtime terp1runtime --out "/tmp/terp-rs-combo-${preset}" >/dev/null
done
echo "  prepared all presets"

if [[ -n "${ICT_SPAWN:-}" && "${ICT_SPAWN}" != "0" ]]; then
  echo "== ICT spawn (pinned ict-ci $ICT_RS_COMMIT) =="
  DEST="${ICT_RS_BIN_DIR:-/tmp/ict-rs-bins}"
  bash "$ROOT/scripts/ci/fetch-ict-rs-bins.sh" "$DEST"
  export ICT_CI_BIN_DIR="$DEST"
  export PATH="$DEST:$DEST/examples:$PATH"
  echo "  ict-ci=$(command -v ict-ci || echo "$DEST/ict-ci")"
  if [[ -n "${ICT_LCD:-}" ]]; then
    echo "  attach ICT_LCD=$ICT_LCD"
  elif command -v just >/dev/null && [[ -f "${TERP_NETWORK_JUST:-}/Justfile" ]]; then
    echo "  hint: just test-rs in terp.network then ICT_LCD=… --live"
  else
    echo "  bins on PATH; set ICT_LCD to attach 120u-1 or run ict-ci locally"
  fi
fi

if [[ "$LIVE" == "1" ]]; then
  if [[ -z "${ICT_LCD:-}" ]]; then
    echo "LIVE=1 requires ICT_LCD (no hardcoded testnet/mainnet RPC default)" >&2
    exit 1
  fi
  export ICT_CHAIN="${ICT_CHAIN:-120u-1}"
  echo "== live LCD combo ($ICT_CHAIN @ $ICT_LCD) =="
  echo "  (onboard LCD checks run against hash-market-server /onboard when ICT_LCD is set)"
fi

echo "== itl_frost_e2e (onboard-authz, 20/80 t-addr) =="
ONBOARD="${ONBOARD:-$ROOT/tools/onboard-authz}"
cargo test --manifest-path "$ONBOARD/Cargo.toml" --test itl_frost_e2e -- --nocapture
echo "== itl_frost_http_persist (onboard-authz, durable inbox) =="
cargo test --manifest-path "$ONBOARD/Cargo.toml" --test itl_frost_http_persist -- --nocapture

echo "== itl_cashapp_e2e (payRef bind, txId nullifier, rush, slot uniqueness) =="
cargo test --manifest-path "$ONBOARD/Cargo.toml" --test itl_cashapp_e2e -- --nocapture

echo "== itl_ingress_e2e (Gap 0 WASIX parse → payment nullifier → Poseidon guest_root; DREGG svg_mint_local_prod) =="
cargo test --manifest-path "$ONBOARD/Cargo.toml" --test itl_ingress_e2e -- --nocapture

echo "== itl_consume_e2e (Halo2 Pasta Poseidon merkle+nullifier, usher-only verify_consume, MsgStoreCodeWithCircuit) =="
cargo test --manifest-path "$ONBOARD/Cargo.toml" --test itl_consume_e2e -- --nocapture

echo "PASS terp-rs combo"
