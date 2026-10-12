#!/usr/bin/env bash
# Init git submodules for Zakura and ztreamer. Depth 1 — do not fetch full history.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if [[ ! -d .git ]]; then
  git init
fi
git submodule update --init --depth 1 --recursive || {
  echo "submodule update failed; adding remotes"
  git submodule add --depth 1 https://github.com/zakura-core/zakura.git runtimes/zakura || true
  git submodule add --depth 1 https://github.com/distractedm1nd/ztreamer.git runtimes/ztreamer || true
}
echo "runtimes: loopback compact only, never zec.rocks"
