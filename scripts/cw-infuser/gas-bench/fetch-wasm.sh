#!/usr/bin/env bash
# Fetch the on-chain code-71 cw721-svg wasm (read-only LCD query) and verify its sha256.
# LCD defaults to the local groot2 tunnel; override with LCD=https://... ./fetch-wasm.sh
set -euo pipefail
cd "$(dirname "$0")"
LCD="${LCD:-http://127.0.0.1:11317}"
WANT=990e3a93b27d1abaabcec2739b36fb6f8b89a1b20d3cd7ea554ba56e10a55b9f
mkdir -p wasm
curl -fsS "$LCD/cosmwasm/wasm/v1/code/71" \
  | python3 -c 'import sys,json,base64;sys.stdout.buffer.write(base64.b64decode(json.load(sys.stdin)["data"]))' \
  > wasm/cw721_svg_code71.wasm
GOT=$(shasum -a 256 wasm/cw721_svg_code71.wasm | cut -d" " -f1)
[ "$GOT" = "$WANT" ] || { echo "sha256 mismatch: $GOT" >&2; exit 1; }
echo "ok wasm/cw721_svg_code71.wasm $GOT"
