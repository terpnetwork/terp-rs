#!/usr/bin/env bash
# terp-rs: fetch the pinned ict-ci tarball and fail if expected suites are missing.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
DEST="${1:-${ICT_RS_BIN_DIR:-/tmp/ict-rs-bins}}"
# shellcheck source=ict-rs-bins.env
. "$HERE/ict-rs-bins.env"
bash "$HERE/fetch-ict-rs-bins.sh" "$DEST"
EXPECT="$HERE/ict-rs-suites.txt"
if [ ! -f "$EXPECT" ]; then
  echo "missing $EXPECT" >&2
  exit 1
fi
missing=0
while IFS= read -r s; do
  [ -n "$s" ] || continue
  case "$s" in \#*) continue ;; esac
  if [ ! -x "$DEST/examples/$s" ]; then
    echo "pin $ICT_RS_COMMIT missing examples/$s" >&2
    missing=1
  fi
done < "$EXPECT"
if [ "$missing" = 1 ]; then
  echo "contents:" >&2
  ls -la "$DEST" "$DEST/examples" >&2
  exit 1
fi
echo "pin $ICT_RS_COMMIT has all suites from $EXPECT"
