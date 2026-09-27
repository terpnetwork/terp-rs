#!/usr/bin/env bash
# Vendor Terp chain protos into crates/terp-rs/proto/ from the terp-core monorepo.
#
# crates/terp-rs/proto/terp and proto/osmosis are VENDORED COPIES. The chain is
# the source of truth; this repo only consumes the generated Rust. Editing a
# vendored .proto here is silently reverted the next time this script runs, so
# land proto changes in terp-core first and re-run the sync.
#
# Usage:
#   scripts/gen/proto/vendor-terp-protos.sh            # sync (write) + record
#   scripts/gen/proto/vendor-terp-protos.sh --check    # drift check, no writes
#   scripts/gen/proto/vendor-terp-protos.sh --help
#
# Environment:
#   TERP_CORE  terp-core checkout root. Default: <repo>/../.. (monorepo layout).
#   TSC_PROTO  fallback for x/clock, which terp-core dropped upstream.
#              Default: $TERP_CORE/tests/tsh/upgrade/terp-core.
#
# Exit codes: 0 ok / 1 drift or missing source / 2 bad usage.

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
PROTO_DIR="$REPO_ROOT/proto"
TERP_CORE="${TERP_CORE:-$(cd -- "$REPO_ROOT/../.." && pwd)}"
TSC_PROTO="${TSC_PROTO:-$TERP_CORE/tests/tsh/upgrade/terp-core}"
MANIFEST="$PROTO_DIR/vendor.sources.json"

MODE="sync"
case "${1:-}" in
  --check) MODE="check" ;;
  --help|-h)
    # Print the leading comment block, minus the shebang and the '#' gutter.
    sed -n '2,${/^#/!q;s/^# \{0,1\}//;p;}' "${BASH_SOURCE[0]}"
    exit 0
    ;;
  "") ;;
  *)
    echo "vendor-terp-protos: unknown argument '$1' (try --help)" >&2
    exit 2
    ;;
esac

die() { echo "vendor-terp-protos: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 1. Resolve the source ref once, so the manifest names an exact commit.
# ---------------------------------------------------------------------------
[ -d "$PROTO_DIR" ] || die "proto dir not found: $PROTO_DIR"
[ -d "$TERP_CORE/proto" ] || die "terp-core proto dir not found: $TERP_CORE/proto (set TERP_CORE)"

terp_core_ref="$(git -C "$TERP_CORE" rev-parse HEAD 2>/dev/null || echo unknown)"
terp_core_branch="$(git -C "$TERP_CORE" rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
terp_core_dirty="false"
if [ -n "$(git -C "$TERP_CORE" status --porcelain -- proto/ 2>/dev/null)" ]; then
  terp_core_dirty="true"
fi

# ---------------------------------------------------------------------------
# 2. Plan. Each entry is "dest_prefix|label|absolute_source_dir". Later entries
#    win, so a pinned fallback can shadow a tree that dropped a module.
# ---------------------------------------------------------------------------
CHAIN_LABEL="terp-core@$terp_core_ref"
PINNED_LABEL="pinned:tsh-upgrade-terp-core"
SYNC_TREES=(
  "terp|$CHAIN_LABEL|$TERP_CORE/proto/terp"
  "osmosis|$CHAIN_LABEL|$TERP_CORE/proto/osmosis"
  "terp/clock|$PINNED_LABEL|$TSC_PROTO/proto/terp/clock"
)

# buf-managed dependency lock. Copied so `buf dep update` here resolves the same
# BSR digests as the chain instead of drifting to another cosmos-sdk snapshot.
SYNC_FILES=(buf.lock)

# Only .proto is vendored. Everything else in a source tree (README, .DS_Store,
# buf module files) belongs to the chain repo and is deliberately not copied.
protos_under() { (cd "$1" && find . -type f -name '*.proto' | LC_ALL=C sort | sed 's|^\./||'); }

sha() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

declare -A SRC_OF=()
declare -A LABEL_OF=()
order=()
for entry in "${SYNC_TREES[@]}"; do
  IFS='|' read -r dst_rel label src_abs <<<"$entry"
  [ -d "$src_abs" ] || die "source tree missing: $src_abs"
  n=0
  while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    dest="$dst_rel/$rel"
    if [ -z "${SRC_OF[$dest]:-}" ]; then order+=("$dest"); fi
    SRC_OF["$dest"]="$src_abs/$rel"
    LABEL_OF["$dest"]="$label"
    n=$((n + 1))
  done < <(protos_under "$src_abs")
  [ "$n" -gt 0 ] || die "no .proto files under $src_abs"
done

# ---------------------------------------------------------------------------
# 3. Apply, or diff, the plan.
# ---------------------------------------------------------------------------
drift=()
copied=0

for dest in "${order[@]}"; do
  src="${SRC_OF[$dest]}"
  dst="$PROTO_DIR/$dest"
  if [ "$MODE" = "check" ]; then
    if [ ! -f "$dst" ]; then
      drift+=("$dest : not vendored")
    elif [ "$(sha "$src")" != "$(sha "$dst")" ]; then
      drift+=("$dest : content differs")
    fi
  else
    mkdir -p "$(dirname -- "$dst")"
    cp "$src" "$dst"
    copied=$((copied + 1))
  fi
done

# A vendored file the plan does not account for is drift too, otherwise a
# hand-edited or stale module survives every --check run unnoticed.
while IFS= read -r rel; do
  [ -n "$rel" ] || continue
  [ -z "${SRC_OF[$rel]:-}" ] && drift+=("$rel : vendored but not in terp-core")
done < <(
  protos_under "$PROTO_DIR/terp" | sed 's|^|terp/|'
  protos_under "$PROTO_DIR/osmosis" | sed 's|^|osmosis/|'
)

if [ "$MODE" = "sync" ]; then
  for f in "${SYNC_FILES[@]}"; do
    [ -f "$TERP_CORE/proto/$f" ] || die "source file missing: $TERP_CORE/proto/$f"
    cp "$TERP_CORE/proto/$f" "$PROTO_DIR/$f"
  done
fi

# ---------------------------------------------------------------------------
# 4. Manifest: source ref + per-file digest, so a reader can prove what landed.
# ---------------------------------------------------------------------------
if [ "$MODE" = "sync" ]; then
  i=0
  {
    printf '{\n'
    printf '  "generated_by": "scripts/gen/proto/vendor-terp-protos.sh",\n'
    printf '  "source_repo": "terp-core",\n'
    printf '  "source_ref": "%s",\n' "$terp_core_branch"
    printf '  "source_commit": "%s",\n' "$terp_core_ref"
    printf '  "source_proto_dirty": %s,\n' "$terp_core_dirty"
    printf '  "files": {\n'
    for dest in "${order[@]}"; do
      i=$((i + 1))
      sep=","
      [ "$i" -eq "${#order[@]}" ] && sep=""
      printf '    "%s": { "src": "%s", "sha256": "%s" }%s\n' \
        "$dest" "${LABEL_OF[$dest]}" "$(sha "$PROTO_DIR/$dest")" "$sep"
    done
    printf '  },\n'
    printf '  "notes": [\n'
    printf '    "protos are vendored verbatim; edit them in terp-core, not here",\n'
    printf '    "terp/clock is pinned from tests/tsh/upgrade/terp-core: terp-core dropped x/clock upstream but crates/sdk/src/gen/terp.clock.v1.rs still needs it",\n'
    printf '    "buf.lock is copied from terp-core so BSR deps track the chain"\n'
    printf '  ]\n'
    printf '}\n'
  } >"$MANIFEST"
fi

# ---------------------------------------------------------------------------
# 5. Report.
# ---------------------------------------------------------------------------
if [ "$MODE" = "check" ]; then
  if [ "${#drift[@]}" -gt 0 ]; then
    echo "vendor-terp-protos: DRIFT against $TERP_CORE ($terp_core_ref)" >&2
    for d in "${drift[@]}"; do echo "  $d" >&2; done
    echo "Run scripts/gen/proto/vendor-terp-protos.sh to resync." >&2
    exit 1
  fi
  echo "vendor-terp-protos: in sync with $TERP_CORE ($terp_core_ref), ${#order[@]} file(s)"
  exit 0
fi

echo "vendor-terp-protos: vendored $copied .proto file(s) from $TERP_CORE ($terp_core_ref)"
if [ "$terp_core_dirty" = "true" ]; then
  echo "vendor-terp-protos: WARNING source proto/ tree has uncommitted edits" >&2
fi
echo "vendor-terp-protos: wrote ${MANIFEST#"$REPO_ROOT/"}"
