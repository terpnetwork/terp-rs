#!/usr/bin/env bash
# Exercise team: FROST coordinator + presig inventory + Noise + privval.
# Sound cases must pass. Unsound cases must fail closed.
# Not a morocco-1 halt. Not Phase G. Not a second kms ELF.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
KMS_SHARD="$(cd "$ROOT/../../../kms-shard" && pwd)"
TERP_RS="$(cd "$ROOT/../.." && pwd)"

run() {
  echo "==> $1"
  cargo test --features privval --lib "$2" -- --test-threads=1
}

echo "==> SOUND"
run "colocated t-of-n → dalek" colocated_2_of_3_signs_ed25519
run "presig earmark consume" inventory_earmark_and_consume
run "idle refill" refill_restores_after_consume
run "VoteSignBytes == Go" vote_sign_bytes_matches_cometbft_0_40
run "SignVote protobuf loopback" cmt_signvote_protobuf_loopback
run "JSON privval + HRS" json_loopback_sign_and_hrs
run "Noise XX lab" noise_xx_roundtrip

echo "==> UNSOUND (must fail closed)"
run "HRS cache + regression refuse" same_hrs_returns_cached
run "abort drops unused nonces" abort_pool_zeroizes_unused
run "prevote does not steal precommit slot" earmark_does_not_steal_precommit_for_prevote
run "drain all" drain_drops_all
run "stock formula" target_formula

echo "==> kms-shard scheme railguards"
(
  cd "$KMS_SHARD"
  cargo test --test dkg_sign dkg_2_of_3_signs_and_verifies_as_ed25519 -- --test-threads=1
  cargo test --test dkg_sign below_threshold_cannot_combine -- --test-threads=1
)

echo "==> ict-rs + cw-orch suite (L0 in-process + L1 mock, no Docker)"
(
  cd "$TERP_RS"
  cargo test -p terp-scripts --test frost_privval_e2e --features hash-market -- --test-threads=4
)

echo "==> e2e-frost-privval OK"
echo "Docs: crates/terp-rs/reviews/HANDOFF-FROST-SIDECAR-E2E-2026-09-03.md"
echo "Public draft: crates/terp-rs/tools/hash-market/docs/frost-privval.md"
echo "Manual fail: sidecar + priv_validator_key.json same pk; sidecar + Go kms same K_cons."
