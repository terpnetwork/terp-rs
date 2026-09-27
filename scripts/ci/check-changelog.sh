#!/usr/bin/env bash
# Validate the changelog fragment a terp-rs PR owns.
#
# Policy: skills/pr-changelog/SKILL.md. One fragment per PR, named for the PR
# number, in changelog/unreleased/. Internal work still gets a file, marked
# none, with a specific reason.
#
# Usage:
#   scripts/ci/check-changelog.sh                 # validate every fragment
#   scripts/ci/check-changelog.sh --scaffold 123  # write the skeleton for #123
#   scripts/ci/check-changelog.sh --help
#
# Exit codes: 0 ok / 1 fragment problem (or none present) / 2 bad usage.

set -euo pipefail

CRATE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIR="$CRATE_ROOT/changelog/unreleased"

CATEGORIES=(Added Changed Deprecated Removed Fixed Security)
NONE_MARK='<!-- changelog: none -->'

# Entries must open with one of these. Past tense, capitalised.
VERBS="Added|Changed|Deprecated|Removed|Fixed|Security|Corrected|Guarded|Moved|Renamed|Documented|Introduced|Upgraded|Pinned|Unpinned|Stopped|Relaxed|Tightened"

usage() {
  sed -n '2,${/^#/!q;s/^# \{0,1\}//;p;}' "${BASH_SOURCE[0]}"
}

red() { printf '\033[31m%s\033[0m\n' "$*" >&2; }
ok()  { printf '  \033[32mok\033[0m %s\n' "$*"; }
bad() { red "  FAIL  $*"; }

# --- scaffold ---------------------------------------------------------------

scaffold() {
  local pr="${1:-}"
  [[ "$pr" =~ ^[0-9]+$ ]] || { red "scaffold needs a numeric PR number, got '$pr'"; exit 2; }
  mkdir -p "$DIR"
  local f="$DIR/$pr.md"
  if [[ -e "$f" ]]; then
    red "changelog fragment already exists: ${f#$CRATE_ROOT/}"
    exit 1
  fi
  cat >"$f" <<EOF
<!--
Uncomment exactly one of the two blocks below and delete this header.

Operator-visible:

## Added

- Added the \`verp\` light client for IBC 08-wasm
  ([#$pr](https://github.com/terpnetwork/terp-rs/pull/$pr)).

Internal only:

$NONE_MARK

This PR only <be specific: what changed and why it is not operator-visible>.
-->

<!--
Categories, in the order they should appear:
$(printf '  %s\n' "${CATEGORIES[@]}")

Every entry starts with a past-tense verb, is one line, and describes the
observable effect rather than the patch.
-->
EOF
  ok "scaffolded ${f#$CRATE_ROOT/}"
  printf '  uncomment one block, delete the header, then: scripts/ci/check-changelog.sh\n'
}

# --- validate ---------------------------------------------------------------

# Split a fragment into three streams:
#
#   SPLIT_ACTIVE     lines a reader actually sees
#   SPLIT_TOP        raw lines that are a *top-level* HTML comment
#   SPLIT_COMMENTED  all comment text, top-level and nested
#
# HTML comments do not nest, so this is a single pass with one boolean: a `-->`
# on a line that also contains `<!--` still ends the comment, and whatever
# follows it is active again. A `<!--` reached while already inside a comment
# is never treated as an opener, which is what separates a real marker from an
# example of one.
#
# The distinction matters because the `none` marker *is* an HTML comment. A
# scaffold can therefore carry a commented-out example of both dispositions and
# still be judged on its active content alone — a fresh scaffold fails on its
# own instructions rather than passing them.
split_comments() {
  local f="$1" a t c
  a="$(mktemp)"; t="$(mktemp)"; c="$(mktemp)"
  awk -v act="$a" -v top="$t" -v com="$c" '
    {
      line = $0
      # Continue an open comment.
      if (incomment) {
        p = index(line, "-->")
        if (p == 0) { print line > com; next }
        print substr(line, 1, p - 1) > com
        line = substr(line, p + 3)
        incomment = 0
      }
      # Open a comment. It may also close on this same line.
      p = index(line, "<!--")
      if (p > 0) {
        head = substr(line, 1, p - 1)
        print head > act
        rest = substr(line, p + 4)
        if (head ~ /^[[:space:]]*$/) print line > top
        q = index(rest, "-->")
        if (q > 0) {                       # single-line comment
          print substr(rest, 1, q - 1) > com
          print substr(rest, q + 3) > act
          next
        }
        print rest > com
        incomment = 1
        next
      }
      print line > act
    }
  ' "$f"
  SPLIT_ACTIVE="$(cat "$a")"
  SPLIT_TOP="$(cat "$t")"
  SPLIT_COMMENTED="$(cat "$c")"
  rm -f "$a" "$t" "$c"
}

validate_one() {
  local f="$1"
  local name; name="$(basename "$f" .md)"
  local errs=0

  split_comments "$f"
  local body="$SPLIT_ACTIVE"

  # Filename must be the PR number.
  if ! [[ "$name" =~ ^[0-9]+$ ]]; then
    bad "$name.md: filename must be the PR number"
    errs=1
  fi

  # Exactly one disposition: marked none, or at least one category.
  local has_none=0 has_cat=0
  grep -qxF "$NONE_MARK" <<<"$SPLIT_TOP" && has_none=1
  local present_cats=()
  for c in "${CATEGORIES[@]}"; do
    if grep -qE "^##[[:space:]]+${c}[[:space:]]*$" <<<"$body"; then
      has_cat=1
      present_cats+=("$c")
    fi
  done

  if (( has_none && has_cat )); then
    bad "$name.md: has both '$NONE_MARK' and a category heading; pick one"
    errs=1
  fi
  if (( !has_none && !has_cat )); then
    bad "$name.md: no active category heading and no active '$NONE_MARK'"
    errs=1
  fi

  if (( has_cat )); then
    # Any '##' heading must be a known category.
    while IFS= read -r cat; do
      local known=0
      for c in "${CATEGORIES[@]}"; do [[ "$cat" == "$c" ]] && known=1; done
      if (( !known )); then
        bad "$name.md: unknown category '## $cat' (allowed: ${CATEGORIES[*]})"
        errs=1
      fi
    done < <(grep -oE '^##[[:space:]]+[A-Za-z]+[[:space:]]*$' <<<"$body" | sed -E 's/^##[[:space:]]+//; s/[[:space:]]+$//')

    # Each category that is present must have at least one entry.
    for c in "${present_cats[@]}"; do
      local entries
      entries="$(awk -v cat="$c" '
        $0 ~ "^##[[:space:]]+" cat "[[:space:]]*$" { inb=1; next }
        /^##[[:space:]]+/ { inb=0 }
        inb && /^-[[:space:]]/ { n++ }
        END { print n+0 }
      ' <<<"$body")"
      if (( entries == 0 )); then
        bad "$name.md: '## $c' has no entries"
        errs=1
      fi
    done

    # Every bullet starts with a past-tense verb, and the PR is linked.
    while IFS= read -r line; do
      [[ "$line" =~ ^-[[:space:]]+($VERBS)[[:space:]] ]] || {
        bad "$name.md: entry does not start with a past-tense verb: ${line:0:60}"
        errs=1
      }
    done < <(grep -E '^-[[:space:]]' <<<"$body")

    grep -qE '\[#[0-9]+\]\(https://github\.com/terpnetwork/terp-rs/pull/[0-9]+\)' <<<"$body" || {
      bad "$name.md: no PR link; expected ([#N](https://github.com/terpnetwork/terp-rs/pull/N))"
      errs=1
    }
  else
    # `none` still needs a specific reason. The marker is a comment, so it is
    # absent from the active stream; the reason is the first non-blank active
    # line. In a scaffold that line is itself inside a comment, so this finds
    # nothing and the unedited scaffold fails.
    local after
    after="$(grep -m1 -E '[^[:space:]]' <<<"$body" || true)"
    if [[ -z "$after" ]]; then
      bad "$name.md: '$NONE_MARK' with no reason on the following line"
      errs=1
    elif (( ${#after} < 25 )); then
      bad "$name.md: reason is too terse to review: '$after'"
      errs=1
    fi
  fi

  (( errs == 0 )) && ok "$name.md"
  return "$errs"
}

# --- main -------------------------------------------------------------------

case "${1:-}" in
  --help|-h) usage; exit 0 ;;
  --scaffold)
    if [[ -z "${2:-}" ]]; then red "--scaffold needs a PR number"; exit 2; fi
    scaffold "$2"
    exit 0
    ;;
  "") ;;
  *)
    red "unknown argument '$1' (try --help)"
    exit 2
    ;;
esac

if [[ ! -d "$DIR" ]]; then
  red "no ${DIR#$CRATE_ROOT/} directory."
  red "Every PR owns one fragment. Internal work too: scripts/ci/check-changelog.sh --scaffold <PR>"
  exit 1
fi

shopt -s nullglob
files=("$DIR"/*.md)
shopt -u nullglob
if (( ${#files[@]} == 0 )); then
  red "no fragments in ${DIR#$CRATE_ROOT/}."
  red "Every PR owns one. Internal work too: scripts/ci/check-changelog.sh --scaffold <PR>"
  exit 1
fi

echo "changelog fragments (${#files[@]}):"
rc=0
for f in "${files[@]}"; do
  validate_one "$f" || rc=1
done

if (( rc == 0 )); then
  echo "changelog ok"
fi
exit "$rc"
