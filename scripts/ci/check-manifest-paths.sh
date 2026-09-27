#!/usr/bin/env bash
# Static manifest audit: find path dependencies that do not resolve.
#
# `cargo metadata` stops at the first broken path dep, so a single missing
# directory hides every other manifest problem. This walks the manifests
# instead, so the full list is visible at once.
#
# Usage:
#   scripts/ci/check-manifest-paths.sh [root]
#   scripts/ci/check-manifest-paths.sh --help
#
# Exit codes: 0 all resolve / 1 at least one missing / 2 bad usage.

set -euo pipefail

ROOT="${1:-$PWD}"
[ "${1:-}" = "--help" ] && { sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0; }
[ -d "$ROOT" ] || { echo "not a directory: $ROOT" >&2; exit 2; }

# Directories never worth descending into.
PRUNE='(^\|/)(target|node_modules|\.git|\.worktrees|_worktrees)(\/|$)'

missing=0
checked=0

# Every path = "..." in a Cargo.toml, resolved relative to that manifest.
while IFS= read -r manifest; do
  dir="$(dirname "$manifest")"
  # Strip the surrounding quotes and any inline table braces.
  while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    case "$rel" in
      http://*|https://*|git+*|registry:*|"$"*|file://*) continue ;;
    esac
    checked=$((checked + 1))
    target="$dir/$rel"
    if [ ! -e "$target" ]; then
      printf 'MISSING  %s\n          -> %s\n' "${manifest#"$ROOT"/}" "$rel"
      missing=$((missing + 1))
    fi
  done < <(
    # Drop comment-only lines first: a commented-out path = is not a dependency.
    # Then join continuations so a multi-line table entry is seen whole.
    grep -vE '^[[:space:]]*#' "$manifest" \
      | tr '\n' ' ' \
      | grep -oE 'path[[:space:]]*=[[:space:]]*"[^"]+"' \
      | sed -E 's/.*"([^"]+)".*/\1/'
  )
done < <(find "$ROOT" -name Cargo.toml -not -path "$ROOT/$PRUNE" 2>/dev/null | LC_ALL=C sort)

echo
if (( missing > 0 )); then
  echo "manifest paths: $missing missing of $checked checked"
  exit 1
fi
echo "manifest paths: all $checked resolve"
