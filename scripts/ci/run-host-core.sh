#!/usr/bin/env bash
# Host-native Tier 0 gate — primary CI exercise (fast, no act/Docker lag).
# Usage: ./scripts/ci/run-host-core.sh  |  just ci-core  |  just act-host-core
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

host_require
cd "${CRATE_ROOT}"

START="$(date +%s)"
STATUS=0
REPORT="${RUNS_DIR}/latest-host-core.md"
mkdir -p "${RUNS_DIR}"

COMMIT_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
COMMIT_SHA="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
TS="$(date -u +%Y%m%dT%H%M%SZ)"

echo "==================== host ci-core ===================="
echo "CWD:    ${CRATE_ROOT}"
echo "Commit: ${COMMIT_SHORT}"
echo "======================================================"

set +e
timed_step "just ci-core" just ci-core
STATUS=$?
set -e

END="$(date +%s)"
DUR=$((END - START))

if [[ "$STATUS" -eq 0 ]]; then
  RESULT="PASS"
else
  RESULT="FAIL"
fi

cat >"${REPORT}" <<EOF
# host ci-core — ${TS}

| Field | Value |
|-------|--------|
| result | **${RESULT}** |
| exit | ${STATUS} |
| duration_s | ${DUR} |
| commit | \`${COMMIT_SHORT}\` (\`${COMMIT_SHA}\`) |
| cwd | \`${CRATE_ROOT}\` |
| command | \`just ci-core\` |

## Recipe coverage

- \`scripts-ibc-preflight offline\`
- \`scripts-ibc-offline\` (lib + ibc_unit + ibc_golden + rebuild-from-public)
- \`scripts-ibc-validate\`
- dense lib packs via `scripts/ci/dense-packs.sh core-libs` (multi-package cargo)

## Notes

This is the **authoritative** local gate. act is optional YAML fidelity only
(\`just act-wire\` / \`just act-core\`) and must always tear down containers.
EOF

# Also refresh combined latest-status pointer
cat >"${RUNS_DIR}/latest-status.md" <<EOF
# Local CI status — ${TS}

| Track | Result | Duration | Detail |
|-------|--------|----------|--------|
| **host ci-core** | ${RESULT} | ${DUR}s | \`latest-host-core.md\` |
| act | (not run in this script) | — | use \`just act-wire\` or \`just act-core\` |

**Commit:** \`${COMMIT_SHORT}\`

Primary gate: **host**. Do not merge on act green alone if host is red.
EOF

echo ""
echo "Wrote ${REPORT} (${RESULT}, ${DUR}s)"
exit "${STATUS}"
