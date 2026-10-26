#!/usr/bin/env bash
# One-command headstash membership-vk local e2e: fresh loopback chain + lane, then harness.
# RESET=1 (default here) wipes /tmp/headstash-mvk-chain. Chain and lane stay running afterwards.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
RESET="${RESET:-1}" "$HERE/bootstrap-chain.sh"
exec python3 "$HERE/run.py"
