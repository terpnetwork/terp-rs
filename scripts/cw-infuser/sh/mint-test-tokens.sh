#!/bin/sh
# mint-test-tokens.sh
#
# Creates and mints tokenfactory test tokens for shitstrap integration testing.
# Run this BEFORE the integration deployment script so the denom format
# (factory/<creator>/<subdenom>) is known and funded prior to contract instantiation.
#
# Usage:
#   KEY=mykey CHAIN_ID=morocco-1 NODE=http://localhost:26657 sh mint-test-tokens.sh
#
# Required env vars:
#   KEY        — terpd key name (from `terpd keys list`)
#   CHAIN_ID   — e.g. morocco-1 or localterp
#   NODE       — RPC endpoint, e.g. http://localhost:26657
#
# Optional:
#   FEES       — tx fee, default 1000000uterp
#   MINT_AMOUNT — base units to mint per denom, default 1000000000000 (1M tokens at 6dp)

set -e

: "${KEY:?KEY is required}"
: "${CHAIN_ID:?CHAIN_ID is required}"
: "${NODE:?NODE is required}"

FEES="${FEES:-1000000uterp}"
MINT_AMOUNT="${MINT_AMOUNT:-1000000000000}"

CREATOR=$(terpd keys show "$KEY" -a)

SUBDENOMS="atom btc akt um bcna monero eth zec"

TX_FLAGS="--from $KEY --chain-id $CHAIN_ID --node $NODE --fees $FEES --yes --broadcast-mode sync"

echo "Creator address : $CREATOR"
echo "Chain           : $CHAIN_ID"
echo "Node            : $NODE"
echo "Mint amount     : $MINT_AMOUNT base units per denom"
echo ""

# ── Create denoms ─────────────────────────────────────────────────────────────
echo "=== Creating tokenfactory denoms ==="
for subdenom in $SUBDENOMS; do
    echo "Creating factory/$CREATOR/$subdenom ..."
    terpd tx tokenfactory create-denom "$subdenom" $TX_FLAGS
    sleep 6
done

# ── Mint supply ───────────────────────────────────────────────────────────────
echo ""
echo "=== Minting supply ==="
for subdenom in $SUBDENOMS; do
    denom="factory/$CREATOR/$subdenom"
    echo "Minting ${MINT_AMOUNT}${denom} ..."
    terpd tx tokenfactory mint "${MINT_AMOUNT}${denom}" $TX_FLAGS
    sleep 6
done

# ── Print resulting denoms for reference ──────────────────────────────────────
echo ""
echo "=== Tokenfactory denoms ready ==="
for subdenom in $SUBDENOMS; do
    echo "  factory/$CREATOR/$subdenom"
done
echo ""
echo "Set CREATOR=$CREATOR in your .env before running the integration script."
