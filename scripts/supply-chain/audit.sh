#!/usr/bin/env bash
# RustSec advisory check for every root in roots.txt.
# cargo-audit 0.22: non-zero only when a vulnerability is reported.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
terp="$(cd "$here/../.." && pwd)"
if ! command -v cargo-audit >/dev/null 2>&1; then
  echo "cargo-audit is not installed. cargo install cargo-audit --locked" >&2
  exit 1
fi
fail=0
while IFS= read -r rel || [[ -n "${rel:-}" ]]; do
  [[ -z "${rel}" || "${rel}" == \#* ]] && continue
  root="$terp/$rel"
  echo "== cargo audit  $root"
  if ! (cd "$root" && cargo audit); then
    fail=1
  fi
done <"$here/roots.txt"
exit "$fail"
