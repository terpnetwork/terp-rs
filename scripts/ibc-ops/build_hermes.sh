#!/usr/bin/env bash
# Build terpnetwork/hermes:08-wasm from https://github.com/terpnetwork/hermes
# (feat/08-wasm = Informal romac/wasm-client with --wasm-checksum).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TAG="${HERMES_IMAGE_TAG:-terpnetwork/hermes:08-wasm}"
REPO="${HERMES_GIT_REPO:-https://github.com/terpnetwork/hermes.git}"
REF="${HERMES_GIT_REF:-feat/08-wasm}"

echo "==> docker build $TAG from $REPO@$REF"
docker build \
  --build-arg "HERMES_REPO=$REPO" \
  --build-arg "HERMES_REF=$REF" \
  -t "$TAG" \
  -f "$ROOT/scripts/ibc-ops/Dockerfile.hermes-wasm" \
  "$ROOT/scripts/ibc-ops"

echo "==> $TAG"
docker run --rm --entrypoint hermes "$TAG" --version
echo "Pin: HERMES_IMAGE_REPO=${TAG%%:*} HERMES_IMAGE_VERSION=${TAG##*:}"
