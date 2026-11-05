#!/usr/bin/env bash
# Copy the browser prover (vk_id 2 + vk_id 3) into the claim page and rewrite
# every pin: JS + WASM in worker.js, then JS + WASM + worker in headstashMembershipWasm.ts.
#
# Usage: headstash-prover-sync.sh [--build]
#   OUT (default /tmp/hs-bp-out) holds zk_headstash.js + zk_headstash_bg.wasm.
#   --build rebuilds OUT first via build-browser-prover.sh (DEST=-).
#   DAPP defaults to <terp-core>/websites/dao-dao-ui.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TERP_CORE="$(cd "$HERE/../../../../.." && pwd)"
DAPP="${DAPP:-$TERP_CORE/websites/dao-dao-ui}"
OUT="${OUT:-/tmp/hs-bp-out}"
PY="${PY:-/usr/local/bin/python3}"
if [ "${1:-}" = "--build" ]; then
  OUT="$OUT" "$HERE/build-browser-prover.sh" -
fi
DEST="$DAPP/apps/dapp/public/headstash-prover/membership"
TS="$DAPP/packages/stateful/components/pages/headstashMembershipWasm.ts"
for f in zk_headstash.js zk_headstash_bg.wasm; do
  test -s "$OUT/$f" || { echo "missing $OUT/$f" >&2; exit 1; }
done
for sym in prove_membership_witness_browser prove_note_witness_browser; do
  grep -q "$sym" "$OUT/zk_headstash.js" || { echo "export $sym missing" >&2; exit 1; }
done
cp "$OUT/zk_headstash.js" "$OUT/zk_headstash_bg.wasm" "$DEST/"
"$PY" - "$DEST" "$TS" <<'PY'
import hashlib, re, sys
dest, ts = sys.argv[1], sys.argv[2]
sha = lambda p: hashlib.sha256(open(p, 'rb').read()).hexdigest()
js, wasm = sha(f'{dest}/zk_headstash.js'), sha(f'{dest}/zk_headstash_bg.wasm')
def pin(path, name, value):
    s = open(path).read()
    s2, n = re.subn(r"(%s\s*=\s*')[0-9a-f]{64}(')" % name, r'\g<1>%s\g<2>' % value, s)
    if n != 1: raise SystemExit(f'{name} not found once in {path}')
    open(path, 'w').write(s2)
worker = f'{dest}/worker.js'
pin(worker, 'MEMBERSHIP_JS_SHA256', js)
pin(worker, 'MEMBERSHIP_WASM_SHA256', wasm)
wk = sha(worker)
pin(ts, 'MEMBERSHIP_JS_SHA256', js)
pin(ts, 'MEMBERSHIP_WASM_SHA256', wasm)
pin(ts, 'MEMBERSHIP_WORKER_SHA256', wk)
print(f'js {js}\nwasm {wasm}\nworker {wk}')
PY
