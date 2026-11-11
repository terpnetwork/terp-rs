# DAO zk-voting — proposal-module extension (workstream A)

Code: `terp-rs/contracts/headstash/cw-vote-ceremony/src/{zk.rs,zk_tests.rs}`, wiring in `msg.rs`, `contract.rs` and `error.rs`.
Suite: `tests/suite/terp-orch/src/suites/privacy/{zk_voting.rs,vote_circuit.rs}` (`ZkVotingModule`, `ValarVoteProver`, fallback `DevVoteProver`);
circuit: `terp-rs/crates/voting-circuits-zakura` (valargroup voting-circuits 0.10.0 on the zakura Halo2 fork, `PORT.md`);
e2e wrapper `contracts/headstash/cw-vote-ceremony/e2e/tests/zk_dao_e2e.rs`.
This extends cw-vote-ceremony: it reuses its ceremony lifecycle and the shared `SPENT` nullifier set (one SSOT). With
`zk_proposals: None` the contract behaves exactly as before.

## Circuit (vote-sdk `vote_proof`, ZKP #2) — host format
- Host call: `deps.api.proof_instance_verify(cast_zkid, proof, instances)` (wasmvm, newest design).
- Instances are 11 × 32-byte little-endian Pallas Fp values, in host order:
  `0 van_nullifier, 1 r_vpk_x, 2 r_vpk_y, 3 vote_authority_note_new, 4 vote_commitment,
   5 vote_comm_tree_root, 6 anchor_height, 7 proposal_id (question index, = 1), 8 voting_round_id,
   9 ea_pk_x, 10 ea_pk_y`.
  (The vote-sdk FFI uses a compressed 9-chunk form; the host form above is the stored-circuit order.)
- The contract enforces slots 5–10 against the proposal, rejects a spent nullifier in slot 0, and requires slot 1/2 to be the
  r_vpk that signed `ZkCastSighash` (RedPallas SpendAuth, `redpallas_spendauth_verify`, with
  `compress(x,y)` = x with bit 255 set to the parity of y).
- `voting_round_id = to_fe(blake2b(ROUND_DOMAIN ‖ lp chain_id ‖ lp contract ‖ proposal_id_be))`, where
  `to_fe` clears the top 2 bits of byte 31. Each proposal is one round and is bound to chain and contract.

## Stored format and footer derivation (packages/zk footer.rs, bin/export_vk.rs)
- The 80-byte footer has the fields `prover_id, curve_id, k, i, param_len, cs_len, vk_len, param_checksum = sha256(params),
  vk_checksum = sha256(cs ‖ vk)`.
- vote_proof: `prover_id = 0` (Plonkish), `curve_id = 2` (VoteCommitment; 1 = Delegation, 3 = ShareReveal),
  `k = 11`, `i = 11`, `cs_len = body length`, `vk_len = 0` (unsplit).
  - **Correction (round 6):** k is **11**. vote-sdk (`valargroup/vote-sdk` 5f23ec18) pins voting-circuits **0.10.0**
    (git 68d69c6d), where `vote_proof::circuit::K = 11` (delegation 12, share_reveal 10). The 13 below came from the older
    0.9 README. Confirmed by keygen on the real circuit: params k = 11, 11 instance cells.
  - **Correction (round 5):** k was 14 here and in `CAST_K`. vote_proof's own `K` is 13 (voting-circuits
    `vote_proof::circuit::K = 13`, README). The footer k must equal the params k, so 14 would reject every real key.
    `export_vk` hardcoded 14 for all three circuits. It now reads k from the params header (delegation 14,
    vote_proof 13, share_reveal 11).
- `appstate = u32::from_be_bytes([prover, curve, k, 0])`, and the key bytes are `appstate.to_le_bytes() = [0x00, 11, 2, 0x00]`.
- `circuit_key (72 B) = param_key ‖ vk_key`, where each half is `appstate_le(4) ‖ checksum(32)`.
- On chain: when `cast_circuit_key` is pinned, `check_circuit_key` requires 72 bytes, the prefix `[0,11,2,0]` at
  [0..4] and [36..40], and byte-for-byte equality with `WasmQuery::CircuitInfo{zk_id}.circuit_key`.
  The stale `zk-vote-bridge` layout (circuit id stored in prover_id, which collides with Stwo=2/Flock=3) is rejected
  (`stale_or_mismatched_circuit_footer_is_rejected`).

## Module API (cw-vote-ceremony)
- `InstantiateMsg.zk_proposals: Option<ZkModuleInit{ dao?, voting_module?, cast_zkid, cast_circuit_key?,
  anchor_registry?, tally: Committee{group_vk} | Circuit{zkid, circuit_key?}, voting_period, threshold_bps,
  quorum_bps, require_rvpk_sig=true }>`. `dao` defaults to the instantiator (dao-core), and `voting_module` defaults to
  dao-core's `voting_module {}`.
- Execute messages:
  - `ZkPropose{title, description, msgs, ea_pk(64B), anchor: Registry{session_id,height}|Root{root,height}}`: the proposer needs power > 0, and the proposal snapshots `total_power_at_height`.
  - `ZkCastVote{proposal_id, proof, instances, ballot, rvpk_sig}`: any sender, so voters can use relayers.
  - `ZkSubmitTally{yes,no,abstain, attestation: Committee{sig}|Proof{proof}}`: only after expiration, with turnout ≤ total.
  - `ZkExecute`.
- Queries: `ZkConfig, ZkProposal, ZkListProposals, ZkBallots, ZkCastSighash, ZkTallyDigest`, plus the DAO DAO
  surface `Dao, Info, ProposalCount, NextProposalId`.
- Pass rule: quorum is `turnout·10⁴ ≥ total·quorum_bps`, and threshold is `yes·10⁴ > (yes+no)·threshold_bps`.
- Tally digest: `blake2b(TALLY_DOMAIN ‖ chain ‖ contract ‖ id ‖ round ‖ ballots_digest ‖ ballots ‖ y‖n‖a u128be)`. It commits to
  the running digest of all accepted ballots, so the EA cannot drop or add ballots. The tally-circuit instances are
  `[round, to_fe(digest), ballots, y, n, a]`.

## DAO integration
An existing DAO passes a proposal through its current module with dao-core `UpdateProposalModules{to_add:[{code_id,
msg: InstantiateMsg{zk_proposals: Some(..)}, admin: CoreModule}]}`. The zk module then runs alongside the existing
modules, and passed zk proposals call `execute_proposal_hook{msgs}` on dao-core. The e2e test proves this against the real
dao-core, dao-voting-cw4, proposal-single and pre-propose contracts, including a treasury payout and continued proposal-single operation.

## Cast circuit keys (round 6: valar's real vote_proof)
- **Source:** valargroup voting-circuits 0.10.0 (git 68d69c6d, as pinned by vote-sdk 5f23ec18), ported to the zakura Halo2 fork in
  `crates/voting-circuits-zakura` (orchard → zakura-orchard `circuit::composition`, rand 0.10, delegation not ported). The
  relation is unchanged (`PORT.md` §Faithfulness); the upstream test suite, the ignored long tests included, passes on the port.
- **Keys:** `VoteProofCircuit` (`#[circuit_interface(id = "vote_proof")]`) + `StoredCircuit::vote_proof()` take params,
  `keygen_vk` and `keygen_pk` from the port's process cache (the keys `build_vote_proof_from_delegation` proves with) and write
  `artifacts/vote_proof_{summary.json,vk.bin}` with `generate_footer` and `validate_combined_binary`. The asserts are K = `CAST_K` = 11,
  11 instances and the cast prefix `[0,11,2,0]`.
- **Prover:** `ValarVoteProver` builds a vote commitment tree of VAN leaves for the module's next `round_id` (`CastProver::election`),
  pins its root and the EA key in `ZkPropose`, and proves each ballot with `build_vote_proof_from_delegation` (16 ElGamal shares to the
  EA key). It signs `ZkCastSighash` with the randomized key behind `r_vpk`. The suite checks that 3 ballots are accepted, a nullifier
  replay is rejected, and a proof over an altered vote commitment is rejected.
- **Fallback:** `DevVoteProver` / `VoteProofDevCircuit` (the shape circuit below) stays for fast tests (`ZkVotingModule::with_prover`).
- Benchmarks: `BENCHMARKS.md [A]` (`tests/privacy_bench.rs`).

## Cast circuit keys (round 5, shape circuit — now the fallback)
- `VoteProofDevCircuit` uses `#[cw_orch::circuit_interface(id = "vote_proof_dev", artifacts_dir = "artifacts")]` +
  `CircuitFooterSpec`, the same derive as headstash / stwo_ssle / flock_ligerito. `VoteProofKeys::load_or_generate` does
  zakura-fork Halo2 keygen at `CAST_K` (11 since round 6), writes `artifacts/vote_proof_dev_summary.json` (`CircuitSummary`), builds the footer
  with `CircuitSummaryLoader::generate_footer`, writes `params ‖ CS ‖ VK ‖ footer` to `vk_path()` and checks it with
  `validate_combined_binary`. It also asserts that the 72-byte `circuit_key` has the cast prefix. Params are reused from the blob after the first run.
- `HaloVoteProver` (CastProver) proves with `create_proof`. On Mock it registers the blob's verifier for the cast zkid:
  `AnyVerifyingKey::try_from(blob)` + `AnyInstance::try_from_bytes(curve_id)` → `VoteVerifyingKey::verify`, the same dispatch
  as wasmvm. This replaces the 0xAA stand-in circuit everywhere (StubProver removed). The suite checks that real proofs are
  accepted, a nullifier replay is rejected, and a proof over an altered public input is rejected.
- On a live chain: `VoteProofDevCircuit::new(daemon).upload_circuit()` → zkid, then
  `ZkVotingModule::with_prover(Box::new(HaloVoteProver::new(zkid)?.pinned()))` pins the 72-byte key against `CircuitInfo`.
- **What is still not real:** the circuit is the vote_proof *shape* (11 public inputs, copy-constrained), not the
  vote_proof relation. voting-circuits' `vote_proof` builds on crates.io `halo2_proofs` 0.3, which has no
  `ConstraintSystem::write` / `VerifyingKey::write`, so its keys can't be put in the stored format. Fix: port
  voting-circuits to `zakura-halo2-proofs` (orchard → zakura-orchard with a voting-circuits feature). Swapping in that
  `Circuit` impl inside `VoteProofKeys` is then the only change; footer, blob, prover and verifier paths stay.

## Tally (decided for launch: FROST committee)
- Launch: the election-authority FROST committee (t-of-n group key, `TallyAuth::Committee`) decrypts the ElGamal
  ballots off chain and signs `ZkTallyDigest`, which commits to the running `ballots_digest` and ballot count. It cannot
  add or drop ballots, but **correctness of the decrypted totals is trusted to the committee**, and the committee
  sees the running tally.
- Committee size is configurable per module (`group_vk` of any FROST t-of-n). Launch should size it to the DAO, not to the
  suite's 2-of-3, which is a test fixture only.
- What a tally/decryption circuit adds (`TallyAuth::Circuit{zkid, circuit_key?}`):
  1. **No trust in the committee for correctness**: the proof shows the totals are the correct decryption of exactly the
     accepted ballots (DLEQ/decryption inside the circuit). A dishonest t-of-n can no longer misreport.
  2. **Verifiable aggregation**: the homomorphic sum of the ballots in `ballots_digest` is recomputed in-circuit,
     so anyone can check that totals = Σ ballots, with nothing replayed off chain.
  3. **Privacy of the running tally**: with threshold decryption of only the final aggregate (shares proven
     in-circuit), no party, the committee included, learns partial results before the window closes.
- Plug-in path: the same check that exists today. `ZkSubmitTally{attestation: Proof{proof}}` calls
  `proof_instance_verify(tally.zkid, proof, [round_id, to_fe(ballots_digest), ballots, yes, no, abstain])`
  (`TALLY_INSTANCES = 6`, `tally_circuit_path` test). No message or state change is needed. A DAO switches by instantiating
  (or migrating) the module with `TallyAuth::Circuit`, and the circuit is stored with the same `circuit_interface` flow.

## Trust and gaps
- Tally correctness and running-tally privacy are trusted to the EA committee until a tally circuit is plugged in (see Tally).
- Cast proofs are valar's real vote_proof (round 6): VAN membership, value range and ElGamal well-formedness are proven.
  Anchors are offline: the suite builds the VAN tree itself. A live election needs vote-sdk's delegation (ZKP #1) to build the
  tree, which is not ported, and `share_reveal` (ZKP #3) for a tally circuit.
- Unit tests (`zk_tests.rs`) keep a toy registered circuit for contract-only properties. Multi-test has no `MsgStoreCircuit` /
  `CircuitInfo`, so key pinning is exercised in unit tests and on live chains only.
