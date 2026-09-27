#!/usr/bin/env bash
# Download the pinned linux ict-ci tarball. Does not compile ict-rs.
#   DEST=/tmp/ict-rs-bins scripts/ci/fetch-ict-rs-bins.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=ict-rs-bins.env
. "$HERE/ict-rs-bins.env"
DEST="${1:-${ICT_RS_BIN_DIR:-/tmp/ict-rs-bins}}"
URL="${ICT_RS_BINS_URL:?}"
NAME="${ICT_RS_TARBALL:?}"
WANT="${ICT_RS_TARBALL_SHA256:?}"
mkdir -p "$DEST"
TAR="$DEST/$NAME"
if [ ! -f "$TAR" ]; then
  echo "fetch $URL"
  curl -fsSL "$URL" -o "$TAR"
fi
GOT="$(shasum -a 256 "$TAR" | awk '{print $1}')"
if [ "$GOT" != "$WANT" ]; then
  echo "sha256 mismatch for $NAME: got $GOT want $WANT" >&2
  exit 1
fi
tar -C "$DEST" -xzf "$TAR"
chmod +x "$DEST/ict-ci" "$DEST/examples/"* 2>/dev/null || true
echo "ICT_CI_BIN_DIR=$DEST"
echo "ICT_RS_COMMIT=$ICT_RS_COMMIT"
