#!/bin/bash
# Local headstash epoch e2e. Loopback terpd only. No image pulls, no tunnels.
# Key material stays in the chain home. This script prints addresses, not seeds.
set -euo pipefail

export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"

WT="${HOME}/abstract/terp-core/crates/terp-rs-headstash-epoch"
DAO="${HOME}/abstract/terp-core/crates/dao-contracts/Cargo.toml"
TERPD="/Users/returniflost/abstract/terp-core/build/terpd-v6.4.0-darwin-arm64"
CHAIN_HOME="/tmp/headstash-epoch-chain"
CHAIN_ID="headstash-epoch-e2e"
NODE="tcp://127.0.0.1:36667"
RPC="http://127.0.0.1:36667"
NIPS_LINE='cw721-nips = { path = "../terp-rs/crates/nips" }'
NIPS_REDIRECT='cw721-nips = { path = "../terp-rs-headstash-epoch/crates/nips" }'

case "$WT" in
  *terp-rs-headstash-epoch) ;;
  *) echo "refusing worktree ${WT}" >&2; exit 1 ;;
esac
test -x "$TERPD"
grep -F "$NIPS_LINE" "$DAO" >/dev/null

free_kb() { df -k / | awk 'NR==2 { print $4 }'; }
if [ "$(free_kb)" -lt 819200 ]; then
  echo "refusing to build with $(free_kb) KB free" >&2
  exit 1
fi

DAO_BACKUP="/tmp/dao-contracts-Cargo.toml.step4"
LOCK_BACKUP="/tmp/terp-rs-Cargo.lock.step4e2e"
cp "$DAO" "$DAO_BACKUP"
cp "$WT/Cargo.lock" "$LOCK_BACKUP"
restore_tree() {
  cp "$DAO_BACKUP" "$DAO"
  if ! cmp -s "$WT/Cargo.lock" "$LOCK_BACKUP"; then
    cp "$LOCK_BACKUP" "$WT/Cargo.lock"
    echo "restored worktree Cargo.lock"
  fi
}
trap restore_tree EXIT

mkdir -p "$CHAIN_HOME"
chmod 700 "$CHAIN_HOME"
# Previous run logs stay beside the home. The new files must not inherit a halt.
if [ -f "$CHAIN_HOME/e2e.log" ]; then
  mv "$CHAIN_HOME/e2e.log" "$CHAIN_HOME/e2e.log.prev"
fi

# The host debug artifacts are this worktree's contract tests. The shared
# terp-rs target is not touched. Wasm needs the room.
if [ ! -f "$CHAIN_HOME/cw_headstash.wasm" ]; then
  echo "building cw-headstash wasm"
  python3 - << PY
from pathlib import Path
p = Path("$DAO")
text = p.read_text()
old = '''$NIPS_LINE'''
new = '''$NIPS_REDIRECT'''
if old not in text:
    raise SystemExit("nips line missing")
p.write_text(text.replace(old, new, 1))
PY
  # This worktree's headstash debug is rebuildable. Release stays for the bench.
  HS_DEBUG="$WT/tools/headstash/target/debug"
  case "$HS_DEBUG" in
    *terp-rs-headstash-epoch/tools/headstash/target/debug)
      if [ "$(free_kb)" -lt 4500000 ]; then
        rm -rf "$HS_DEBUG"
        echo "removed worktree headstash debug, free $(free_kb) KB"
      fi
      ;;
  esac
  # An earlier failed redirect left a zero-byte file. That is not a proof.
  if [ -f "$CHAIN_HOME/wavs.json" ] && [ ! -s "$CHAIN_HOME/wavs.json" ]; then
    rm -f "$CHAIN_HOME/wavs.json"
  fi
  # Print the public WAVS proof while the host debug artifacts still exist.
  # The helper is cfg-gated on the interface feature.
  if [ ! -s "$CHAIN_HOME/wavs.json" ]; then
    (
      cd "$WT"
      CARGO_BUILD_JOBS=1 cargo run -p cw-headstash --offline --features interface \
        --example print_wavs --quiet \
        >"$CHAIN_HOME/wavs.json" 2>"$CHAIN_HOME/print_wavs.err"
    )
    python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$CHAIN_HOME/wavs.json"
    echo "wavs json bytes $(wc -c < "$CHAIN_HOME/wavs.json")"
  fi
  case "$WT/target/debug" in
    *terp-rs-headstash-epoch/target/debug) rm -rf "$WT/target/debug" ;;
    *) echo "refusing to delete $WT/target/debug" >&2; exit 1 ;;
  esac
  if [ "$(free_kb)" -lt 2500000 ]; then
    echo "refusing wasm build with $(free_kb) KB free" >&2
    exit 1
  fi
  (
    cd "$WT"
    CARGO_BUILD_JOBS=1 cargo build -p cw-headstash --release \
      --target wasm32-unknown-unknown --offline --locked --lib
  )
  RAW="$WT/target/wasm32-unknown-unknown/release/cw_headstash.wasm"
  test -f "$RAW"
  # -Oz lifts one function from 93 locals to 107. The chain rejects more than 100.
  wasm-opt -O1 --enable-bulk-memory --strip-debug -o "$CHAIN_HOME/cw_headstash.wasm" "$RAW"
  echo "wasm bytes $(wc -c < "$CHAIN_HOME/cw_headstash.wasm")"
  restore_tree
fi

grep -F "$NIPS_LINE" "$DAO" >/dev/null

stop_halted_epoch() {
  local pid cmd
  while read -r pid cmd; do
    case "$cmd" in
      *preview-chain*) continue ;;
    esac
    case "$cmd" in
      *terpd*headstash-epoch-chain*)
        if ! grep -q "CONSENSUS FAILURE" "$CHAIN_HOME/terpd.log" 2>/dev/null; then
          echo "epoch terpd $pid is up and its log has no consensus failure; leaving it" >&2
          return 0
        fi
        echo "stopping halted epoch terpd pid $pid"
        kill "$pid"
        local i
        for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
          if ! kill -0 "$pid" 2>/dev/null; then
            break
          fi
          sleep 1
        done
        if kill -0 "$pid" 2>/dev/null; then
          echo "epoch terpd $pid did not exit" >&2
          exit 1
        fi
        ;;
    esac
  done < <(ps -axww -o pid=,command=)
  local i
  for i in $(seq 1 20); do
    if ! curl -fsS --max-time 1 "$RPC/status" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "epoch rpc still answering after stop" >&2
  exit 1
}

stop_halted_epoch

if ! curl -fsS --max-time 2 "$RPC/status" >/dev/null 2>&1; then
  echo "initializing ${CHAIN_ID}"
  rm -rf "$CHAIN_HOME/config" "$CHAIN_HOME/data" "$CHAIN_HOME/keyring-test"
  "$TERPD" init epoch-e2e --chain-id "$CHAIN_ID" --default-denom uterp --home "$CHAIN_HOME" \
    >/tmp/headstash-epoch-init.log 2>&1
  add_key() {
    local name="$1"
    "$TERPD" keys add "$name" --keyring-backend test --home "$CHAIN_HOME" --output json \
      >"$CHAIN_HOME/${name}.json" 2>"$CHAIN_HOME/${name}.err" </dev/null
    chmod 600 "$CHAIN_HOME/${name}.json" "$CHAIN_HOME/${name}.err"
    python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["address"])' \
      "$CHAIN_HOME/${name}.json"
  }
  OWNER_ADDR="$(add_key owner)"
  RECIPIENT_ADDR="$(add_key recipient)"
  STRANGER_ADDR="$(add_key stranger)"
  umask 077
  cat >"$CHAIN_HOME/addresses.env" << EOF
OWNER_ADDR=${OWNER_ADDR}
RECIPIENT_ADDR=${RECIPIENT_ADDR}
STRANGER_ADDR=${STRANGER_ADDR}
EOF
  chmod 600 "$CHAIN_HOME/addresses.env"
  "$TERPD" genesis add-genesis-account "$OWNER_ADDR" 100000000000uterp \
    --home "$CHAIN_HOME" --keyring-backend test >/tmp/headstash-epoch-genesis.log
  "$TERPD" genesis add-genesis-account "$RECIPIENT_ADDR" 1000000uterp \
    --home "$CHAIN_HOME" --keyring-backend test >>/tmp/headstash-epoch-genesis.log
  "$TERPD" genesis add-genesis-account "$STRANGER_ADDR" 100000000uterp \
    --home "$CHAIN_HOME" --keyring-backend test >>/tmp/headstash-epoch-genesis.log
  "$TERPD" genesis gentx owner 1000000uterp --chain-id "$CHAIN_ID" \
    --home "$CHAIN_HOME" --keyring-backend test >>/tmp/headstash-epoch-genesis.log 2>&1
  "$TERPD" genesis collect-gentxs --home "$CHAIN_HOME" >>/tmp/headstash-epoch-genesis.log 2>&1
  umask 077
  python3 -c 'import os,sys; from pathlib import Path; p=Path(sys.argv[1]); p.write_bytes(os.urandom(32)); os.chmod(p, 0o600)' "$CHAIN_HOME/custody.seed"
  CUSTODY_PUB_B64="$(python3 -c 'import base64,sys; from pathlib import Path; from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey; from cryptography.hazmat.primitives import serialization; seed=Path(sys.argv[1]).read_bytes();
assert len(seed)==32
key=Ed25519PrivateKey.from_private_bytes(seed)
pub=key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
sys.stdout.write(base64.b64encode(pub).decode())' "$CHAIN_HOME/custody.seed")"
  python3 "$WT/tools/hash-market/scripts/patch_epoch_genesis.py" "$CHAIN_HOME" "$CUSTODY_PUB_B64"
  "$TERPD" genesis validate --home "$CHAIN_HOME" >>/tmp/headstash-epoch-genesis.log 2>&1
  HASHMERCHANT_SIDECAR_URL="http://127.0.0.1:19080" \
    "$TERPD" start --home "$CHAIN_HOME" --log_level "*:info,x/hashmerchant:debug" \
    >"$CHAIN_HOME/terpd.log" 2>&1 &
  echo $! >"$CHAIN_HOME/terpd.pid"
  echo "terpd pid $(cat "$CHAIN_HOME/terpd.pid")"
fi

# shellcheck disable=SC1091
source "$CHAIN_HOME/addresses.env"
ready=0
for _ in $(seq 1 60); do
  if curl -fsS --max-time 2 "$RPC/status" -o "$CHAIN_HOME/status.json"; then
    if python3 -c 'import json,sys; h=int(json.load(open(sys.argv[1]))["result"]["sync_info"]["latest_block_height"]); raise SystemExit(0 if h >= 3 else 1)' "$CHAIN_HOME/status.json"; then
      echo "chain height ready"
      ready=1
      break
    fi
  fi
  sleep 2
done
if [ "$ready" != 1 ]; then
  echo "chain did not reach height 3" >&2
  exit 1
fi
python3 -c 'import json,sys; h=int(json.load(open(sys.argv[1]))["result"]["sync_info"]["latest_block_height"]); print("height", h)' "$CHAIN_HOME/status.json"
"$TERPD" q hashmerchant chains --node "$NODE" -o json >"$CHAIN_HOME/chains.json"
python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); src=d["chains"][0]["oracle_sources"][0]; auth=src["authenticator"]; assert src["source_id"]=="headstash-batch" and src["enabled"] is True; assert auth["algorithm"]=="ed25519" and auth["scope"]=="headstash-batch"; assert len(auth["pubkey"])>0; print("oracle source headstash-batch enabled ed25519")' "$CHAIN_HOME/chains.json"

unset HASHMERCHANT_ATTESTATIONS || true
export HASHMERCHANT_CHAIN_UID=headstash-claim
export TERPD CHAIN_HOME NODE CHAIN_ID
export WASM="$CHAIN_HOME/cw_headstash.wasm"
export WAVS="$CHAIN_HOME/wavs.json"
export HOME_CHAIN="$CHAIN_HOME"
export RECIPIENT_ADDR
export OWNER_KEY=owner
export STRANGER_KEY=stranger
export BIND=127.0.0.1:19080
export CUSTODY_SEED_FILE="$CHAIN_HOME/custody.seed"
if [ ! -s "$CUSTODY_SEED_FILE" ]; then
  echo "missing custody seed" >&2
  exit 1
fi

if [ "${SKIP_E2E:-0}" = 1 ]; then
  echo CHAIN_READY
  exit 0
fi
cd "$WT/tools/hash-market"
# The committed lock already contains the zk-headstash graph. Do not copy
# the pre-zk additive lock over it: --locked would then refuse to build.
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
case "$LOCK_SHA" in
  8d5f3ee3c10f7ad0428a131bdc58b3fb11f0e8515541dd92241a436804570064) ;;
  *) echo "refusing hash-market lock ${LOCK_SHA}" >&2; exit 1 ;;
esac
HALO2_LOCK="/tmp/hash-market-Cargo.lock.halo2"
cp Cargo.lock "$HALO2_LOCK"
restore_hm() {
  if ! cmp -s Cargo.lock "$HALO2_LOCK"; then
    echo "hash-market lock changed during e2e; restoring halo2 lock"
    cp "$HALO2_LOCK" Cargo.lock
  fi
}
trap 'restore_tree; restore_hm' EXIT
CARGO_BUILD_JOBS=2 cargo run --release --offline --locked --features headstash --bin headstash-epoch-e2e -- \
  >"$CHAIN_HOME/e2e.log" 2>&1
echo "e2e log $CHAIN_HOME/e2e.log"
tail -40 "$CHAIN_HOME/e2e.log"
