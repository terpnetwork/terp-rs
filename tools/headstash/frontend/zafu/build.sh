#!/bin/sh
# Build ../zafu-client.js. WORK = a directory holding node_modules installed from package.json (npm install --ignore-scripts=false).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd); WORK=${WORK:-/tmp/hsfe-v2/zafu-client}
cp "$HERE/entry.js" "$WORK/entry.js"
cd "$WORK" && PATH="$(dirname "${NODE_BIN:-/opt/homebrew/bin/node}"):$PATH" ./node_modules/.bin/esbuild entry.js --bundle --format=esm --minify --legal-comments=eof --target=es2022 --metafile=meta.json --outfile="$HERE/../zafu-client.js"
