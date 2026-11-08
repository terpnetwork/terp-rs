#!/bin/sh
# Local ultra-light prove: contrast lock vs repo, then ict-rs BTC + ZEC (ztreamer) lab e2e.
# Does not start groot2 house, cookie-harness LWD, or bid Akash.
set -eu
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
"$DIR/ultralight-contrast.sh"
"$DIR/lab-e2e-btc.sh"
"$DIR/lab-e2e-zec.sh"
