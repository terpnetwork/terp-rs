#!/bin/sh
# Confirm DREGG can observe HD/ephemeral dests on ETH, Base, BTC, ZEC, native.
# Run on groot2. Never prints mnemonics, HD seeds, or RPC passwords.
# Exit 0 only if rails_ok=true (ZEC may be skipped with PREFLIGHT_ALLOW_ZEC_SKIP=1).
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck disable=SC1091
. "$SCRIPT_DIR/rpc-allowlist.sh"

hn=$(hostname 2>/dev/null || echo "")
if [ "$hn" = "groot2" ] || [ "$hn" = "groot2.local" ]; then
  REQUIRE_SELF="${SVG_MINT_REQUIRE_SELF_HOSTED_RPC:-1}"
else
  REQUIRE_SELF="${SVG_MINT_REQUIRE_SELF_HOSTED_RPC:-0}"
fi

check_self_hosted() {
  name="$1"
  url="$2"
  [ "$REQUIRE_SELF" = "1" ] || return 0
  [ -n "$url" ] || return 0
  if rpc_url_is_self_hosted "$url"; then
    echo "$name allowlist ok"
    return 0
  fi
  add_blocker "$name is not self-hosted (house/lab). Public SaaS RPC refused under SVG_MINT_REQUIRE_SELF_HOSTED_RPC=1"
}

redact_url() {
  echo "$1" | sed -E 's#(://)[^/@]+@#\1***@#'
}

have() { command -v "$1" >/dev/null 2>&1; }

blockers=""
add_blocker() {
  blockers="${blockers}
- $1"
}

json_get() {
  # tiny python if jq missing
  if have jq; then
    echo "$1" | jq -r "$2" 2>/dev/null || echo ""
  else
    python3 -c "import json,sys; v=json.loads(sys.argv[1]); k=sys.argv[2].lstrip('.');
print(v.get(k,'' ) if '.' not in k else '')" "$1" "$2" 2>/dev/null || echo ""
  fi
}

# Load env files without printing them
if [ -f "${HOME}/.terp/onboard.env" ]; then
  set -a
  # shellcheck disable=SC1090
  . "${HOME}/.terp/onboard.env"
  set +a
fi
if [ -f "${HOME}/.terp/dregg-svg.env" ]; then
  set -a
  # shellcheck disable=SC1090
  . "${HOME}/.terp/dregg-svg.env"
  set +a
fi

# If the live fed is running, copy RPC names from its environment (values stay in this shell)
PID_FILE="${HOME}/.terp/svg-mint-fed.pid"
if [ -f "$PID_FILE" ]; then
  fed_pid=$(cat "$PID_FILE")
  if [ -n "$fed_pid" ] && [ -r "/proc/${fed_pid}/environ" ]; then
    eval "$(tr '\0' '\n' < "/proc/${fed_pid}/environ" | grep -E '^(ETH_RPC|BASE_RPC|BTC_RPC|ZCASH_DEVTOOL_|DREGG_|SVG_MINT_|ONBOARD_)=' | sed 's/^/export /' || true)"
  fi
fi

echo "REQUIRE_SELF_HOSTED_RPC=${REQUIRE_SELF}"
echo "=== DREGG HTTP ==="
DREGG_LOCAL="${DREGG_LOCAL:-http://127.0.0.1:8421}"
health=$(curl -sS -m 8 "${DREGG_LOCAL}/health" || echo "{}")
echo "health: ${health}"
svc=$(echo "$health" | python3 -c "import json,sys
try:
 d=json.load(sys.stdin); print(d.get('service',''))
except Exception:
 print('')")
if [ "$svc" != "dregg-svg" ]; then
  add_blocker "DREGG /health is not service=dregg-svg"
fi

RECIP="${PREFLIGHT_RECIPIENT:-terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052}"
pt=$(curl -sS -m 8 -w "\nHTTP:%{http_code}" "${DREGG_LOCAL}/v1/pay-targets?recipient=${RECIP}" || echo "HTTP:000")
pt_code=$(echo "$pt" | sed -n 's/^HTTP://p' | tail -1)
pt_body=$(echo "$pt" | sed '/^HTTP:/d')
echo "pay-targets HTTP ${pt_code}"
echo "$pt_body" | python3 -c "import json,sys
try:
 d=json.load(sys.stdin)
 print('eth_addr', (d.get('eth_addr') or '')[:10]+'…' if d.get('eth_addr') else None)
 print('base_addr', (d.get('base_addr') or '')[:10]+'…' if d.get('base_addr') else None)
 print('btc_addr', d.get('btc_addr'))
 print('zec_uri_set', bool(d.get('zec_uri')))
 print('memo_hex_prefix', (d.get('memo_hex') or '')[:4])
 print('zip321', d.get('zip321'))
except Exception as e:
 print('pay-targets parse', e)
" || true
eth_ok_pt=$(echo "$pt_body" | python3 -c "import json,sys
d=json.load(sys.stdin); print('1' if d.get('eth_addr','').startswith('0x') else '0')" 2>/dev/null || echo 0)
btc_ok_pt=$(echo "$pt_body" | python3 -c "import json,sys
d=json.load(sys.stdin); print('1' if d.get('btc_addr') else '0')" 2>/dev/null || echo 0)
memo_ok=$(echo "$pt_body" | python3 -c "import json,sys
d=json.load(sys.stdin); print('1' if str(d.get('memo_hex','')).startswith('6431') else '0')" 2>/dev/null || echo 0)
zec_uri_set=$(echo "$pt_body" | python3 -c "import json,sys
d=json.load(sys.stdin); print('1' if d.get('zec_uri') else '0')" 2>/dev/null || echo 0)
if [ "$pt_code" != "200" ] || [ "$eth_ok_pt" != "1" ] || [ "$btc_ok_pt" != "1" ] || [ "$memo_ok" != "1" ]; then
  add_blocker "pay-targets did not return HD eth/btc dests with memo_hex 6431 (HTTP ${pt_code})"
fi

echo
echo "=== HD seed (presence only) ==="
if [ -n "${DREGG_HD_SEED:-}" ]; then
  echo "DREGG_HD_SEED is set (len $(printf %s "$DREGG_HD_SEED" | wc -c | tr -d ' '))"
elif [ -s "${HOME}/.terp/dregg-hd.seed" ]; then
  echo "DREGG_HD_SEED file ~/.terp/dregg-hd.seed present"
else
  echo "DREGG_HD_SEED missing"
  add_blocker "no persisted DREGG_HD_SEED — ephemeral dests would rotate on restart"
fi

echo
echo "=== Ethereum ==="
if [ -z "${ETH_RPC:-}" ]; then
  echo "ETH_RPC unset — watcher no-ops"
  add_blocker "ETH_RPC unset; cannot see payments to HD eth_addr"
else
  echo "ETH_RPC $(redact_url "$ETH_RPC")"
  check_self_hosted ETH_RPC "$ETH_RPC"
  er=$(curl -sS -m 8 -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' -H 'content-type: application/json' "$ETH_RPC" || echo '{}')
  echo "$er"
  cid=$(echo "$er" | python3 -c "import json,sys
d=json.load(sys.stdin); print(d.get('result',''))" 2>/dev/null || echo "")
  if [ "$cid" != "0x1" ]; then
    add_blocker "ETH_RPC chainId is ${cid:-empty}, want 0x1"
  fi
fi

echo
echo "=== Base ==="
if [ -z "${BASE_RPC:-}" ]; then
  echo "BASE_RPC unset — watcher no-ops"
  add_blocker "BASE_RPC unset; cannot see payments to HD base_addr"
else
  echo "BASE_RPC $(redact_url "$BASE_RPC")"
  check_self_hosted BASE_RPC "$BASE_RPC"
  br=$(curl -sS -m 8 -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' -H 'content-type: application/json' "$BASE_RPC" || echo '{}')
  echo "$br"
  cid=$(echo "$br" | python3 -c "import json,sys
d=json.load(sys.stdin); print(d.get('result',''))" 2>/dev/null || echo "")
  if [ "$cid" != "0x2105" ]; then
    add_blocker "BASE_RPC chainId is ${cid:-empty}, want 0x2105 (8453)"
  fi
fi

echo
echo "=== Bitcoin ==="
if [ -z "${BTC_RPC:-}" ]; then
  echo "BTC_RPC unset — watcher no-ops"
  add_blocker "BTC_RPC unset; cannot see payments to HD btc_addr"
else
  echo "BTC_RPC $(redact_url "$BTC_RPC")"
  check_self_hosted BTC_RPC "$BTC_RPC"
  bb=$(curl -sS -m 8 -d '{"jsonrpc":"1.0","id":"pf","method":"getblockchaininfo","params":[]}' -H 'content-type: application/json' "$BTC_RPC" || echo '{}')
  echo "$bb" | python3 -c "import json,sys
d=json.load(sys.stdin); r=d.get('result') or {}; print('chain', r.get('chain'), 'blocks', r.get('blocks'));
print('error', d.get('error'))" 2>/dev/null || echo "btc parse fail"
  chain=$(echo "$bb" | python3 -c "import json,sys
d=json.load(sys.stdin); print((d.get('result') or {}).get('chain') or '')" 2>/dev/null || echo "")
  if [ -z "$chain" ]; then
    add_blocker "BTC_RPC getblockchaininfo failed"
  fi
  if [ "${DREGG_BTC_MAINNET:-}" = "1" ] && [ "$chain" != "main" ]; then
    add_blocker "DREGG_BTC_MAINNET=1 but bitcoind chain is ${chain}"
  fi
fi

echo
echo "=== Zcash compact streamer ==="
if [ -z "${ZCASH_DEVTOOL_SERVER:-}" ] && [ -z "${ZCASHD_RPC:-}" ]; then
  echo "no ZCASH_DEVTOOL_SERVER / ZCASHD_RPC"
  if [ "${PREFLIGHT_ALLOW_ZEC_SKIP:-}" = "1" ]; then
    echo "ZEC skipped honestly (PREFLIGHT_ALLOW_ZEC_SKIP=1)"
  else
    add_blocker "ZEC watcher unconfigured (need compact-tx-streamer 9067 + linux zcash-devtool, or hide ZEC in FE)"
  fi
else
  echo "ZCASH_DEVTOOL_SERVER=${ZCASH_DEVTOOL_SERVER:-}"
  if [ -n "${ZCASH_DEVTOOL_SERVER:-}" ]; then
    check_self_hosted ZCASH_DEVTOOL_SERVER "grpc://${ZCASH_DEVTOOL_SERVER}"
    case "${ZCASH_DEVTOOL_SERVER}" in
      *:8232) add_blocker "ZCASH_DEVTOOL_SERVER must be CompactTxStreamer :9067 (house) / :19067 (lab), not Zakura JSON-RPC :8232" ;;
    esac
  fi
  echo "ZCASH_DEVTOOL_NETWORK=${ZCASH_DEVTOOL_NETWORK:-}"
  bin="${ZCASH_DEVTOOL_BIN:-}"
  if [ -n "$bin" ] && [ -f "$bin" ]; then
    mag=$(od -An -t x1 -N 4 "$bin" | tr -d ' \n')
    echo "zcash-devtool magic $mag"
    if [ "$mag" != "7f454c46" ]; then
      add_blocker "ZCASH_DEVTOOL_BIN is not a Linux ELF"
    fi
  else
    add_blocker "ZCASH_DEVTOOL_BIN missing"
  fi
  if [ "$zec_uri_set" != "1" ]; then
    if [ "${PREFLIGHT_ALLOW_ZEC_SKIP:-}" = "1" ]; then
      echo "pay-targets zec_uri null — FE must hide ZEC"
    else
      add_blocker "pay-targets zec_uri is null; cannot issue ZIP-321 dests"
    fi
  fi
fi

echo
echo "=== Native Terp (minter LCD) ==="
if [ -z "${DREGG_MINTER_LCD:-}" ] || [ -z "${DREGG_MINTER_ADDR:-}" ]; then
  echo "DREGG_MINTER_LCD/ADDR unset"
  add_blocker "native checkout_ready cannot query minter balance (set DREGG_MINTER_LCD + DREGG_MINTER_ADDR)"
else
  echo "LCD set, minter addr ${DREGG_MINTER_ADDR}"
  nb=$(curl -sS -m 8 "${DREGG_MINTER_LCD%/}/cosmos/bank/v1beta1/balances/${DREGG_MINTER_ADDR}" || echo '{}')
  echo "$nb" | python3 -c "import json,sys
d=json.load(sys.stdin); print([(b.get('denom'), b.get('amount')) for b in d.get('balances') or []])" 2>/dev/null || echo "lcd parse fail"
fi

echo
echo "=== HashMerchant onboard ==="
ONBOARD_LOCAL="${ONBOARD_LOCAL:-http://127.0.0.1:19090}"
ost=$(curl -sS -m 8 "${ONBOARD_LOCAL}/onboard/status" || curl -sS -m 8 "${ONBOARD_LOCAL}/status" || echo "{}")
echo "$ost" | python3 -c "import json,sys
try:
 d=json.load(sys.stdin)
 print('ready', d.get('ready'))
 print('runtime', d.get('runtime') or d.get('dao'))
 print('hashmerchant', d.get('hashmerchant') or d.get('runtime_key'))
 print('fee_granter', d.get('fee_granter'))
 print('broadcast', d.get('broadcast'))
except Exception as e:
 print('onboard parse', e)
" || true
python3 - <<PY
import json, os
raw = '''$ost'''
try:
    d = json.loads(raw)
except Exception:
    raise SystemExit
rt = d.get("runtime") or d.get("dao") or ""
hm = d.get("hashmerchant") or d.get("runtime_key") or ""
if not str(rt).startswith("terp1") or rt in ("DAO_ADDR",):
    print("BLOCK onboard runtime is not a terp1")
if not str(hm).startswith("terp1") or hm in ("RUNTIME_KEY_ADDR","HASHMERCHANT_ADDR"):
    print("BLOCK onboard hashmerchant is not a terp1")
if rt and hm and rt == hm:
    print("BLOCK runtime equals hashmerchant (collapsed fee.granter)")
if d.get("broadcast") is False:
    print("BLOCK onboard broadcast is false — feegrants will not land")
PY
# collect python BLOCK lines
oblocks=$(python3 - <<PY
import json
raw = '''$ost'''
try:
    d = json.loads(raw)
except Exception:
    print("onboard status not JSON")
    raise SystemExit
rt = d.get("runtime") or d.get("dao") or ""
hm = d.get("hashmerchant") or d.get("runtime_key") or ""
if not str(rt).startswith("terp1") or rt in ("DAO_ADDR",):
    print("onboard runtime is not a live terp1")
if not str(hm).startswith("terp1") or hm in ("RUNTIME_KEY_ADDR","HASHMERCHANT_ADDR"):
    print("onboard hashmerchant is not a live terp1")
if rt and hm and rt == hm:
    print("runtime equals hashmerchant")
if d.get("broadcast") is False:
    print("onboard broadcast is false")
PY
)
if [ -n "$oblocks" ]; then
  oldifs=$IFS
  IFS='
'
  for line in $oblocks; do
    [ -n "$line" ] && add_blocker "$line"
  done
  IFS=$oldifs
fi

RT="${ONBOARD_RUNTIME:-}"
HM="${ONBOARD_HASHMERCHANT:-}"
if [ -n "$RT" ] && [ -n "$HM" ]; then
  echo "authz LCD query granter=$RT grantee=$HM"
  ag=$(curl -sS -m 8 "https://api.terp.network/cosmos/authz/v1beta1/grants?granter=${RT}&grantee=${HM}" || echo '{}')
  echo "$ag" | python3 -c "import json,sys
d=json.load(sys.stdin); g=d.get('grants') or []; print('grant_count', len(g))"
  gc=$(echo "$ag" | python3 -c "import json,sys
d=json.load(sys.stdin); print(len(d.get('grants') or []))" 2>/dev/null || echo 0)
  if [ "$gc" = "0" ]; then
    add_blocker "no on-chain authz from runtime to HashMerchant (run hashmerchant_authz_bootstrap with AUTHZ_BROADCAST=1)"
  fi
  hb=$(curl -sS -m 8 "https://api.terp.network/cosmos/bank/v1beta1/balances/${HM}" || echo '{}')
  echo "$hb" | python3 -c "import json,sys
d=json.load(sys.stdin); print('hm_balances', [(b.get('denom'), b.get('amount')) for b in d.get('balances') or []])"
  hm_amt=$(echo "$hb" | python3 -c "import json,sys
d=json.load(sys.stdin)
n=0
for b in d.get('balances') or []:
    if b.get('denom') in ('uthiol','uterp'):
        n=max(n,int(b.get('amount') or 0))
print(n)" 2>/dev/null || echo 0)
  if [ "$hm_amt" = "0" ]; then
    add_blocker "HashMerchant account has no uterp/uthiol — cannot pay grant fees or runway"
  fi
fi

echo
echo "=== files (no contents) ==="
ls -l "${HOME}/.terp/onboard-runtime.mnemonic" "${HOME}/.terp/hashmerchant.mnemonic" "${HOME}/.terp/dregg-svg-runtime.mnemonic" "${HOME}/.terp/dregg-hd.seed" 2>&1 | sed 's#/home/[^/]*#~#'

echo
if [ -z "$blockers" ]; then
  echo "PREFLIGHT OK"
  exit 0
fi
echo "PREFLIGHT BLOCKERS:${blockers}"
exit 1
