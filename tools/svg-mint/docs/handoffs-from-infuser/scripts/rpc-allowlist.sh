#!/bin/sh
# Shared allowlist for DREGG watch RPCs. Source or: rpc-allowlist.sh check URL
# Exit 0 = self-hosted / lab. Exit 1 = public SaaS leak.
# Extra hosts: DREGG_RPC_ALLOWLIST=host1,host2

rpc_url_is_self_hosted() {
  url="$1"
  low=$(printf '%s' "$url" | tr 'A-Z' 'a-z')
  case "$low" in
    *alchemy.com*|*infura.io*|*publicnode.com*|*blastapi.io*|*ankr.com*|*llamarpc.com*)
      return 1
      ;;
  esac
  host=$(python3 -c "
import sys
from urllib.parse import urlparse
u=sys.argv[1].strip()
if '://' not in u:
    u='http://'+u
p=urlparse(u)
h=(p.hostname or '').lower()
print(h)
" "$url" 2>/dev/null || echo "")
  [ -n "$host" ] || return 1
  case "$host" in
    127.0.0.1|localhost|::1|anvil|bitcoind|geth|reth|base-el|hardhat|host.docker.internal) return 0 ;;
    192.168.10.*) return 0 ;;
    10.*) return 0 ;;
    172.1[6-9].*|172.2[0-9].*|172.3[0-1].*) return 0 ;;
  esac
  extra="${DREGG_RPC_ALLOWLIST:-}"
  oldifs=$IFS
  IFS=,
  for h in $extra; do
    h=$(printf '%s' "$h" | tr 'A-Z' 'a-z' | sed 's/^ *//;s/ *$//')
    [ -n "$h" ] || continue
    if [ "$host" = "$h" ]; then
      IFS=$oldifs
      return 0
    fi
  done
  IFS=$oldifs
  return 1
}

# Only when executed, not sourced (sourcing inherits $1 from the caller).
case "${0##*/}" in
  rpc-allowlist.sh)
    if [ "${1:-}" = "check" ]; then
      rpc_url_is_self_hosted "${2:-}"
      exit $?
    fi
    ;;
esac
