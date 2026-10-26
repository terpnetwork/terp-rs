#!/usr/bin/env bash
# One-command headstash membership-vk local e2e: fresh loopback chain + lane, then harness.
# RESET=1 (default here) wipes /tmp/headstash-mvk-chain. Chain and lane stay running afterwards.
set -euo pipefail
export PATH="/usr/local/bin:$PATH"
HERE="$(cd "$(dirname "$0")" && pwd)"
RESET="${RESET:-1}" "$HERE/bootstrap-chain.sh"
exec /usr/local/bin/python3 "$HERE/run.py"
