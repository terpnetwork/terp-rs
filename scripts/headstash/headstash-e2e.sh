#!/usr/bin/env bash
# Local Headstash e2e. Loopback only. Does not pull images and does not broadcast
# to a public chain. Refuses ICT_MOCK. Test keys are created inside the containers.
set -euo pipefail

ROOT="${TERP_CORE:-$HOME/abstract/terp-core}"
HS="$ROOT/crates/terp-rs/tools/headstash"
SC="$ROOT/crates/terp-rs/scripts/headstash"
TRS="$ROOT/crates/terp-rs"
ICT="$ROOT/crates/ict-rs"
DIR="${HEADSTASH_E2E_DIR:-/tmp/headstash-e2e}"
export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"
export DOCKER_HOST="${DOCKER_HOST:-unix:///Users/returniflost/.docker/run/docker.sock}"
export TERP_IMAGE_REPO="${TERP_IMAGE_REPO:-registry.terp.network/terp-core}"
export TERP_IMAGE_VERSION="${TERP_IMAGE_VERSION:-v6.4.0}"
export HERMES_IMAGE_REPO="${HERMES_IMAGE_REPO:-ghcr.io/informalsystems/hermes}"
export HERMES_IMAGE_VERSION="${HERMES_IMAGE_VERSION:-1.13.1}"
export ZK_ROOT="${ZK_ROOT:-$HOME/abstract}"
export HEADSTASH_E2E_DIR="$DIR"
export HEADSTASH_E2E_KEEP="${HEADSTASH_E2E_KEEP:-1}"

if [[ "${ICT_MOCK:-}" == "1" ]]; then
  echo "ICT_MOCK=1 is not a Headstash e2e" >&2
  exit 2
fi
if [[ -n "${ICT_PUBLISH_HOST:-}" && "${ICT_PUBLISH_HOST}" != "127.0.0.1" ]]; then
  echo "ICT_PUBLISH_HOST must be 127.0.0.1" >&2
  exit 2
fi
export ICT_PUBLISH_HOST="127.0.0.1"

mkdir -p "$DIR"
if ! docker info >/dev/null 2>&1; then
  echo "DOCKER_DOWN: start Docker Desktop on this host. No chain was started." >&2
  exit 2
fi
docker image inspect "${TERP_IMAGE_REPO}:${TERP_IMAGE_VERSION}" >/dev/null
docker image inspect "${HERMES_IMAGE_REPO}:${HERMES_IMAGE_VERSION}" >/dev/null

echo "== host prover (reuse a host-verified file, else K=18 Proof::create) =="
prove_status=0
reuse=0
if [[ -f "$DIR/claim_proof.json" ]]; then
  if python3 - "$DIR/claim_proof.json" <<'PY'
import json, os, sys
doc = json.load(open(sys.argv[1], encoding="utf-8"))
vk = doc.get("vk_path") or ""
proof = doc.get("proof_hex") or ""
ok = (
    doc.get("prover") == "zk_headstash::circuit::Proof::create"
    and doc.get("host_verify") is True
    and doc.get("k") == 18
    and isinstance(proof, str)
    and len(proof) >= 2
    and len(proof) % 2 == 0
    and bool(vk)
    and os.path.isfile(vk)
    and os.path.getsize(vk) > 1_000_000
)
raise SystemExit(0 if ok else 1)
PY
  then
    reuse=1
    echo "reusing host-verified claim_proof.json"
  fi
fi
if [[ "$reuse" -eq 0 ]]; then
  set +e
  bash "$SC/headstash-prove.sh"
  prove_status=$?
  set -e
fi
if [[ -f "$DIR/claim_proof.json" ]]; then
  HEADSTASH_VK_PATH="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("vk_path") or "")' "$DIR/claim_proof.json")"
  if [[ -n "$HEADSTASH_VK_PATH" && -f "$HEADSTASH_VK_PATH" ]]; then
    export HEADSTASH_VK_PATH
    echo "store-circuit blob is the prover vk $HEADSTASH_VK_PATH"
  fi
fi
ZKAPI_WASM="$HS/artifacts/cw_headstash_zkapi.wasm"
if [[ -f "$ZKAPI_WASM" ]]; then
  export HEADSTASH_WASM="$ZKAPI_WASM"
  echo "claim wasm is the zk-api build $ZKAPI_WASM"
fi
if [[ "$prove_status" -ne 0 ]]; then
  echo "prover exit ${prove_status}. positive_claim stays a gap. tail:"
  python3 - "$DIR/prove-error.txt" <<'PY'
import sys
text = open(sys.argv[1], encoding="utf-8", errors="replace").read() if __import__("os").path.exists(sys.argv[1]) else ""
print("\n".join(text.splitlines()[-15:]))
PY
fi

echo "== wavs proof of possession (public bytes only) =="
(
  cd "$TRS"
  cargo run -p cw-headstash --features interface --example print_wavs --quiet
) >"$DIR/wavs.json"
python3 - "$DIR/wavs.json" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))
poos = doc.get("poos") or []
if len(poos) != 1:
    raise SystemExit(f"wavs json expected 1 operator, got {len(poos)}")
print(f"wavs operators={len(poos)} threshold={doc['msg']['threshold']}")
PY

echo "== headstash local e2e =="
set +e
(
  cd "$ICT"
  cargo run -p ict-rs --example headstash_local_e2e --features docker --quiet
)
status=$?
set -e

cp "$HS/e2e/claim/index.html" "$DIR/index.html"
if curl -fsS -o /dev/null --max-time 2 http://127.0.0.1:19820/index.html; then
  echo "claim page already on http://127.0.0.1:19820/"
else
  python3 -m http.server 19820 --bind 127.0.0.1 --directory "$DIR" >"$DIR/http.log" 2>&1 &
  echo $! >"$DIR/http.pid"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl -fsS -o /dev/null --max-time 2 http://127.0.0.1:19820/index.html; then
      break
    fi
    sleep 0.3
  done
fi
curl -fsS -o /dev/null --max-time 2 http://127.0.0.1:19820/index.html
echo "claim page http://127.0.0.1:19820/  report $DIR/report.json"

# Loopback signer, then a headless click. HTTP 200 on the HTML is not the click.
if [[ -f "$DIR/endpoints.json" && -f "$DIR/broadcast.json" ]]; then
  # The signer reads endpoints.json at startup. An old process still has the previous run's ports.
  if [[ -f "$DIR/signer.pid" ]]; then
    oldpid="$(cat "$DIR/signer.pid" 2>/dev/null || true)"
    if [[ -n "${oldpid}" ]]; then
      kill "$oldpid" 2>/dev/null || true
    fi
  fi
  if command -v lsof >/dev/null 2>&1; then
    for p in $(lsof -nP -iTCP:19821 -sTCP:LISTEN -t 2>/dev/null || true); do
      cmd="$(ps -p "$p" -o command= 2>/dev/null || true)"
      case "$cmd" in
        *e2e/claim/signer.py*) kill "$p" 2>/dev/null || true ;;
      esac
    done
    sleep 0.3
  fi
  python3 "$HS/e2e/claim/signer.py" >"$DIR/signer.log" 2>&1 &
  echo $! >"$DIR/signer.pid"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl -s -o /dev/null --max-time 2 -X POST http://127.0.0.1:19821/broadcast -H 'content-type: application/json' -d '{}'; then
      break
    fi
    sleep 0.3
  done
  NODE=""
  for candidate in \
    "${HOME}/.nvm/versions/node/v22.14.0/bin/node" \
    /opt/homebrew/bin/node \
    "$(command -v node || true)"
  do
    if [[ -n "$candidate" && -x "$candidate" ]]; then
      NODE="$candidate"
      break
    fi
  done
  export NODE_PATH
  NODE_PATH=""
  for root in \
    "/tmp/headstash-playwright/node_modules" \
    "${HOME}/abstract/terp-core/crates/terp-rs/tools/headstash/e2e/claim/node_modules" \
    "${HOME}/abstract/terp-core/websites/dao-dao-ui/node_modules" \
    "${HOME}/abstract/terp-core/node_modules"
  do
    if [[ -d "$root/playwright" ]]; then
      NODE_PATH="$root"
      break
    fi
  done
  if [[ -n "$NODE" && -n "$NODE_PATH" ]]; then
    set +e
    HEADSTASH_CLAIM_URL="http://127.0.0.1:19820/" \
      "$NODE" "$HS/e2e/claim/broadcast.spec.cjs" >"$DIR/playwright.log" 2>&1
    click_status=$?
    set -e
    if [[ "$click_status" -ne 0 ]]; then
      echo "playwright click failed. See $DIR/playwright.log" >&2
      status=1
    else
      echo "playwright click ok"
    fi
  else
    echo "playwright not installed (node=${NODE:-missing}). Click was not run." | tee "$DIR/playwright.log" >&2
    status=1
  fi
else
  echo "broadcast.json missing. Playwright was not run." | tee "$DIR/playwright.log" >&2
  status=1
fi
exit "$status"
