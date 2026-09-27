#!/usr/bin/env bash
# emit_bridge_mint_fixture.sh — Tacit anvil → Domain B mint-packet fixture glue
#
# Round 3 (spectrum): produces a JSON file compatible with
# `cw-headstash::bridge::BridgeMintClaimPublic` + ReflectionSnapshot (mock LC).
#
# Modes:
#   1) Default / degrade: write the **pre-baked golden** hinge-happy fixture
#      (labels match bridge_auth_seams::hinge_happy_fixture + cw-headstash tests).
#   2) --regenerate: recompute SHA-256 label hashes (python3) and rewrite golden.
#   3) Optional anvil path: if forge/anvil/cast are available and --try-anvil is set,
#      run crates/tacit/tests/evm-confidential-anvil-roundtrip.sh and attach a
#      meta.tacit_anvil annotation. Does **not** invent SP1 proofs or real LC IMT.
#
# Usage:
#   bash docs/plans/spectrum/fixtures/emit_bridge_mint_fixture.sh
#   bash docs/plans/spectrum/fixtures/emit_bridge_mint_fixture.sh --out /tmp/mint.json
#   bash docs/plans/spectrum/fixtures/emit_bridge_mint_fixture.sh --regenerate
#   bash docs/plans/spectrum/fixtures/emit_bridge_mint_fixture.sh --try-anvil
#
# See: docs/plans/spectrum/agents/ROUND3-TACIT-GLUE.md
#      docs/plans/spectrum/SPEC-tacit-bridge-mapping.md §9
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
GOLDEN_DEFAULT="$SCRIPT_DIR/bridge_mint_claim_happy.v1.json"
OUT="$GOLDEN_DEFAULT"
REGENERATE=0
TRY_ANVIL=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT="${2:?}"; shift 2 ;;
    --regenerate) REGENERATE=1; shift ;;
    --try-anvil) TRY_ANVIL=1; shift ;;
    -h|--help)
      sed -n '2,24p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Docs: how to run Tacit anvil confidential roundtrip (independent of fixture)
# ---------------------------------------------------------------------------
cat <<'EOF' >&2
== Tacit EVM confidential anvil roundtrip (optional live path) ==
  cd crates/tacit
  # Requires: anvil, cast, forge, node + dapp deps
  bash tests/evm-confidential-anvil-roundtrip.sh
  # Proves: module prover → mint(tx) → supply/noteStatus on local anvil
  # Does NOT emit Domain B BridgeMintClaimPublic (Bitcoin reflection burn path).
  # This glue maps **known hinge-happy labels** into that claim shape for Terp.

EOF

# ---------------------------------------------------------------------------
# Optional anvil annotation
# ---------------------------------------------------------------------------
ANVIL_META_JSON='{"ran":false,"reason":"not requested"}'
if [[ "$TRY_ANVIL" -eq 1 ]]; then
  if command -v anvil >/dev/null 2>&1 \
    && command -v cast >/dev/null 2>&1 \
    && command -v forge >/dev/null 2>&1 \
    && command -v node >/dev/null 2>&1; then
    echo "== try-anvil: running tacit evm-confidential-anvil-roundtrip ==" >&2
    set +e
    ANVIL_LOG="$(mktemp)"
    (
      cd "$REPO_ROOT/crates/tacit"
      bash tests/evm-confidential-anvil-roundtrip.sh
    ) >"$ANVIL_LOG" 2>&1
    ANVIL_RC=$?
    set -e
    if [[ $ANVIL_RC -eq 0 ]]; then
      TOKEN="$(grep -E '^token:' "$ANVIL_LOG" | awk '{print $2}' | tail -1 || true)"
      SUPPLY="$(grep -E '^supply:' "$ANVIL_LOG" | awk '{print $2}' | tail -1 || true)"
      NOTE_STATUS="$(grep -E '^note status:' "$ANVIL_LOG" | awk '{print $3}' | tail -1 || true)"
      CX="$(grep -E '^note C\.x:' "$ANVIL_LOG" | awk '{print $3}' | tail -1 || true)"
      ANVIL_META_JSON=$(python3 - <<PY
import json
print(json.dumps({
  "ran": True,
  "ok": True,
  "script": "crates/tacit/tests/evm-confidential-anvil-roundtrip.sh",
  "token": """${TOKEN:-}""" or None,
  "supply": """${SUPPLY:-}""" or None,
  "note_status": """${NOTE_STATUS:-}""" or None,
  "note_cx": """${CX:-}""" or None,
  "note": "Anvil proves EVM confidential mint path only; Domain B claim fields remain hinge-happy golden (mock LC)."
}))
PY
)
      echo "anvil roundtrip OK (meta attached)" >&2
    else
      ANVIL_META_JSON='{"ran":true,"ok":false,"reason":"anvil script failed; degraded to golden claim"}'
      echo "anvil roundtrip FAILED (rc=$ANVIL_RC); degrading to golden fixture" >&2
      tail -n 40 "$ANVIL_LOG" >&2 || true
    fi
    rm -f "$ANVIL_LOG"
  else
    ANVIL_META_JSON='{"ran":false,"reason":"anvil/cast/forge/node not all available; using golden"}'
    echo "anvil tooling incomplete — golden fixture only" >&2
  fi
fi

# ---------------------------------------------------------------------------
# Emit / regenerate golden BridgeMintClaimPublic-compatible packet
# ---------------------------------------------------------------------------
if [[ "$REGENERATE" -eq 1 ]] || [[ ! -f "$GOLDEN_DEFAULT" ]]; then
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 required to regenerate golden fixture" >&2
    exit 1
  fi
  echo "== regenerate hinge-happy mint packet (SHA-256 labels) ==" >&2
  python3 - "$OUT" "$ANVIL_META_JSON" <<'PY'
import hashlib, json, struct, sys

out_path = sys.argv[1]
anvil_meta = json.loads(sys.argv[2])

def h32(label: str) -> bytes:
    return hashlib.sha256(label.encode()).digest()

def hx(b: bytes) -> str:
    return "0x" + b.hex()

def terp_asset_id(source_chain_tag: str, tacit_asset_id: bytes, unit_scale: int) -> bytes:
    x = hashlib.sha256()
    x.update(b"terp-tacit-asset-v1")
    x.update(source_chain_tag.encode())
    x.update(tacit_asset_id)
    x.update(struct.pack(">Q", unit_scale))
    return x.digest()

def derive_claim_id(dest_domain, dest_commitment, nu, asset_id, value: int) -> bytes:
    x = hashlib.sha256()
    x.update(b"terp-bridge-claim-v1")
    x.update(dest_domain)
    x.update(dest_commitment)
    x.update(nu)
    x.update(asset_id)
    x.update(struct.pack(">Q", value))
    return x.digest()

def derive_domain_binding(src, dst, lc, asset_id, claim_or_nu, height: int, commitment_root) -> bytes:
    x = hashlib.sha256()
    x.update(b"terp-private-bridge-v1")
    x.update(src)
    x.update(dst)
    x.update(lc)
    x.update(asset_id)
    x.update(claim_or_nu)
    x.update(struct.pack(">Q", height))
    x.update(commitment_root)
    return x.digest()

K = 6
MAX_LAG = 64
HEIGHT = 100
TIP = HEIGHT + K
VALUE = 1_000_000
UNIT = 1
SOURCE_TAG = "bitcoin-mainnet"

dest = h32("terp-chain-1")
tacit = h32("tacit-btc-etch-1")
terp_asset = terp_asset_id(SOURCE_TAG, tacit, UNIT)
nu = h32("nu-hinge-happy")
dest_cm = h32("dest-commitment-A")
pool_root = h32("pool-root-1")
spent_root = h32("spent-root-1")
burn_root = h32("burn-root-1")
src_chain = h32("src-bitcoin-mainnet")
dst_chain = dest
lc_client = h32("lc-client-reflection-0")
claim_id = derive_claim_id(dest, dest_cm, nu, tacit, VALUE)
domain_binding = derive_domain_binding(
    src_chain, dst_chain, lc_client, tacit, nu, HEIGHT, burn_root
)
rcm = h32("rcm-hinge-happy")
pool_domain = h32("terp-pool-0")
cm_public = h32("cm-leaf-dest-1")

field_docs = {
    "source_chain_tag": "Domain B §9 source_chain_tag (e.g. bitcoin-mainnet)",
    "tacit_asset_id": "Foreign/Tacit asset id (32B); registry maps → terp asset_id",
    "value_u64": "Opened burn value; conservation with mint",
    "nullifier": "ν of burned source note; once-per-ν via BRIDGE_MINTED",
    "dest_commitment": "Burn-bound destCommitment (A6)",
    "dest_domain": "Terp destination domain bind",
    "claim_id": "Re-derived H(terp-bridge-claim-v1 ‖ dest ‖ dest_cm ‖ ν ‖ asset ‖ value)",
    "source_pool_root": "Pool membership pin (bitcoinPoolRoot)",
    "source_burn_root": "Bridge-burn IMT root pin (bitcoinBurnRoot) — mint authority H-1",
    "source_height": "Source inclusion height H",
    "domain_binding": "C §3.1 domain bind digest",
    "unit_scale": "Asset map unit scale pin",
    "pool_domain": "Terp note-set / pool domain",
    "cm_public": "Destination leaf fingerprint for SEAM-NOTE-OUT",
    "rcm": "Optional Pedersen opening; non-zero → rcm_flag=1 (DEX-consumable)",
    "in_burn_set": "Mock LC: ν ∈ bridge-burn set (NOT generic spent set)",
    "in_pool_root": "Mock LC: note membership under pool root",
    "spent_only": "H-1 test flag; spent_only && !in_burn_set → reject",
    "src_chain_id": "Domain bind input (hash of source chain id label)",
    "dst_chain_id": "Domain bind input (dest domain)",
    "lc_client_id": "Domain bind input (reflection LC client id hash)",
    "burn_dest_commitment": "Burn-set recorded destCommitment (must equal dest_commitment)",
    "snapshot.pool_root": "Reflection snapshot bitcoinPoolRoot",
    "snapshot.spent_root": "Reflection snapshot bitcoinSpentRoot (never mint alone)",
    "snapshot.burn_root": "Reflection snapshot bitcoinBurnRoot (must match claim pin)",
    "snapshot.tip_height": "LC tip (≥ source_height + K)",
    "snapshot.confirmations_k": "Confirmation depth K",
    "snapshot.max_lc_lag": "Freshness lag residual after K",
    "snapshot.frozen": "Misbehaviour freeze — reject all mints",
    "cfg.mock_verify": "Round-2 stub; real SP1/LC not wired",
}

doc = {
    "schema": "bridge-mint-claim-public-v1",
    "version": 1,
    "description": (
        "Golden BridgeMintClaimPublic + ReflectionSnapshot + BridgeCfg for "
        "cw-headstash BridgeMintNote (mock LC). Labels match "
        "bridge_auth_seams::hinge_happy_fixture and cw-headstash bridge unit fixture."
    ),
    "ssot": [
        "docs/plans/spectrum/SPEC-tacit-bridge-mapping.md §9",
        "docs/plans/spectrum/SEAM-NOTE-OUT.md",
        "crates/headstash/contracts/cw-headstash/src/bridge.rs",
        "docs/plans/spectrum/fixtures/bridge_auth_seams",
    ],
    "source": {
        "kind": "golden-hinge-happy",
        "labels": {
            "dest_domain": "terp-chain-1",
            "tacit_asset": "tacit-btc-etch-1",
            "nullifier": "nu-hinge-happy",
            "dest_commitment": "dest-commitment-A",
            "pool_root": "pool-root-1",
            "spent_root": "spent-root-1",
            "burn_root": "burn-root-1",
            "src_chain": "src-bitcoin-mainnet",
            "lc_client": "lc-client-reflection-0",
            "rcm": "rcm-hinge-happy",
            "pool_domain": "terp-pool-0",
            "cm_public": "cm-leaf-dest-1",
        },
        "hash_fn": "sha256(label_utf8) for label_hash; domain tags per bridge.rs",
        "tacit_anvil": anvil_meta,
    },
    "field_docs": field_docs,
    "snapshot": {
        "pool_root": hx(pool_root),
        "spent_root": hx(spent_root),
        "burn_root": hx(burn_root),
        "source_height": HEIGHT,
        "tip_height": TIP,
        "confirmations_k": K,
        "max_lc_lag": MAX_LAG,
        "frozen": False,
    },
    "cfg": {
        "dest_domain": hx(dest),
        "confirmations_k": K,
        "max_lc_lag": MAX_LAG,
        "lc_client_id": "08-wasm-tacit-reflection-0",
        "mock_verify": True,
    },
    "asset": {
        "asset_id": hx(terp_asset),
        "local_denom": "ubtc",
        "origin": "tacit:bitcoin-mainnet",
        "status": "active",
    },
    "claim": {
        "source_chain_tag": SOURCE_TAG,
        "tacit_asset_id": hx(tacit),
        "value_u64": VALUE,
        "nullifier": hx(nu),
        "dest_commitment": hx(dest_cm),
        "dest_domain": hx(dest),
        "claim_id": hx(claim_id),
        "source_pool_root": hx(pool_root),
        "source_burn_root": hx(burn_root),
        "source_height": HEIGHT,
        "domain_binding": hx(domain_binding),
        "unit_scale": UNIT,
        "pool_domain": hx(pool_domain),
        "cm_public": hx(cm_public),
        "rcm": hx(rcm),
        "in_burn_set": True,
        "in_pool_root": True,
        "spent_only": False,
        "src_chain_id": hx(src_chain),
        "dst_chain_id": hx(dst_chain),
        "lc_client_id": hx(lc_client),
        "burn_dest_commitment": hx(dest_cm),
    },
    # Matches cw_headstash::bridge::mock_bridge_proof_bytes (non-empty mock).
    "proof_hex": "0xdeadbeef",
    "execute_sketch": {
        "msg": "BridgeMintNote",
        "claim_fields": "claim object above (Binary fields as 0x hex in this fixture)",
        "proof": "proof_hex — mock only; mock_verify must be true",
    },
}

with open(out_path, "w") as f:
    json.dump(doc, f, indent=2)
    f.write("\n")
print(out_path)
PY
else
  # Copy golden → OUT, patch tacit_anvil meta if needed
  if [[ "$OUT" != "$GOLDEN_DEFAULT" ]] || [[ "$TRY_ANVIL" -eq 1 ]]; then
    python3 - "$GOLDEN_DEFAULT" "$OUT" "$ANVIL_META_JSON" <<'PY'
import json, sys
src, dst, meta = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
with open(src) as f:
    doc = json.load(f)
doc.setdefault("source", {})["tacit_anvil"] = meta
with open(dst, "w") as f:
    json.dump(doc, f, indent=2)
    f.write("\n")
print(dst)
PY
  else
    echo "$GOLDEN_DEFAULT"
  fi
fi

echo "== fixture ready: $OUT ==" >&2
echo "Load in harness: zk-test-press::harness::load_bridge_mint_fixture" >&2
echo "Contract: cw-headstash ExecuteMsg::BridgeMintNote { claim, proof } (mock_verify)" >&2
