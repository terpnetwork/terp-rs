#!/usr/bin/env bash
# Sketch recipe for oracle-priced NFT mint demo (testnet/local).
# Does not deploy a custom contract yet — orchestrates support tools + prints txs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=../../lib.sh
source "${ROOT}/lib.sh"

MEMBERS="${1:-$ROOT/whitelist/fixtures/members.json}"
OUT="${2:-/tmp/hm-nft-demo}"
mkdir -p "$OUT"

hm_log "=== nft-oracle-mint recipe ==="
hm_info "members=$MEMBERS out=$OUT"

python3 "$ROOT/whitelist/build.py" --in "$MEMBERS" --out "$OUT/tree"
hm_ok "whitelist root=$(cat "$OUT/tree/root.txt")"

# Optional: dry-run feegrant plan
bash "$ROOT/accounts/feegrant-batch.sh" --file "$MEMBERS" --granter ops --dry-run || true

cat > "$OUT/NEXT_STEPS.md" <<EOF
# Next steps (chain-connected)

1. Resolve real addresses:
   hm-support accounts bootstrap --file $MEMBERS --out $OUT/members.resolved.json

2. Rebuild tree from resolved addresses:
   hm-support whitelist build --in $OUT/members.resolved.json --out $OUT/tree

3. Feegrant:
   hm-support accounts feegrant --granter <ops> --file $OUT/members.resolved.json

4. Deploy CW721 + minter; set whitelist root:
   root=$(cat "$OUT/tree/root.txt")

5. Price from oracle (Path B):
   - Ensure HASHMERCHANT_SIDECAR_URL and register-contract if using sudo roots
   - Query mid / set unit_price with bps band
   - NEVER use oracle to bump max supply

6. Mint as eligible member with --fee-granter and proof from:
   $OUT/tree/proofs/<address>.json

Hard rule: oracle prices; whitelist gates; feegrant sponsors gas only.
EOF

hm_ok "wrote $OUT/NEXT_STEPS.md"
hm_log "recipe offline portion complete"
