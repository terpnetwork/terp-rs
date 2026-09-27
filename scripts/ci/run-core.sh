#!/usr/bin/env bash
# Full act run for ci-core.yml jobs (expensive; Docker Desktop fragile on macOS).
# Prefer: just act-host-core  (authoritative) + just act-wire (YAML list).
#
# Usage:
#   ./scripts/ci/run-core.sh
#   ACT_PULL=1 ACT_JOB_TIMEOUT=1200 ./scripts/ci/run-core.sh
#   just act-core
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

act_require
trap 'act_teardown; act_prune_runs' EXIT INT TERM
act_teardown quiet

WORKFLOW="$(act_workflow_for_act ci-core.yml)"
if [[ ! -f "${WORKFLOW}" ]]; then
  echo "error: missing ${WORKFLOW}" >&2
  exit 1
fi

TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
COMMIT_SHA="$(cd "${CRATE_ROOT}" && git rev-parse HEAD 2>/dev/null || echo unknown)"
COMMIT_SHORT="$(cd "${CRATE_ROOT}" && git rev-parse --short HEAD 2>/dev/null || echo unknown)"
TMPDIR="${RUNS_DIR}/${TIMESTAMP}_tmp"
mkdir -p "${TMPDIR}"

echo "==================== act ci-core (full) ===================="
echo "Monorepo:  ${MONOREPO_ROOT}"
echo "Workflow:  ${WORKFLOW}"
echo "Crate:     ${CRATE_ROOT}"
echo "Image:     ${ACT_IMAGE}"
echo "Pull:      ${ACT_PULL}  timeout/job: ${ACT_JOB_TIMEOUT}s"
echo "Commit:    ${COMMIT_SHORT}"
echo "============================================================"
echo "NOTE: Host gate is authoritative: just act-host-core / just ci-core"

cd "${MONOREPO_ROOT}"

failed_jobs=()
times_file="${TMPDIR}/times.txt"
: >"${times_file}"

for job in terp-scripts-offline internal-libs; do
  echo ""
  echo "--- Job: ${job} ---"
  t0="$(date +%s)"
  set +e
  act_run_job "${WORKFLOW}" "${job}" "${TMPDIR}/${job}.log"
  rc=$?
  set -e
  t1="$(date +%s)"
  dur=$((t1 - t0))
  echo "${job} ${dur}" >>"${times_file}"
  if [[ "$rc" -eq 0 ]]; then
    echo ">>> PASS: ${job} (${dur}s)"
  else
    echo ">>> FAIL: ${job} exit=${rc} (${dur}s)"
    failed_jobs+=("${job}")
  fi
done

STATUS_FILE="${RUNS_DIR}/latest-status.md"
{
  echo "# act ci-core full — ${TIMESTAMP}"
  echo ""
  echo "| Field | Value |"
  echo "|-------|--------|"
  echo "| commit | \`${COMMIT_SHORT}\` (\`${COMMIT_SHA}\`) |"
  echo "| image | ${ACT_IMAGE} |"
  echo "| pull | ${ACT_PULL} |"
  echo "| job_timeout_s | ${ACT_JOB_TIMEOUT} |"
  echo ""
  echo "## Results"
  echo ""
  echo "| Job | Status | Duration |"
  echo "|-----|--------|----------|"
  while read -r job dur; do
    st="PASS"
    for f in "${failed_jobs[@]+"${failed_jobs[@]}"}"; do
      [[ "$f" == "$job" ]] && st="FAIL"
    done
    echo "| ${job} | ${st} | ${dur}s |"
  done <"${times_file}"
  echo ""
  echo "## Teardown"
  echo ""
  echo "Containers/volumes cleaned after each job and on EXIT (\`scripts/ci/teardown.sh\`)."
  echo ""
  echo "## Friction"
  echo ""
  echo "- Full act binds monorepo root so path deps (\`../cosmwasm\`, etc.) resolve."
  echo "- GHA: checkout root is terp-rs; act sets \`GITHUB_WORKSPACE\` to crate root."
  echo "- Docker Desktop macOS: prefer host \`just ci-core\` if RWLayer crashes."
  echo "- Default \`ACT_PULL=0\` avoids re-pulling images every run."
  echo ""
  echo "## Logs"
  echo ""
  echo "\`${TIMESTAMP}_tmp/<job>.log\`"
  echo ""
  echo "## Host gate"
  echo ""
  echo "Always also run: \`just act-host-core\`."
} >"${STATUS_FILE}"

echo ""
echo "==================== Summary ====================="
if [[ ${#failed_jobs[@]} -eq 0 ]]; then
  echo "All act ci-core jobs passed."
  exit 0
fi
echo "Failed: ${failed_jobs[*]}"
echo "If Docker Desktop crashed: just act-teardown && just act-host-core"
exit 1
