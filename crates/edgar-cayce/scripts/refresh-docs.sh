#!/bin/bash
# Re-ingest the public docs corpus into the sqlite store.
#
# Reuses `discord-demo ingest` (src/main.rs `cli_ingest` → `docs::ingest`).
# That entrypoint always writes ./data/docs/terp-docs.sqlite relative to the
# process cwd, so this script runs the binary in a temp directory, snapshots
# a single consistent database file, and atomically replaces the live file.
#
# Env (names only; this script does not print their values):
#   TERP_DOCS_SQLITE  destination sqlite
#                     default: /opt/terp-bot/data/docs/terp-docs.sqlite
#   TERP_DOCS_URL     source passed to `ingest --url`
#                     default: https://terp.network
#   TERP_DOCS_LABEL   corpus label
#                     default: terp
#   EDGAR_BIN         optional path to the edgar / discord-demo binary
#
# A github.com URL uses the github ingest path; anything else uses URL ingest
# (llms-full.txt / llms.txt / markdown). Failure leaves the live file in place.
# Examples only when paired with deploy/terp-docs-refresh.*; this script does
# not install units or restart services.
set -euo pipefail

DEST="${TERP_DOCS_SQLITE:-/opt/terp-bot/data/docs/terp-docs.sqlite}"
URL="${TERP_DOCS_URL:-https://terp.network}"
LABEL="${TERP_DOCS_LABEL:-terp}"

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORKDIR="$(mktemp -d)"
STAGE=""

cleanup() {
  rm -rf "$WORKDIR"
  if [[ -n "${STAGE}" && -f "${STAGE}" ]]; then
    rm -f "${STAGE}"
  fi
}
trap cleanup EXIT

pick_bin() {
  if [[ -n "${EDGAR_BIN:-}" ]]; then
    if [[ ! -x "${EDGAR_BIN}" ]]; then
      echo "refresh-docs: EDGAR_BIN is not executable" >&2
      exit 1
    fi
    printf '%s\n' "${EDGAR_BIN}"
    return
  fi
  local c
  for c in edgar discord-demo; do
    if command -v "$c" >/dev/null 2>&1; then
      command -v "$c"
      return
    fi
  done
  local target="${CARGO_TARGET_DIR:-$REPO/target}"
  local cand
  for cand in \
    "$target/release/discord-demo" \
    "$target/debug/discord-demo" \
    "/opt/terp-bot/bin/edgar" \
    "/opt/terp-bot/bin/discord-demo"
  do
    if [[ -x "$cand" ]]; then
      printf '%s\n' "$cand"
      return
    fi
  done
  echo "refresh-docs: no edgar binary (set EDGAR_BIN or build discord-demo)" >&2
  exit 1
}

BIN="$(pick_bin)"
mkdir -p "$WORKDIR/data/docs"

echo "refresh-docs: ingest via cli_ingest (TERP_DOCS_URL, TERP_DOCS_LABEL)" >&2
(
  cd "$WORKDIR"
  "$BIN" ingest --url "$URL" --label "$LABEL"
)

BUILT="$WORKDIR/data/docs/terp-docs.sqlite"
if [[ ! -s "$BUILT" ]]; then
  echo "refresh-docs: ingest did not write a sqlite file; live corpus unchanged" >&2
  exit 1
fi

DEST_DIR="$(dirname -- "$DEST")"
mkdir -p -- "$DEST_DIR"
STAGE="$(mktemp "${DEST_DIR}/.terp-docs.sqlite.XXXXXX")"

# Snapshot to one file so the swap does not depend on -wal/-shm sidecars.
python3 - "$BUILT" "$STAGE" <<'PY'
import sqlite3, sys
src, dst = sys.argv[1], sys.argv[2]
s = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
d = sqlite3.connect(dst)
with d:
    s.backup(d)
d.close()
s.close()
PY

# Publish by rename on the destination filesystem (atomic on the same volume).
mv -f -- "$STAGE" "$DEST"
STAGE=""
# Stale sidecars belong to the previous inode. A new opener must not replay them.
rm -f -- "${DEST}-wal" "${DEST}-shm"
echo "refresh-docs: swapped corpus onto TERP_DOCS_SQLITE" >&2
