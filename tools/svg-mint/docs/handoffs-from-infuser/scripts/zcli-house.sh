#!/usr/bin/env bash
# House zcli: never public zidecar. Endpoint is loopback ztreamer (grpc-web
# CompactTxStreamer GetLightdInfo works; zidecar.v1 GetTip is not on ztreamer —
# use `ztreamer probe` + DREGG CompactTxStreamer watch for deposits).
set -euo pipefail
EP="${ZCLI_ENDPOINT:-http://127.0.0.1:9067}"
case "$EP" in
  *rotko.net*|*zec.rocks*|*lightwalletd*)
    echo "zcli-house: refusing public indexer $EP" >&2
    exit 1
    ;;
esac
if [[ "$EP" != *127.0.0.1* && "$EP" != *localhost* ]]; then
  echo "zcli-house: endpoint must be loopback (got $EP)" >&2
  exit 1
fi
export ZCLI_ENDPOINT="$EP"
exec "${ZCLI_BIN:-zcli}" "$@"
