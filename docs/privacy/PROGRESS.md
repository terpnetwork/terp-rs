# Privacy products — progress

Launch order (owner): auctions demo → DAO zk-voting → bridge → seams, built in parallel; all synthesise into the single suite (`e2e/SUITE.md`). All work uncommitted in terp-rs `feat/mint-desk-v98`.

<!-- Workstream A: add your sections (Auctions / DAO zk-voting) below this line; B does not edit them. -->

## [A] Private auctions — DONE (Mock); ict-rs hookup pending
- svg-auction private extension: per-bid FROST DKGs (B3/P2/S1 shares, reclaim 5-of-6, finalize 6-of-6). Spec: auctions/SPEC.md.
- Unit tests: 17/17 pass (12 new). Clippy is clean, the wasm32 release build succeeds, and the schema is regenerated.
- E2E: `cargo test -p svg-auction-e2e --test private_auction_e2e` passes (cw721-svg mint, 3 sealed bids, B+P reclaim, P+S refusal,
  B+P+S finalize with fee/seller/refund/NFT asserts, solo reclaim after close). `private_auction_flow<Chain: CwEnv>` is ready for ict-rs.
- Blocker: not yet called from B's ict-rs harness (the flow lives in an integration-test file and needs a lib home to be shared).
- 2026-11-10 update: the flow now lives in terp-orch `suites/privacy/auctions.rs` (`PrivateAuctionModule`, `private_auction_flow<C: CwEnv>`), which fills the `auctions-frost` slot.
  It runs with no waits and uses roles platform = default sender, seller = op0, bidders = op1/op2. The offline suite and `svg-auction-e2e` (now a thin wrapper) pass, and `privacy-live` compiles (not run).
- 2026-11-10 round 5 (owner decisions): settlement is **private by default in shielded ZEC**. No Terp funds, amounts, deposit floor or destinations on chain.
  The bid escrow is a ZEC note owned by the 5-of-6 key (`escrow_cm`). Reclaim/Finalize record a `ShieldedRelease` (notes_cm over terp-seams SeamNoteOutV0 + OVK-encrypted envelopes + ovk_tag), bound into the signed digest. Finalize moves only the NFT. `DisclosePrivateRelease{ovk}` is the optional public reveal.
  The Zcash leg reuses frost_escrow Object A + Object B spend packages, FROST-signed by the bid group and verified (`verify_escrow_release_encoded` / `verify_spend_encoded`).
  The bidder solo reclaim is kept (decided) and records the {B,P} refund pre-signed at bid time. The seller may cancel only with no live bids.
  Tests: svg-auction 17/17 (12 private), svg-auction-e2e passes, offline suite passes (incl. OVK disclose + decrypt check), clippy clean, wasm32 ok, schema regenerated. Spec: auctions/SPEC.md §Settlement, §Exchange order.
  Residual: NFT↔ZEC is not atomic. P coordinates the order (pay, then Finalize), and B+P collusion against the seller stays open until Finalize is gated on a Zcash payment proof (light client, B).

- 2026-11-10 round 6: **seller view + atomic settlement.**
  - Each bid carries `memo_cm`, which labels chain, contract, lot and bid index. The contract recomputes it.
  - `ovk_tag` fixes the bid's OVK. A `sealed_bid` envelope (X25519 + XChaCha20-Poly1305, aad = memo_cm) gives the seller the OVK and the commitment opening. The seller opens and ranks all bids; third parties and replayed labels can't.
  - **Finalize now needs a Confirmed zap-1 payment inclusion** from `payment_client` (B's `cw-lc-zcash` `PaymentInclusion`, mirrored field-for-field and checked against `cw_lc_zcash::msg`): note = signed `payment_cm`, group = bid reclaim key, leaf = `program_entry("tsa1:"‖memo_cm)`, one settle per `payment_ref`. Preconfirmed/Reorged don't release. Solo reclaim and 6/6 kept.
  - Offline uses `MockZcashLc`. TODO: drive B's live header attestation + `VerifyPaymentInclusion` from the live flow (client shipped).
  - Design: auctions/ATOMICITY.md (seam curated for IBC v2/CosmWasm).
  - Tests: svg-auction 20/20, terp-orch bid_view 3/3, offline suite 1/1 (4 slots), svg-auction-e2e passes. Clippy is clean on touched packages.
  - Size: svg_auction 491,487 B (60% of limit). Gas: BENCHMARKS.md [A] (bid ≈57k, finalize ≈83k, reclaim ≈75k SDK gas, lower bound).
  - Cargo.lock is `--locked`-clean for svg-auction (zap1-verify, cosmwasm-vm dev-dep). The B-reported stale lock was mid-edit; no broad update was run.

## [A] DAO zk-voting — DONE (multi-test, stand-in circuit); real proof fixtures pending
- cw-vote-ceremony zk proposal module: footer-pinned vote_proof (prover 0, curve 2, k 14, i 11; key prefix [0,14,2,0]) via
  proof_instance_verify, plus a FROST EA tally. Spec: zk-voting/SPEC.md.
- Unit tests: 19/19 pass (9 new). Clippy is clean, the wasm32 release build succeeds, and the schema is regenerated.
- E2E: `cargo test -p cw-vote-ceremony-e2e --test zk_dao_e2e` passes (real cw4 DAO adds the module via proposal-single
  UpdateProposalModules, then runs propose, 3 relayed anonymous casts, a replay rejection, the tally, and a treasury payout; proposal-single keeps working).
- Blocker: a real vote_proof needs vote-sdk keygen and proof fixtures at k=14, plus MsgStoreCircuit on the ict-rs chain.
- Ledger: ledger/A-touched-files.md, A-git-status-{before,after}.txt, A-Cargo.lock.before.
- 2026-11-10 update: the flow now lives in terp-orch `suites/privacy/zk_voting.rs` (`ZkVotingModule`, `deploy_dao`, `add_zk_module`, `CastProver`), which fills the `dao-zk-voting` slot.
  It uses a cw-orch generic DAO DAO (core, cw4 voting, proposal-single, pre-propose). Mock uses `StubProver`. On a live chain, pass `ZkVotingModule::with_prover(..)`; without one the slot reports Skipped after the DAO-integration half.
  The offline suite and `cw-vote-ceremony-e2e` (now a thin wrapper) pass, and `privacy-live` compiles (not run).
- 2026-11-10 round 5: **k corrected 14 → 13** (vote_proof `K = 13`; the footer k must equal the params k). `CAST_K`, tests and spec now use prefix [0,13,2,0]. zk `export_vk` reads k from the params header instead of hardcoding 14.
  Real cast keys: `VoteProofDevCircuit` (`cw_orch::circuit_interface` + `CircuitFooterSpec`) with zakura-Halo2 keygen at k=13 / i=11 writes `artifacts/vote_proof_dev_{summary.json,vk.bin}` via `generate_footer` and `validate_combined_binary`. `HaloVoteProver` creates real proofs. The Mock host verifies them through `AnyVerifyingKey` → `VoteVerifyingKey::verify` (the wasmvm dispatch). StubProver is removed, and the suite also rejects a proof over an altered public input.
  Tally decision: the FROST EA committee signs the tally for launch (size configurable; 2-of-3 is only the suite fixture). A tally/decryption circuit would add committee-independent correctness, verifiable aggregation and running-tally privacy. It plugs in via `TallyAuth::Circuit` on the same `proof_instance_verify` check with 6 instances, with no message changes (zk-voting/SPEC.md §Tally).
  Blocker: the circuit is the vote_proof *shape*, not the relation. voting-circuits is on crates.io halo2_proofs 0.3 (no CS/VK write), so it must be ported to zakura-halo2-proofs before real vote_proof keys can be stored. Check K against voting-circuits 0.10.0 (only 0.9.0-rc.3 is local).
  Tests: cw-vote-ceremony 19/19, cw-vote-ceremony-e2e passes (~10 s with real proofs), offline suite 4 passed, privacy-live compiles (not run), clippy clean.

- 2026-11-10 round 6: **valar's real vote_proof is in.**
  - voting-circuits 0.10.0 (git 68d69c6d, as pinned by vote-sdk 5f23ec18) is ported to the zakura Halo2 fork as `crates/voting-circuits-zakura` (PORT.md; delegation not ported). Upstream tests: 124 pass, long ones ignored by default.
  - **K corrected 13 → 11** (0.10.0 `vote_proof::K = 11`; 11 public inputs; prefix [0,11,2,0]). `CAST_K`, the zk_tests assert, export_vk comment and SPEC are updated.
  - `VoteProofCircuit` (circuit_interface id `vote_proof`) stores the real keys. `ValarVoteProver` builds the VAN tree per round and proves with `build_vote_proof_from_delegation`. It is the Mock default; `DevVoteProver` shape circuit kept as fallback.
  - Bench: keygen 0.98 s, prove 0.77 s, verify 31 ms, proof 11,008 B, cast ≈142k SDK gas (BENCHMARKS.md [A]).
  - Tests: cw-vote-ceremony 19/19, cw-vote-ceremony-e2e passes, offline suite passes. Clippy is clean.
  - Gaps: live anchors need vote-sdk delegation (ZKP #1, not ported); a tally circuit needs share_reveal.
- 2026-11-10 round 6 **blocker:** groot-wan's disk is full (100%, ~120 MiB free; terp-rs target/ 108 G, kache cache 163 G). These are blocked until space is freed (owner call; A deleted nothing):
  - `privacy-live --no-run`;
  - the post-clippy wasm re-measure;
  - the schema regen for svg-auction.
- 2026-11-10 18:40Z **unblocked** (owner approved).
  - Cleanup: `kache clean --cache` (160.3 GiB, 81,014 entries) and removed terp-rs `target/debug` only. Free space on Data went from 225 MiB to 135 GiB.
  - `privacy-live --no-run` passes.
  - The svg-auction schema is regenerated (`memo_cm`, `ovk_tag`, `payment_ref`, `payment_cm`, policy `memo_domain`/`payment_memo_prefix`).
  - The svg_auction wasm is unchanged at 491,487 B, and cosmwasm-check passes.

## [B] Zcash IBC v2 light client
- Spec: `light-client/SPEC.md`. Implemented `contracts/light-clients/cw-lc-registry` (5 tests, clippy clean, wasm32 ok).
- Next: cw-lc-aggregator (FROST preconf), cw-lc-rotation, 08-wasm client; build wasm artifact.
- 2026-11-10 update: dynamic committee landed. `cw-lc-rotation` derives each epoch's committee from the registry operator set (weight → FROST shares, threshold = ceil(2/3 of shares), no single member ≥ t), with FE01 (frost_escrow) / TC01 (threshold_committee) keys, ack + proof + hand-off activation, and rotation on RotateShard / liveness fault / equivocation. `cw-lc-aggregator` takes proactive per-block preconfs, checks parent links, records equivocation evidence, confirms against the registry k-of-n tip at depth, and falls back to that tip when the committee is absent or stalled. The registry now checks the hm-vext capability quorum root (gRPC HashRoot). Tests: registry 7, rotation 2, aggregator integration 2, all passing; clippy is clean and the wasm32 check is ok. Spec: `light-client/SPEC.md` §3–6.
- Needs: hm-vext must accept the `/terp.hashmerchant.v1.Query/HashRoot` query for wasm (ideally plus a CapabilityQuorum query); the optimizer must build artifacts; the 08-wasm client is still to do.
- 2026-11-10 round 6: **the IBC v2 client is started: `contracts/light-clients/cw-lc-zcash`** (role `client`).
  - Setup: an operator registers the client in the registry (`RegisterClient` with attestors, then `BindIbcClient`). The client binds registry + aggregator + `ibc_client_id`, and proxies `ConsensusTip` / `NodeStates` (zakura node state) / `LatestHeader`.
  - **Interface for [A] (atomic auction settlement):** zap-1 payment inclusion. `ExecuteMsg::VerifyPaymentInclusion(PaymentInclusionMsg)` checks:
    - the leaf, Merkle path and IBC v2 `app_data_hash = H(note‖amount‖frost_group‖leaf)`, using the existing `cw-ics08-wasm-zap1::verify_zap1_ibcv2`;
    - against the committee-attested `payment_root` of the aggregator header at `height`. That header must be Confirmed unless `accept_preconfirmed`.
    - It has a `(source_client, sequence)` replay guard.
  - Settlement contracts query `QueryMsg::PaymentInclusion {app_data_hash_hex}`. It returns `Option<{inclusion, status: Preconfirmed|Confirmed|Reorged}>`; status is re-checked live, so release only on `Confirmed`. Types: `cw_lc_zcash::msg` (feature `library`).
  - Aggregator `Header` gained an optional `payment_root` (hex32, in the preconf digest).
  - Tests: zcash 1 (lifecycle: no header / not final / no root / bad root / tampered / replay / reorg), aggregator 2, rotation 2, registry 7, frost_escrow 8, threshold_committee 10. Clippy is clean and wasm32 is ok.
- Round 6 benchmarks: see `BENCHMARKS.md` [B]. All B wasm files are ≤ 475 KiB (limit 800). VM gas harness `cw-lc-zcash/tests/vm_gas.rs`; FROST timing `frost_escrow/tests/timing.rs`.
- Limits / next:
  - zap1's `hosting_payment` leaf hard-codes month 8 / year 2026 (an existing-crate limitation, left unchanged). A real auction leaf kind (e.g. `Transfer`) needs a zap1 tweak; owner call.
  - Consider confirming by chain_work on top (Himess/seam `confirmationWork`) instead of depth. In-contract Equihash checking is the trust-minimised option: ~576k EVM gas per header in seam.
  - Still needs hm-vext acceptedQueries for HashRoot.

## [B] tacit BTC→ETH→Terp bridge
- Spec: `bridge/SPEC.md`. Pending slot in suite. Not implemented.
- 2026-11-10 update: tacit is ON HOLD (owner). The bridge will be powered by the team's FCR integration draft, the next big step. The draft lives in `~/grok-bot/xchain-liquidity-implementation-plan.html` on the Mac (WP1 hashmerchant FCR attestation, WP2 IBC v2 client with FCR ≤ 1M cap / finalized above). Not implemented. The suite slot is renamed `bridge-fcr`. See `bridge/SPEC.md`.

## [B] terp-seams
- `seams/RECOVERY.md`. dex APIs + real committee crates restored; test-press compiles; 52/54 harness tests pass.
- 2026-11-10 update: the harness failures are root-caused, see `seams/RECOVERY.md` §Fixes. (1) The golden-file default path now points at `tests/private-bridge/zakura/` (clean file, applied). (2) The two failures come from a cross-module env race plus a wrong assertion. A shared `CORRIDOR_TEST_ENV_LOCK` is applied in clean files; the edits to user-dirty files are a proposed patch only, `ledger/B-proposed-harness-fixes.patch`, verified in a scratch copy.
- 2026-11-10 round 6: the approved harness patch is applied (pre-apply copies are in `ledger/pre-apply-r6/`). `zk-test-press --lib --features seams-escrow`: 133 passed / 0 failed / 1 ignored, ×3 consecutive runs. Default features: 78 passed / 1 ignored.
- cw-private-dex (seams): 348 KiB optimized; SettleSwap (mock verify) ≈ 3.4k SDK gas VM + 14.4k KV writes; real proof_instance_verify adds ≈ 38k SDK gas (BENCHMARKS.md).

## [B] Unified e2e suite
- `e2e/SUITE.md`. Offline suite passes; live ict-rs test compiles (not run).
