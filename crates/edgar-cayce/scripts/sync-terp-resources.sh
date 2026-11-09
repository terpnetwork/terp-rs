#!/bin/bash
# Sync known Terp resources into bot-ingestible snapshots.
#
# Materializes, under $TERPCORE/bot-sync/:
#   deps-cargo.md        direct dependency inventory per Cargo manifest
#   deps-cargo-tree.txt  transitive summary (cargo tree, best-effort)
#   docs-inventory.md    documentation file listing w/ sizes
#   chain-snapshot.json  live chain state via TERP_NODE_REST (if set)
#   state.json           copied live-chain static asset (if found in repo)
#   MANIFEST.md          what/when + suggested /edgar ingest-local commands
#
# Then, with the bot running and TERP_SYNC_ROOT=$TERPCORE:
#   /edgar ingest-local path:bot-sync/deps-cargo.md label:terp-deps
#   /edgar ingest-local path:bot-sync/docs-inventory.md label:terp-docs
#   /edgar ingest-local path:bot-sync/chain-snapshot.json label:terp-chain
set -u

ROOT="${TERPCORE:-$(cd "$(dirname "$0")/../../.." && pwd)}"
OUT="$ROOT/bot-sync"
mkdir -p "$OUT"
STAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

echo "== terp resource sync @ $STAMP =="
echo "root: $ROOT"

# ── 1. cargo dependency inventory (parsed, not raw lockfiles) ──
python3 - "$ROOT" "$OUT/deps-cargo.md" "$STAMP" <<'EOF'
import sys, tomllib
from pathlib import Path
root, out, stamp = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
manifests = sorted(root.glob("**/Cargo.toml"))
# skip vendored/lockfile-only noise
manifests = [m for m in manifests if ".cargo" not in str(m) and "checkouts" not in str(m) and ".worktrees" not in str(m)][:20]
lines = [f"# terp-core cargo dependencies", "", f"_synced {stamp}_", ""]
for m in manifests:
    try:
        data = tomllib.loads(m.read_text())
    except Exception as e:
        lines += [f"## {m.relative_to(root)}", f"(unparseable: {e})", ""]
        continue
    rel = m.relative_to(root)
    pkg = data.get("package", {}).get("name", "?")
    lines += [f"## {rel} (package `{pkg}`)"]
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        deps = data.get(section, {})
        if deps:
            lines += [f"### {section}"]
            for name in sorted(deps):
                spec = deps[name]
                if isinstance(spec, dict):
                    ver = spec.get("version", "")
                    src = spec.get("git", spec.get("path", ""))
                    extra = f" {ver} {src}".strip()
                    lines += [f"- {name} {extra}".rstrip()]
                else:
                    lines += [f"- {name} {spec}"]
    lines += [""]
out.write_text("\n".join(lines))
print(f"deps-cargo.md: {len(manifests)} manifests")
EOF

# ── 2. transitive tree (best-effort, needs cargo + lockfiles) ──
TREE_OK=0
{
  echo "# cargo tree (depth 2)"
  echo ""
  for crate in "$ROOT" "$ROOT/crates/edgar-cayce" "$ROOT/crates/wasmer-sdk"; do
    if [ -f "$crate/Cargo.toml" ]; then
      echo "## $crate"
      (cd "$crate" && cargo tree --depth 2 2>/dev/null | head -n 120) || echo "(cargo tree unavailable)"
      echo ""
      TREE_OK=1
    fi
  done
} > "$OUT/deps-cargo-tree.txt"
echo "deps-cargo-tree.txt written"

# ── 3. docs inventory ──
{
  echo "# terp-core documentation inventory"
  echo ""
  echo "_synced ${STAMP}_"
  echo ""
  find "$ROOT" \( -path "*/node_modules" -o -path "*/target" -o -path "*/.git" -o -path "*/dist" -o -path "*/.worktrees" \) -prune \
    -o -type f \( -iname "*.md" -o -iname "*.mdx" \) -print 2>/dev/null \
    | sort | head -n 300 | while read -r f; do
      size=$(wc -c < "$f" | tr -d ' ')
      echo "- ${f#$ROOT/} (${size} bytes)"
    done
} > "$OUT/docs-inventory.md"
echo "docs-inventory.md written"

# ── 4. live chain snapshot (static asset reflecting the chain) ──
if [ -n "${TERP_NODE_REST:-}" ]; then
  BASE="$(echo "$TERP_NODE_REST" | sed 's:/*$::')"
  {
    echo "{"
    echo "  \"synced\": \"$STAMP\","
    echo "  \"rest\": \"$BASE\","
    for ep in "cosmos/base/tendermint/v1beta1/blocks/latest|latest_block" \
               "cosmos/bank/v1beta1/supply|supply" \
               "cosmos/mint/v1beta1/inflation|inflation" \
               "cosmos/staking/v1beta1/pool|staking_pool" \
               "cosmos/gov/v1/proposals?proposal_status=2&pagination.limit=20&pagination.count_total=true|active_proposals"; do
      path="${ep%%|*}"; key="${ep##*|}"
      body=$(curl -s --max-time 10 "$BASE/$path" 2>/dev/null | head -c 60000)
      if [ -n "$body" ]; then
        echo "  \"$key\": $body,"
      else
        echo "  \"$key\": null,"
      fi
    done
    echo "  \"_truncated\": \"each section capped at 60KB\""
    echo "}"
  } > "$OUT/chain-snapshot.json"
  echo "chain-snapshot.json written (live)"
else
  echo '{"synced":"'"$STAMP"'","note":"TERP_NODE_REST not set — no live snapshot"}' > "$OUT/chain-snapshot.json"
  echo "chain-snapshot.json written (stub — set TERP_NODE_REST for live data)"
fi

# ── 5. state.json static asset (first match in repo) ──
STATE=$(find "$ROOT" -maxdepth 4 -name "state.json" -not -path "*/node_modules/*" -not -path "*/target/*" -not -path "*/.git/*" 2>/dev/null | head -n 1)
if [ -n "$STATE" ]; then
  cp "$STATE" "$OUT/state.json"
  echo "state.json copied from ${STATE#$ROOT/}"
else
  echo '{"note":"no state.json found in repo at sync time"}' > "$OUT/state.json"
  echo "state.json stub written (none found)"
fi

# ── 6. manifest with ingest commands ──
cat > "$OUT/MANIFEST.md" <<EOF
# bot-sync manifest — $STAMP
#
# Run the bot with TERP_SYNC_ROOT=$ROOT, then (admin):
/edgar ingest-local path:bot-sync/deps-cargo.md label:terp-deps
/edgar ingest-local path:bot-sync/deps-cargo-tree.txt label:terp-deps
/edgar ingest-local path:bot-sync/docs-inventory.md label:terp-docs
/edgar ingest-local path:bot-sync/chain-snapshot.json label:terp-chain
/edgar ingest-local path:bot-sync/state.json label:terp-chain
#
# Live on-chain reads inside the RLM loop need no sync:
# set TERP_NODE_REST and the loop can node_query() allowlisted LCD paths.
# Re-run this script on a cron for freshness; re-ingest supersedes versions.
EOF
echo "MANIFEST.md written"
echo "== done: $OUT =="
