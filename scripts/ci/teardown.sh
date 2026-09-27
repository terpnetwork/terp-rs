#!/usr/bin/env bash
# Force-clean nektos/act leftovers (containers + volumes). Safe to run anytime.
# Usage:
#   ./scripts/ci/teardown.sh
#   just act-teardown
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
act_teardown
act_prune_runs
echo "runs dir pruned (keep last ${ACT_RUNS_KEEP}): ${RUNS_DIR}"
