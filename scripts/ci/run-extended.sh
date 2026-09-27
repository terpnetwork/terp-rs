#!/usr/bin/env bash
# Full act for ci-extended.yml (contracts-core + scripts-integration).
# Prefer host: just ci-extended
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

act_require
trap 'act_teardown; act_prune_runs' EXIT INT TERM
act_teardown quiet

WORKFLOW="$(act_workflow_for_act ci-extended.yml)"
[[ -f "${WORKFLOW}" ]] || { echo "missing ${WORKFLOW}" >&2; exit 1; }

TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
COMMIT_SHORT="$(cd "${CRATE_ROOT}" && git rev-parse --short HEAD 2>/dev/null || echo unknown)"
TMPDIR="${RUNS_DIR}/${TIMESTAMP}_ext_tmp"
mkdir -p "${TMPDIR}"

echo "==================== act ci-extended ===================="
echo "Workflow: ${WORKFLOW}"
echo "Prefer host: just ci-extended"
echo "========================================================="

cd "${MONOREPO_ROOT}"
failed_jobs=()

for job in contracts-core scripts-integration; do
  echo ""
  echo "--- Job: ${job} ---"
  set +e
  act_run_job "${WORKFLOW}" "${job}" "${TMPDIR}/${job}.log"
  rc=$?
  set -e
  if [[ "$rc" -eq 0 ]]; then
    echo ">>> PASS: ${job}"
  else
    echo ">>> FAIL: ${job}"
    failed_jobs+=("${job}")
  fi
done

{
  echo ""
  echo "## Extended act — ${TIMESTAMP} (\`${COMMIT_SHORT}\`)"
  echo ""
  echo "| Job | Status |"
  echo "|-----|--------|"
  for job in contracts-core scripts-integration; do
    if [[ " ${failed_jobs[*]-} " == *" ${job} "* ]]; then
      echo "| ${job} | FAIL |"
    else
      echo "| ${job} | PASS |"
    fi
  done
  echo ""
  echo "Teardown ran after each job. Host: \`just ci-extended\`."
} >>"${RUNS_DIR}/latest-status.md"

[[ ${#failed_jobs[@]} -eq 0 ]] || exit 1
