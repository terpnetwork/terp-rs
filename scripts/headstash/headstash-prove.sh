#!/usr/bin/env bash
# Host K=18 prove. Writes claim_proof.json. Does not broadcast and does not
# treat cw-orch Mock as a claim. A compile or runtime failure is the gap.
set -euo pipefail

ROOT="${TERP_CORE:-$HOME/abstract/terp-core}"
DIR="${HEADSTASH_E2E_DIR:-/tmp/headstash-e2e}"
OUT="${HEADSTASH_CLAIM_PROOF:-$DIR/claim_proof.json}"
export PATH="/usr/local/bin:/opt/homebrew/bin:${HOME}/.cargo/bin:${PATH}"
export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-8}"
export HEADSTASH_PROVE_ONLY=1
export HEADSTASH_CLAIM_PROOF="$OUT"
mkdir -p "$DIR"
rm -f "$OUT" "$DIR/prove-error.txt"

if [[ "${ICT_MOCK:-}" == "1" ]]; then
  echo "ICT_MOCK=1 is not a proof" | tee "$DIR/prove-error.txt" >&2
  exit 2
fi

set +e
(
  cd "$ROOT/crates/terp-rs"
  cargo test -p terp-orch --features headstash --test export_claim_proof -- --ignored --nocapture
) >"$DIR/prove.log" 2>&1
status=$?
set -e

python3 - "$DIR/prove.log" "$DIR/prove-error.txt" <<'PY'
import sys
src, dst = sys.argv[1], sys.argv[2]
lines = open(src, encoding="utf-8", errors="replace").read().splitlines()
kept = []
for line in lines:
    low = line.lower()
    if "mnemonic" in low or "seed phrase" in low or "private key" in low:
        continue
    kept.append(line)
tail = "\n".join(kept[-40:])
open(dst, "w", encoding="utf-8").write(tail + "\n")
PY

if [[ "$status" -ne 0 || ! -s "$OUT" ]]; then
  echo "prover exit ${status}. See $DIR/prove-error.txt" >&2
  exit "$status"
fi

python3 - "$OUT" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1], encoding="utf-8"))
if doc.get("prover") != "zk_headstash::circuit::Proof::create" or doc.get("host_verify") is not True:
    raise SystemExit("proof file is not a host-verified Proof::create output")
proof = doc.get("proof_hex") or ""
if len(proof) < 2 or len(proof) % 2:
    raise SystemExit("proof hex missing")
print(f"proof_bytes={len(proof)//2} denom={doc.get('denom')} value={doc.get('value')} k={doc.get('k')}")
PY
