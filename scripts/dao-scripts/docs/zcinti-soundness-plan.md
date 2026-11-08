# zcinti validator — KMS + Zigner soundness test plan (PREPARED, NOT EXECUTED)

Status: plan only. Nothing here has been run. No service was started/stopped, no tx broadcast.
Gate: opencode2 `~/abstract/zigner/docs/security-posture.md` says **do not use a live validator key** until P0.1–P0.5 are fixed. Every phase below uses a throwaway consensus key on a disposable chain until Phase 5.

## Phase 0 — preconditions (code review, no keys)
- [ ] P0.1 KMS import pins the phone Noise static; unknown responder refused (unit + integration test).
- [ ] P0.2 two-sided fingerprint confirmation replaces TOFU pin.
- [ ] P0.3 high-water mark durable (fsync) and fail-closed; extension type/step validated.
- [ ] P0.4 live-key export blocked; legacy plaintext removed; user auth required.
- [ ] P0.5 KMS identity out of /tmp; console authenticated.
- [ ] Release build from verified manifest (P1).

## Phase 1 — disposable localnet (ict-rs `SpawnedTerp`, 1 validator = zcinti-test)
1. Generate a fresh test consensus key with `zcinti_keygen` into a temp dir (path only, never printed).
2. Run KMS + Zigner against the localnet; sign 1,000 blocks.
3. Double-sign probes (must all be refused): replay same H/R/S, lower height, same height different block id, KMS restart with HWM file deleted/corrupted (must refuse to sign, not reset).
4. Kill -9 KMS mid-sign, restart: no regression below HWM.
5. Wrong chain-id in sign request → refused (KMS + cw-orch-kms `ensure_chain_id`).
6. Unknown responder / re-pair with different Noise static → refused.

## Phase 2 — cw-orch-kms authz flow on localnet
- `cargo test -p cw-orch-kms --lib` (51 pass as of this change).
- Granter challenge v2: expired, wrong chain-id, reused nonce → refused.
- zafu approval with principal = terp address ≠ signer → refused.
- Session grant: MsgInstantiateContract2 + checksum-pinned stores only; attempt MsgSend/UpdateAdmin via MsgExec → chain rejects.
- Feegrant create/revoke on the isolated node; confirm revoke leaves no allowance.

## Phase 3 — bounty DAO on localnet
- `bounty_dao --dry-run` → instantiate on localnet; `app_setup --spawn --bounty-dao <dao>`; assert contract-info admin == owner == DAO for all three contracts (built-in assertion); execute `update_config` only through a DAO proposal.

## Phase 4 — testnet (120u-1) shadow
- zcinti as non-signing sentry first, then a testnet validator using the KMS (testnet key only), 72h, monitor missed blocks, HWM file, KMS logs.

## Phase 5 — morocco-1 (requires explicit owner approval)
- Only after Phases 0–4 pass and the owner signs off. Migration: stop old signer → copy HWM forward → start KMS; never two signers live.

## Pass criteria
Zero double-sign attempts accepted; all refusal tests refuse; no secret written outside the KMS store; no ephemeral key retains ownership or admin.

## TODO: Penumbra relaying custody (future work)

For now, Penumbra relaying keeps using rotko-hermes with its own view database and spend authority. That setup is out of scope for cw-orch-kms and is unchanged.
Future work to move Penumbra relaying onto external custody via Zigner:
- [ ] Zigner: an IbcRelay transaction display (recv/ack/timeout packet summary) so the phone can show what it approves.
- [ ] rotko-hermes: a remote custody backend (spend auth through a Zigner QR/socket round-trip in place of a local spend key).
- [ ] Dashboard: a Penumbra lane next to the cosmos chain profiles (osmosis/juno/atomone/terp).
