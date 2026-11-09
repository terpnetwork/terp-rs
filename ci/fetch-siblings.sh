#!/usr/bin/env bash
# Materialise the out-of-repo path dependencies that the workspace (and its forks) expect next to terp-rs.
# Each line of ci/siblings.tsv: <dest relative to repo root> <git url> <full commit sha>. Pinned by rev; no branches.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
while read -r dest url rev; do
  case "$dest" in ''|\#*) continue;; esac
  dir="$root/$dest"
  if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$rev" ]; then continue; fi
  if [ -e "$dir" ] && [ ! -d "$dir/.git" ]; then echo "skip $dest (exists, not a git checkout)"; continue; fi
  mkdir -p "$dir"
  git -C "$dir" init -q
  git -C "$dir" remote add origin "$url" 2>/dev/null || git -C "$dir" remote set-url origin "$url"
  git -C "$dir" fetch -q --depth 1 origin "$rev"
  git -C "$dir" checkout -q --detach FETCH_HEAD
  echo "pinned $dest @ ${rev:0:10}"
done < "$root/ci/siblings.tsv"
