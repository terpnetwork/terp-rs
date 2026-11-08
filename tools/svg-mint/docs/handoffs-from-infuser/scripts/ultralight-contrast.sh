#!/bin/sh
# Contrast teammate ultra-light lock vs repo files. Prints one JSON object.
# ok=true means topology matches (house prune, porch P2P-only, allowlist).
# pending_ttl_shipped=true after DREGG_PENDING_TTL_SECS landed.
set -eu
ROOT="${TERP_CORE:-${HOME:-/Users/returniflost}/abstract/terp-core/crates}"
MINT="$ROOT/dregg/svg-mint"
HOUSE="$MINT/deploy/house/docker-compose.yml"
PORCH="$MINT/deploy/akash/porch-multichain/deploy-porch.yaml"
ALLOW="$MINT/src/rpc_allowlist.rs"
FED="$MINT/src/bin/svg-mint-fed.rs"
CHECKOUT="$MINT/src/checkout.rs"

house_prune=false
house_loopback=false
porch_p2p_only=false
allowlist=false
mint_timeout=false
listsince=false
pending_ttl=false
import_hook=false
electrs=false
ok=true

if [ -f "$HOUSE" ] && grep -q 'prune=550' "$HOUSE"; then house_prune=true; fi
if [ -f "$HOUSE" ] && grep -q '127.0.0.1:8332' "$HOUSE"; then house_loopback=true; fi
if [ -f "$HOUSE" ] && grep -qi electrs "$HOUSE"; then electrs=true; fi

porch_rpc_leak=false
if [ -f "$PORCH" ]; then
  if grep -E 'as:[[:space:]]*(8332|8545|8546|9067)([[:space:]]|$)' "$PORCH" >/dev/null; then
    porch_rpc_leak=true
  fi
  if grep -q 'as: 8333' "$PORCH" && [ "$porch_rpc_leak" = false ]; then
    porch_p2p_only=true
  fi
fi

if [ -f "$ALLOW" ] && grep -q 'alchemy.com' "$ALLOW" && grep -q '127.0.0.1' "$ALLOW"; then
  allowlist=true
fi
if [ -f "$CHECKOUT" ] && grep -q 'DREGG_MINT_TIMEOUT_SECS' "$CHECKOUT"; then
  mint_timeout=true
fi
if [ -f "$FED" ] && grep -q 'listsinceblock' "$FED"; then
  listsince=true
fi
# Unpaid-cart TTL would be a dedicated env / turn (not mint-after-entitle timeout).
if grep -R -n 'DREGG_PENDING_TTL\|pending_ttl_secs\|abandon_pending\|drop.*watch-only' \
  "$MINT/src" "$MINT/tests" 2>/dev/null | grep -v 'PORCH-' >/dev/null; then
  pending_ttl=true
fi
if grep -q 'importaddress\|importmulti' "$FED" 2>/dev/null; then
  import_hook=true
fi

if [ "$house_prune" != true ] || [ "$house_loopback" != true ]; then ok=false; fi
if [ "$porch_p2p_only" != true ]; then ok=false; fi
if [ "$allowlist" != true ]; then ok=false; fi
if [ "$mint_timeout" != true ]; then ok=false; fi
if [ "$listsince" != true ]; then ok=false; fi
if [ "$electrs" = true ]; then ok=false; fi

# JSON booleans (no trailing comma issues)
printf '%s' '{'
printf '"ok":%s,' "$ok"
printf '"house_prune550":%s,' "$house_prune"
printf '"house_loopback_rpc":%s,' "$house_loopback"
printf '"porch_p2p_only":%s,' "$porch_p2p_only"
printf '"allowlist":%s,' "$allowlist"
printf '"mint_timeout_shipped":%s,' "$mint_timeout"
printf '"listsinceblock":%s,' "$listsince"
printf '"pending_ttl_shipped":%s,' "$pending_ttl"
printf '"importaddress_in_fed":%s,' "$import_hook"
printf '"electrs_in_house":%s,' "$electrs"
printf '"dregg_queries_akash":false,'
printf '"public_saas_allowed":false'
printf '}\n'

if [ "$ok" = true ]; then
  exit 0
fi
exit 1
