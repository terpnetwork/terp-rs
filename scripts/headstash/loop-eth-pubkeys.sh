#!/usr/bin/env bash
# Repeat scrape until two consecutive passes add fewer than MIN_OK keys
# (or query remaining is 0). Resume-safe.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MAP="$ROOT/scripts/scripts-data/address_pubkey_map.csv"
MAX="${ETH_SCRAPE_LOOPS:-12}"
MIN_OK="${ETH_SCRAPE_MIN_OK:-20}"
stall=0
prev=$(wc -l < "$MAP" | tr -d ' ')
cd "$ROOT"
for i in $(seq 1 "$MAX"); do
  echo "======== loop $i/$MAX (map_rows=$prev) ========"
  ETH_RPC_MODE="${ETH_RPC_MODE:-public}" \
    ETH_SCRAPE_CONCURRENCY="${ETH_SCRAPE_CONCURRENCY:-16}" \
    ETH_RPC_BATCH="${ETH_RPC_BATCH:-20}" \
    ETH_RPC_PARALLEL="${ETH_RPC_PARALLEL:-8}" \
    cargo run -p headstash-eth-pubkeys --release
  now=$(wc -l < "$MAP" | tr -d ' ')
  gained=$((now - prev))
  echo "======== loop $i gained=$gained map_rows=$now ========"
  if [[ "$gained" -lt "$MIN_OK" ]]; then
    stall=$((stall + 1))
  else
    stall=0
  fi
  prev=$now
  if [[ "$stall" -ge 2 ]]; then
    echo "plateau: two passes with gained < $MIN_OK — stop"
    break
  fi
done
echo "done map_rows=$prev"
