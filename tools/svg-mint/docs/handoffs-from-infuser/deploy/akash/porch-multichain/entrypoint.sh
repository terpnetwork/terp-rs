#!/bin/sh
# One process tree, three P2P daemons. No public JSON-RPC.
set -eu
DATA="${DATA:-/data}"
mkdir -p "$DATA/btc" "$DATA/eth" "$DATA/base"

bitcoind \
  -datadir="$DATA/btc" \
  -listen=1 \
  -bind=0.0.0.0 \
  -server=0 \
  -disablewallet \
  -prune=550 \
  -txindex=0 \
  -printtoconsole=1 &
BTC_PID=$!

geth \
  --mainnet \
  --syncmode=snap \
  --gcmode=full \
  --state.scheme=path \
  --datadir="$DATA/eth" \
  --port=30303 \
  --discovery.port=30303 \
  --maxpeers=40 \
  --http=false \
  --ws=false \
  --nodekey=/etc/porch/eth.nodekey \
  --nat=none &
ETH_PID=$!

op-geth \
  --op-network=base-mainnet \
  --syncmode=snap \
  --gcmode=full \
  --datadir="$DATA/base" \
  --port=30304 \
  --discovery.port=30304 \
  --maxpeers=40 \
  --http=false \
  --ws=false \
  --authrpc.addr=127.0.0.1 \
  --authrpc.port=8552 \
  --authrpc.vhosts=localhost \
  --nodekey=/etc/porch/base.nodekey \
  --nat=none \
  --rollup.sequencerhttp=https://mainnet-sequencer.base.org \
  --rollup.disabletxpoolgossip=true &
BASE_PID=$!

term() {
  kill -TERM "$BTC_PID" "$ETH_PID" "$BASE_PID" 2>/dev/null || true
  wait || true
}
trap term INT TERM

while kill -0 "$BTC_PID" 2>/dev/null \
  && kill -0 "$ETH_PID" 2>/dev/null \
  && kill -0 "$BASE_PID" 2>/dev/null; do
  sleep 5
done
term
exit 1
