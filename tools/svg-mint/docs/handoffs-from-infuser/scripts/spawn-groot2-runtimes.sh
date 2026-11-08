#!/bin/sh
# Spawn DREGG + HashMerchant onboard on groot2, bound to loopback, behind nginx.
# Run ON groot2-wan (x86_64). Source ~/.terp/dregg-svg.env and ~/.terp/onboard.env first
# (never print those files). Usage:
#   ssh groot2-wan
#   set -a && . ~/.terp/dregg-svg.env && . ~/.terp/onboard.env && set +a
#   ./spawn-groot2-runtimes.sh
set -eu

HOME="${HOME:-/Users/returniflost}"
ROOT="${TERP_CORE:-$HOME/abstract/terp-core/crates}"
DREGG_BIN="${DREGG_NODE_BIN:-$ROOT/dregg/svg-mint/target/release/svg-mint-fed}"
ONBOARD_BIN="${ONBOARD_NODE_BIN:-$ROOT/terp-rs/tools/onboard-authz/target/release/onboard-runtime}"
NGINX_SRC="$ROOT/dregg/svg-mint/scripts"

need() {
  eval "v=\${$1:-}"
  if [ -z "$v" ]; then
    echo "missing env $1 (load ~/.terp/*.env; do not invent)" >&2
    exit 2
  fi
}

need DREGG_HD_SEED
need DREGG_MINTER_LCD
need DREGG_MINTER_ADDR
need ONBOARD_RUNTIME
need ONBOARD_CHAIN

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck disable=SC1091
. "$SCRIPT_DIR/rpc-allowlist.sh"
REQUIRE_SELF="${SVG_MINT_REQUIRE_SELF_HOSTED_RPC:-1}"
if [ "$REQUIRE_SELF" = "1" ]; then
  need ETH_RPC
  need BASE_RPC
  need BTC_RPC
  need ONBOARD_HASHMERCHANT
  for pair in "ETH_RPC:$ETH_RPC" "BASE_RPC:$BASE_RPC" "BTC_RPC:$BTC_RPC"; do
    n=${pair%%:*}
    u=${pair#*:}
    if ! rpc_url_is_self_hosted "$u"; then
      echo "$n is not self-hosted (set SVG_MINT_REQUIRE_SELF_HOSTED_RPC=0 only in lab)" >&2
      exit 4
    fi
  done
fi

if [ ! -x "$DREGG_BIN" ]; then
  echo "missing amd64 svg-mint-fed at $DREGG_BIN — cargo build --release --bin svg-mint-fed on groot2" >&2
  exit 3
fi
if [ ! -x "$ONBOARD_BIN" ]; then
  echo "missing amd64 onboard-runtime at $ONBOARD_BIN — cargo build --release -p terp-onboard-authz --bin onboard-runtime on groot2" >&2
  exit 3
fi
# Refuse aarch64 ELFs copied from groot-wan
if command -v file >/dev/null 2>&1; then
  echo "$DREGG_BIN $(file -b "$DREGG_BIN")"
  echo "$ONBOARD_BIN $(file -b "$ONBOARD_BIN")"
  echo "$DREGG_BIN" | grep -q . || true
fi

umask 077
mkdir -p "$HOME/.terp"

# Loopback only — Cloudflare/nginx own the public face.
export SVG_MINT_FED_BIND="${SVG_MINT_FED_BIND:-127.0.0.1:8421}"
export SVG_MINT_FED_STORE="${SVG_MINT_FED_STORE:-$HOME/.terp/svg-mint-fed-cell.json}"
if [ -z "${DREGG_HD_SEED:-}" ] && [ -f "$HOME/.terp/dregg-hd.seed" ]; then
  DREGG_HD_SEED=$(tr -d '\n' < "$HOME/.terp/dregg-hd.seed")
  export DREGG_HD_SEED
fi
# ZIP-321: compact streamer (ztreamer preferred, house LWD fallback) + linux ELF zcash-devtool
if [ -z "${ZCASH_DEVTOOL_SERVER:-}" ]; then
  if nc -z 127.0.0.1 19067 2>/dev/null; then
    export ZCASH_DEVTOOL_SERVER=127.0.0.1:19067
  elif nc -z 127.0.0.1 9067 2>/dev/null; then
    export ZCASH_DEVTOOL_SERVER=127.0.0.1:9067
  fi
fi
export ZCASH_DEVTOOL_NETWORK="${ZCASH_DEVTOOL_NETWORK:-main}"
if [ -z "${ZCASH_DEVTOOL_BIN:-}" ]; then
  for c in \
    "$HOME/.terp/zcash-devtool" \
    "$ROOT/zcash/zcash-devtool/target/release/zcash-devtool" \
    "$HOME/zcash-devtool/target/release/zcash-devtool"
  do
    if [ -x "$c" ]; then export ZCASH_DEVTOOL_BIN="$c"; break; fi
  done
fi
if [ -z "${ZCASH_DEVTOOL_WALLET:-}" ] && [ -d "$HOME/.terp/ufvk-wallet" ]; then
  export ZCASH_DEVTOOL_WALLET="$HOME/.terp/ufvk-wallet"
fi
export ONBOARD_BIND="${ONBOARD_BIND:-127.0.0.1}"
export ONBOARD_PORT="${ONBOARD_PORT:-19090}"
export ONBOARD_SERVICE="${ONBOARD_SERVICE:-hashmerchant-onboard}"

# mint-grant stays disabled in nginx even if the binary still has the route
export ONBOARD_BROADCAST="${ONBOARD_BROADCAST:-1}"

stop_pid() {
  f="$1"
  if [ -f "$f" ]; then
    old=$(cat "$f" || true)
    if [ -n "${old:-}" ] && kill -0 "$old" 2>/dev/null; then
      kill "$old" || true
      sleep 1
    fi
    rm -f "$f"
  fi
}

stop_pid "$HOME/.terp/svg-mint-fed.pid"
stop_pid "$HOME/.terp/onboard-runtime.pid"

nohup "$DREGG_BIN" >"$HOME/.terp/svg-mint-fed.log" 2>&1 &
echo $! >"$HOME/.terp/svg-mint-fed.pid"
nohup "$ONBOARD_BIN" >"$HOME/.terp/onboard-runtime.log" 2>&1 &
echo $! >"$HOME/.terp/onboard-runtime.pid"

# nginx sites (needs sudo). Tunnel Host headers hit :80.
if command -v nginx >/dev/null 2>&1 && [ -d /etc/nginx/sites-available ]; then
  sudo cp "$NGINX_SRC/nginx-dregg-svg.permissionless.money.conf" /etc/nginx/sites-available/dregg-svg.permissionless.money.conf
  sudo cp "$NGINX_SRC/nginx-onboard.permissionless.money.conf" /etc/nginx/sites-available/onboard.permissionless.money.conf
  sudo ln -sfn /etc/nginx/sites-available/dregg-svg.permissionless.money.conf /etc/nginx/sites-enabled/dregg-svg.permissionless.money.conf
  sudo ln -sfn /etc/nginx/sites-available/onboard.permissionless.money.conf /etc/nginx/sites-enabled/onboard.permissionless.money.conf
  sudo nginx -t
  sudo nginx -s reload || sudo systemctl reload nginx
else
  echo "nginx not found or no sites-available — install sites by hand" >&2
fi

echo "local DREGG:   curl -sS -H 'Host: dregg-svg.permissionless.money' http://127.0.0.1/health"
echo "local onboard: curl -sS -H 'Host: hash.terp.network' http://127.0.0.1/onboard/status"
echo "mint-grant must 404: curl -sS -o /dev/null -w '%{http_code}' -H 'Host: hash.terp.network' http://127.0.0.1/onboard/v1/mint-grant"
echo "public: https://dregg-svg.permissionless.money/health  https://hash.terp.network/onboard/status"
