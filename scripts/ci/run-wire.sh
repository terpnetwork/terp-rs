#!/usr/bin/env bash
# Fast act wiring check — list/parse workflows + optional dry container start.
# Does NOT compile the Rust workspace (saves Docker Desktop from RWLayer crashes).
#
# Usage:
#   ./scripts/ci/run-wire.sh
#   just act-wire
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

act_require
trap 'act_teardown quiet' EXIT INT TERM

cd "${MONOREPO_ROOT}"

TS="$(date -u +%Y%m%dT%H%M%SZ)"
COMMIT_SHORT="$(cd "${CRATE_ROOT}" && git rev-parse --short HEAD 2>/dev/null || echo unknown)"
mkdir -p "${RUNS_DIR}"
LOG="${RUNS_DIR}/${TS}_wire.log"
STATUS=0

echo "==================== act wire (fast) ===================="
echo "Monorepo: ${MONOREPO_ROOT}"
echo "Crate:    ${CRATE_ROOT}"
echo "========================================================="

{
  echo "# act wire — ${TS}"
  echo "commit: ${COMMIT_SHORT}"
  echo ""
} >"${LOG}"

for wf in ci-core.yml ci-extended.yml ci-heavy.yml; do
  path="$(act_workflow_for_act "$wf")"
  if [[ ! -f "$path" ]]; then
    echo "MISSING workflow: $path" | tee -a "${LOG}"
    STATUS=1
    continue
  fi
  echo ""
  echo "--- list jobs: ${wf} ---"
  set +e
  # shellcheck disable=SC2046
  act -W "${path}" -l $(act_arch_args) 2>&1 | tee -a "${LOG}"
  rc=${PIPESTATUS[0]}
  set -e
  if [[ "$rc" -ne 0 ]]; then
    echo "FAIL list ${wf} rc=${rc}" | tee -a "${LOG}"
    STATUS=1
  fi
done

# Optional: one empty job dry-run if ACT_WIRE_DRY=1 (still needs image)
if [[ "${ACT_WIRE_DRY:-0}" == "1" ]]; then
  path="$(act_workflow_for_act ci-core.yml)"
  echo ""
  echo "--- dry graph for core-gate (echo only) ---"
  set +e
  # shellcheck disable=SC2046
  act -W "${path}" -j core-gate -P "ubuntu-latest=${ACT_IMAGE}" --rm --no-cache-server \
    $(act_arch_args) \
    2>&1 | tee -a "${LOG}"
  rc=${PIPESTATUS[0]}
  set -e
  [[ "$rc" -eq 0 ]] || STATUS=1
fi

act_teardown
act_prune_runs

RESULT=PASS
[[ "$STATUS" -eq 0 ]] || RESULT=FAIL

cat >"${RUNS_DIR}/latest-wire.md" <<EOF
# act wire — ${TS}

| Field | Value |
|-------|--------|
| result | **${RESULT}** |
| commit | \`${COMMIT_SHORT}\` |
| log | \`${TS}_wire.log\` |

Workflows listed via \`act -l\`. No full cargo under act (use host \`just ci-core\`).
EOF

# Merge note into latest-status if host report exists
if [[ -f "${RUNS_DIR}/latest-host-core.md" ]]; then
  {
    echo ""
    echo "## act wire"
    echo ""
    echo "| result | ${RESULT} |"
    echo "| log | \`${TS}_wire.log\` |"
  } >>"${RUNS_DIR}/latest-status.md"
fi

echo ""
echo "act wire: ${RESULT}"
exit "${STATUS}"
