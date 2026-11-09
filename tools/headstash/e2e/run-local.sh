#!/usr/bin/env bash
# One-command headstash membership-vk local e2e: fresh loopback chain + lane, then harness (direct ClaimNote by default).
# RESET=1 (default here) wipes /tmp/headstash-mvk-chain. Chain and lane stay running afterwards.
set -euo pipefail
export PATH="/usr/local/bin:$PATH"
HERE="$(cd "$(dirname "$0")" && pwd)"
RESET="${RESET:-1}" "$HERE/bootstrap-chain.sh"
# NOTE_EX / ZK_EX: halo2 circuit example dir (ZK_EX defaults to NOTE_EX, then /tmp/hm-build-target/release/examples).
# HARNESS=direct (default): v1 direct on-chain ClaimNote path. HARNESS=lane: off-chain lane path.
if [ "${HARNESS:-direct}" = "lane" ]; then
  exec /usr/local/bin/python3 "$HERE/run.py"
fi
[ "${HARNESS:-direct}" = "dao" ] && exec /usr/local/bin/python3 "$HERE/run_dao.py"
exec /usr/local/bin/python3 "$HERE/run_direct.py"
