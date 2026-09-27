#!/bin/sh
# Write runtime → temp HashMerchant authz msgs (cw-orch broadcasts them).
# Does not invent keys. Does not use wallet-as-runtime.
#
#   ONBOARD_RUNTIME=terp1... ONBOARD_HASHMERCHANT=terp1... \
#     ./scripts/bootstrap-runtime-to-hashmerchant.sh
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${ONBOARD_AUTHZ_OUT:-$HOME/.terp/hashmerchant-authz}"
RT="${ONBOARD_RUNTIME:-}"
HM="${ONBOARD_HASHMERCHANT:-}"
CHAIN="${ONBOARD_CHAIN:-morocco-1}"
if [ -z "$RT" ] || [ -z "$HM" ]; then
  echo "set ONBOARD_RUNTIME (long-lived authz granter) and ONBOARD_HASHMERCHANT (temp fee.granter)" >&2
  exit 2
fi
if [ "$RT" = "$HM" ]; then
  echo "collapsed keys: runtime must not equal hashmerchant" >&2
  exit 3
fi
mkdir -p "$OUT"
chmod 700 "$OUT"
cd "$ROOT"
cargo run --quiet --bin prepare-authz -- \
  --mode wallet-proxy \
  --preset register \
  --chain-id "$CHAIN" \
  --runtime "$RT" \
  --hashmerchant "$HM" \
  --from-addr "$RT" \
  --out "$OUT"
echo "wrote $OUT"
echo "cw-orch grant (derives bech32 from ~/.terp/*.mnemonic):"
echo "  cargo run --example hashmerchant_authz_bootstrap -p ict-rs-cw-orch"
echo "  AUTHZ_BROADCAST=1 cargo run --example hashmerchant_authz_bootstrap -p ict-rs-cw-orch"
echo "then HashMerchant env:"
echo "  ONBOARD_RUNTIME=$RT"
echo "  ONBOARD_HASHMERCHANT=$HM"
echo "  ONBOARD_CHAIN=$CHAIN"
echo "  ONBOARD_FROM=<key name of $HM>"
echo "  ONBOARD_BROADCAST=1"
echo "  # do not set ONBOARD_ALLOW_COLLAPSE"
