#!/usr/bin/env bash
# Local GHA for terp-rs combo. --bind so we do not docker-cp the monorepo.
#   ./scripts/ci/act-run.sh                 # offline job
#   ./scripts/ci/act-run.sh -j live-lcd
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

cleanup() {
  set +e
  if command -v docker >/dev/null 2>&1; then
    docker ps -aq --filter "name=act-" 2>/dev/null | xargs -r docker rm -f >/dev/null 2>&1 || true
    docker ps -aq --filter "label=act" 2>/dev/null | xargs -r docker rm -f >/dev/null 2>&1 || true
    docker network ls --format '{{.Name}}' 2>/dev/null | grep -E '^act-' | xargs -r docker network rm >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

if ! command -v act >/dev/null 2>&1; then
  echo "act is not installed (brew install act)" >&2
  exit 127
fi

export ACT=true
ARCH="${ACT_ARCH:-linux/arm64}"
IMAGE="${ACT_IMAGE:-ghcr.io/catthehacker/ubuntu:act-latest}"
WF="$ROOT/.github/workflows/combo.yml"

# --bind: mount this crate (do not copy sibling buzz/dregg/zec trees).
# --rm: drop the job container when the job ends.
act pull_request \
  --rm \
  --bind \
  --container-architecture "$ARCH" \
  -P "ubuntu-latest=$IMAGE" \
  --workflows "$WF" \
  "$@"
