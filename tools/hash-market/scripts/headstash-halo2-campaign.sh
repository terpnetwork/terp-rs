#!/bin/bash
# Instantiate cw-headstash on a loopback terpd and set one Halo2 campaign.
# wB:p3 runs this against the 4-validator ict-rs chain. This script does
# not start Docker, does not print keys, wavs, or the vk hash, and refuses
# any node that is not 127.0.0.1. It also refuses the live epoch chain
# (chain id headstash-epoch-e2e, port 36667) unless ALLOW_EPOCH_CHAIN=1.
#
# Required:
#   NODE=tcp://127.0.0.1:PORT
#   CHAIN_ID=headstash-fm-1
#   KEY=key name inside HOME_CHAIN
#   HOME_CHAIN=directory that holds the test keyring
#   WAVS=path to wavs.json (read, never printed)
#   ALLOCATION_ROOT_HEX=64 hex   genesis root and campaign allocation root
#   DENOM_HEX=64 hex             existing-fungible proof bytes, raw denom uterp
#   TRUSTED_HEX=64 hex           32-byte ed25519 verifying key
# Optional:
#   CTX_HEX=64 hex               default 64 zeros
#   CUTOFF=80                    contract cutoff height
#   AMOUNT=15                    payout amount, base units
#   CAP=1
#   MAX_ROWS=4
#   ESCROW=1000000uterp          coins sent at instantiate
#   REGISTER_FUNDS=1000000uterp  hashmerchant register-contract deposit
#   LABEL=headstash-halo2
#   TERPD=.../build/terpd-v6.4.0-darwin-arm64
#   WASM=/tmp/headstash-epoch-chain/cw_headstash.wasm
#   KEYRING_BACKEND=test
#   GAS_PRICES=0.025uterp
#   VK_HASH_B64=standard base64 of the 32-byte epoch blob hash
#   SRC_NODE=tcp://127.0.0.1:36667
#   SRC_CONTRACT=terp1...        used when VK_HASH_B64 is unset
#   WORK=directory for the JSON files (mode 700)
#   DRY=1                        resolve the hash and stop before any tx
#
# cw-orch: ExecuteMsg::SetEpochCampaign is the same write when the contract
# is built with feature interface. This script is the terpd step.
#
# Replay, after an accepted root is already stored and the first
# ApplyAcceptedBatch of a row has paid: send that same
# apply_accepted_batch again. The contract returns
# "nullifier already exists". One staged batch that repeats a nullifier
# returns "duplicate nullifier in batch".
# Unclaimed: there is no sweep message. query bank balances of the
# contract still shows the escrow. A new drop is a new campaign.

set -euo pipefail

export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"

TERPD="${TERPD:-/Users/returniflost/abstract/terp-core/build/terpd-v6.4.0-darwin-arm64}"
WASM="${WASM:-/tmp/headstash-epoch-chain/cw_headstash.wasm}"
NODE="${NODE:-}"
CHAIN_ID="${CHAIN_ID:-}"
KEY="${KEY:-}"
HOME_CHAIN="${HOME_CHAIN:-}"
WAVS="${WAVS:-}"
ALLOCATION_ROOT_HEX="${ALLOCATION_ROOT_HEX:-}"
DENOM_HEX="${DENOM_HEX:-}"
TRUSTED_HEX="${TRUSTED_HEX:-}"
CTX_HEX="${CTX_HEX:-0000000000000000000000000000000000000000000000000000000000000000}"
CUTOFF="${CUTOFF:-80}"
AMOUNT="${AMOUNT:-15}"
CAP="${CAP:-1}"
MAX_ROWS="${MAX_ROWS:-4}"
ESCROW="${ESCROW:-1000000uterp}"
REGISTER_FUNDS="${REGISTER_FUNDS:-1000000uterp}"
LABEL="${LABEL:-headstash-halo2}"
KEYRING_BACKEND="${KEYRING_BACKEND:-test}"
GAS_PRICES="${GAS_PRICES:-0.025uterp}"
VK_HASH_B64="${VK_HASH_B64:-}"
SRC_NODE="${SRC_NODE:-tcp://127.0.0.1:36667}"
SRC_CONTRACT="${SRC_CONTRACT:-terp1eyfccmjm6732k7wp4p6gdjwhxjwsvje44j0hfx8nkgrm8fs7vqfs90760x}"
DRY="${DRY:-0}"
ALLOW_EPOCH_CHAIN="${ALLOW_EPOCH_CHAIN:-0}"

loopback() {
  case "$1" in
    tcp://127.0.0.1:[0-9]*|http://127.0.0.1:[0-9]*)
      case "$1" in
        *0.0.0.0*|*\[::\]*) return 1 ;;
      esac
      ;;
    *) return 1 ;;
  esac
  local port="${1##*:}"
  case "$port" in
    ''|*[!0-9]*) return 1 ;;
  esac
  return 0
}

if ! loopback "$NODE"; then
  echo "refusing node" >&2
  exit 1
fi
if ! loopback "$SRC_NODE"; then
  echo "refusing source node" >&2
  exit 1
fi
case "$NODE" in
  *0.0.0.0*|*\[::\]*) echo "refusing bind" >&2; exit 1 ;;
esac
if [ "$ALLOW_EPOCH_CHAIN" != "1" ]; then
  if [ "$CHAIN_ID" = "headstash-epoch-e2e" ]; then
    echo "refusing epoch chain id" >&2
    exit 1
  fi
  case "$NODE" in
    *:36667) echo "refusing epoch chain port" >&2; exit 1 ;;
  esac
fi
if [ -z "$CHAIN_ID" ] || [ -z "$KEY" ] || [ -z "$HOME_CHAIN" ] || [ -z "$WAVS" ]; then
  echo "missing CHAIN_ID KEY HOME_CHAIN or WAVS" >&2
  exit 1
fi
if [ ! -x "$TERPD" ]; then
  echo "terpd missing" >&2
  exit 1
fi
if [ ! -s "$WASM" ]; then
  echo "wasm missing" >&2
  exit 1
fi
if [ ! -s "$WAVS" ]; then
  echo "wavs missing" >&2
  exit 1
fi
if [ ! -d "$HOME_CHAIN" ]; then
  echo "home missing" >&2
  exit 1
fi

if [ -z "${WORK:-}" ]; then
  WORK=$(mktemp -d /tmp/headstash-halo2-campaign.XXXXXX)
fi
mkdir -p "$WORK"
chmod 700 "$WORK"

HASH_FILE="$WORK/vk.hash"
chmod 700 "$WORK"
if [ -n "$VK_HASH_B64" ]; then
  printf '%s' "$VK_HASH_B64" > "$HASH_FILE"
else
  "$TERPD" query wasm contract-state smart "$SRC_CONTRACT" '{"epoch_campaign":{}}' \
    --node "$SRC_NODE" -o json > "$WORK/src-campaign.json"
  python3 - "$WORK/src-campaign.json" "$HASH_FILE" << 'PY'
import json, sys
src, dest = sys.argv[1], sys.argv[2]
raw = json.load(open(src))
data = raw.get("data", raw)
if data.get("vk_id") != 1 or data.get("backend") != "halo2":
    raise SystemExit("source campaign is not vk 1 halo2")
h = data.get("vk_hash")
if not isinstance(h, str) or len(h) < 4:
    raise SystemExit("source vk hash missing")
open(dest, "w").write(h)
PY
  rm -f "$WORK/src-campaign.json"
fi
chmod 600 "$HASH_FILE"

export ALLOCATION_ROOT_HEX DENOM_HEX TRUSTED_HEX CTX_HEX CUTOFF AMOUNT CAP MAX_ROWS LABEL
python3 - "$HASH_FILE" "$WORK" "$WAVS" << 'PY'
import base64, json, os, pathlib, sys
hash_file, work, wavs_path = sys.argv[1:]
work = pathlib.Path(work)
blob = pathlib.Path(hash_file).read_text().strip()
raw = base64.b64decode(blob, validate=True)
if len(raw) != 32:
    raise SystemExit("vk hash must be 32 bytes")
vk = base64.b64encode(raw).decode()
pathlib.Path(hash_file).write_text(vk)

def b64hex(name):
    text = os.environ[name].strip().lower()
    if len(text) != 64 or any(c not in "0123456789abcdef" for c in text):
        raise SystemExit(name + " must be 64 hex")
    return base64.b64encode(bytes.fromhex(text)).decode()

def num(name):
    text = os.environ[name].strip()
    if not text.isdigit() or int(text) == 0:
        raise SystemExit(name + " must be a positive integer")
    return int(text)

alloc = b64hex("ALLOCATION_ROOT_HEX")
denom = b64hex("DENOM_HEX")
trusted = b64hex("TRUSTED_HEX")
ctx = b64hex("CTX_HEX")
label = os.environ["LABEL"]
if not label or any(ch in label for ch in "\"\n\r"):
    raise SystemExit("label")
wavs = json.loads(pathlib.Path(wavs_path).read_text())
init = {
    "genesis_root": alloc,
    "genesis_label": label,
    "token_strategy": {"existing_fungible": {"proof": denom, "raw": "uterp"}},
    "wavs": wavs,
}
campaign = {
    "set_epoch_campaign": {
        "campaign": {
            "ctx": ctx,
            "backend": "halo2",
            "cutoff_height": num("CUTOFF"),
            "bech32_hrp": "terp",
            "denom": denom,
            "amounts": [{"amount": num("AMOUNT"), "cap": num("CAP")}],
            "reserved": [],
            "trusted": [trusted],
            "trusted_verify_threshold": 1,
            "trusted_sign_threshold": 1,
            "max_rows": num("MAX_ROWS"),
            "allocation_root": alloc,
            "root_id": 0,
            "vk_id": 1,
            "vk_hash": vk,
        }
    }
}
for name, body in (("instantiate.json", init), ("campaign.json", campaign)):
    path = work / name
    path.write_text(json.dumps(body, separators=(",", ":")))
    path.chmod(0o600)
print("vk_hash_bytes", len(raw))
PY

if [ "$DRY" = "1" ]; then
  echo "wasm_bytes $(wc -c < "$WASM" | tr -d ' ')"
  echo "vk_id 1"
  echo "dry ok"
  echo "work $WORK"
  exit 0
fi

OWNER=$("$TERPD" keys show "$KEY" -a --keyring-backend "$KEYRING_BACKEND" --home "$HOME_CHAIN")
case "$OWNER" in
  terp1*) ;;
  *) echo "owner address missing" >&2; exit 1 ;;
esac

tx_code_file() {
  local gas="$1" dest="$2"
  shift 2
  local out="$WORK/last-tx.json"
  "$TERPD" "$@" \
    --from "$KEY" \
    --home "$HOME_CHAIN" \
    --keyring-backend "$KEYRING_BACKEND" \
    --chain-id "$CHAIN_ID" \
    --node "$NODE" \
    --gas "$gas" \
    --gas-prices "$GAS_PRICES" \
    -y -b sync -o json > "$out"
  chmod 600 "$out"
  local hash
  hash=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("txhash",""))' "$out")
  case "$hash" in
    '') echo "no txhash" >&2; exit 1 ;;
  esac
  local i=0
  while [ "$i" -lt 60 ]; do
    if "$TERPD" query tx "$hash" --node "$NODE" -o json > "$dest" 2>/dev/null; then
      chmod 600 "$dest"
      if python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); raise SystemExit(0 if "code" in d or "txhash" in d else 1)' "$dest"; then
        rm -f "$out"
        return 0
      fi
    fi
    i=$((i + 1))
    sleep 2
  done
  echo "tx not committed" >&2
  exit 1
}

require_code() {
  local step="$1" file="$2"
  python3 - "$step" "$file" << 'PY'
import json, sys
step, path = sys.argv[1:]
d = json.load(open(path))
code = d.get("code", 1)
if code != 0:
    log = str(d.get("raw_log", ""))[:160]
    low = log.lower()
    if any(word in low for word in ("mnemonic", "seed", "private", ".env")):
        log = "hidden"
    print(f"{step} failed code {code} {log}", file=sys.stderr)
    raise SystemExit(1)
PY
}

attr_of() {
  python3 - "$1" "$2" << 'PY'
import json, sys
key, path = sys.argv[1:]
def walk(node):
    if isinstance(node, dict):
        if node.get("key") == key and isinstance(node.get("value"), str):
            return node["value"]
        for child in node.values():
            found = walk(child)
            if found:
                return found
    elif isinstance(node, list):
        for child in node:
            found = walk(child)
            if found:
                return found
    return None
found = walk(json.load(open(path)))
if not found:
    raise SystemExit(1)
print(found)
PY
}

echo "wasm_bytes $(wc -c < "$WASM" | tr -d ' ')"
tx_code_file 250000000 "$WORK/store.json" tx wasm store "$WASM"
require_code store "$WORK/store.json"
CODE_ID=$(attr_of code_id "$WORK/store.json")
echo "code_id $CODE_ID"
rm -f "$WORK/store.json"

INIT_MSG=$(cat "$WORK/instantiate.json")
tx_code_file 80000000 "$WORK/instantiate-tx.json" \
  tx wasm instantiate "$CODE_ID" "$INIT_MSG" \
  --label "$LABEL" --admin "$OWNER" --amount "$ESCROW"
require_code instantiate "$WORK/instantiate-tx.json"
CONTRACT=$(attr_of _contract_address "$WORK/instantiate-tx.json")
case "$CONTRACT" in
  terp1*) ;;
  *) echo "contract missing" >&2; exit 1 ;;
esac
echo "contract $CONTRACT"
rm -f "$WORK/instantiate-tx.json"

CAMP_MSG=$(cat "$WORK/campaign.json")
tx_code_file 5000000 "$WORK/campaign-tx.json" \
  tx wasm execute "$CONTRACT" "$CAMP_MSG"
require_code campaign "$WORK/campaign-tx.json"
rm -f "$WORK/campaign-tx.json"
echo "campaign_set ok"

tx_code_file 3000000 "$WORK/register-tx.json" \
  tx hashmerchant register-contract "$CONTRACT" headstash-claim bank "$REGISTER_FUNDS"
require_code register "$WORK/register-tx.json"
rm -f "$WORK/register-tx.json"
echo "registered ok"
echo "escrow $ESCROW"
echo "vk_id 1"
echo "work $WORK"
echo "replay: second ApplyAcceptedBatch of a paid row returns nullifier already exists"
echo "unclaimed: no sweep message; bank balances of the contract keep the escrow"
