# Headstash registration surface: security review (2026-10-26)

Scope: cw-headstash registration + epoch path (`src/registration.rs`, `src/epoch.rs`,
`src/msg.rs`, `src/lib.rs`, `src/wavs.rs`), the registration relay
(`tools/hash-market/src/bin/headstash-register-relay.rs`), the eligibility build
(`tools/headstash/target/eligibility/`), and the mainnet modules it depends on
(hashmerchant, wasm circuit params). Read-only mainnet queries via
`https://terp.api.m.anode.team` / `https://terp.rpc.m.anode.team` (morocco-1, app v6.4.0).

Severity: Critical / High / Medium / Low / Info. "Fixed" means a commit in this branch with a test.

## Summary

| # | Sev | Title | Status |
|---|-----|-------|--------|
| H1 | High | No ownership transfer: owner is the instantiating key forever | **Fixed** (UpdateOwnership, two-step, renounce disabled; test) |
| H2 | High | `RegistrationPath` smart query is O(N) storage + ~2N Poseidon | Open: needs contract change or off-chain path builder |
| H3 | High (privacy) | Unique payout amounts deanonymize ~8.8% of identities | Open: design decision |
| H4 | High (trust) | Owner can stage arbitrary rows, re-set campaign, sweep early | Open: governance/operational control |
| H5 | High (deploy blocker) | Mainnet has no `headstash-claim` chain registered; mainnet binary predates VE custody fix | Open: gov message drafted; upgrade decision |
| M1 | Medium | `ctx` is owner-chosen, not derived on-chain | Mitigated by runbook derivation (`deploy/render.py`) |
| M2 | Medium | Relay batch failure can fail innocent items after front-runs | Open |
| M3 | Medium | Sweep allowed after cutoff while staged batches are unpaid | Open (runbook ordering) |
| M4 | Medium | Browser/witness prover source only exists in `/tmp/zk-standalone`, not committed | Open (provenance) |
| L1 | Low | key_com griefing by an eligible attacker | Accept |
| L2 | Low | Relay `hits`/`status` maps unbounded | Open |
| L3 | Low | Relay trusts `X-Real-IP` from loopback | Runbook: nginx must set it |
| L4 | Low | Amount-class cap is per batch, not cumulative | Accept (nullifier dedupe covers double pay) |
| L5 | Low | WAVS proof-of-possession not bound to contract address | Accept (standard PoP) |
| I1-I6 | Info | see below | - |
| N1 | Nit | wavs.rs got/want swapped; stale "0 disables threshold" comment | **Fixed** |

No Critical issues were found.

## High

### H1. Owner cannot be changed (fixed)
`instantiate` calls `cw_ownable::initialize_owner(info.sender)` but `ExecuteMsg` exposed no
`UpdateOwnership`. The owner signs `set_epoch_campaign`, `stage_epoch_batch` and
`sweep_unclaimed`. A lost or compromised deployer key was unrecoverable except by a gov
migrate, and the deployer could never hand the campaign to the DAO.

Fix: `ExecuteMsg::UpdateOwnership(cw_ownable::Action)` (two-step transfer/accept).
`RenounceOwnership` is rejected so staged rows can always be paid and swept.
Test `update_ownership_two_step_and_no_renounce`: non-owner cannot propose, non-pending
cannot accept, new owner takes over, old owner loses `assert_owner`, renounce errors.

### H2. RegistrationPath query cost grows with registrations
`registration.rs` stores leaves (`reg_leaf`) and a frontier only. The path query loads every
leaf and recomputes subtrees (`subtree()`), O(N) reads and about 2N Poseidon hashes. At the
17.8k-identity scale this will very likely exceed the node's smart-query gas limit, so the
claim page cannot get its Merkle path from the contract.
Recommendation (pick one before opening claims):
1. Contract: persist interior nodes per level on insert (`Map<(u8,u32),Binary>`) so the path is
   16 reads. Needs a new code id and a fresh e2e run.
2. Off-chain: build paths from `wasm` register events (they carry `index` and `leaf`) in the
   relay or a static indexer, and verify the result against the accepted anchor before proving.
Not fixed here: it changes storage layout and the e2e chain must not be touched by this job.

### H3. Unique amounts link payouts to eligible addresses
Each payout row carries the registration amount. In the current eligibility build 1,831
distinct amounts cover 17,826 identities and **1,568 identities have an amount nobody else
has**. The eligibility list is public, so a payout of that amount names the source wallet
regardless of the ZK membership proof. `min_anonymity_set` bounds the tree size, not the
amount class.
Recommendation: split allocations into a small fixed denomination set (e.g. powers of ten in
uterp, several notes per identity) or round to bucket values, or document that large/unique
holders get unlinkability only for the registration step. Needs a user decision.

### H4. Owner trust
The owner can: stage any rows (destinations and amounts) that pay once the oracle attests
the matching accepted root; call `set_epoch_campaign` again before staging (ctx, cutoff,
trusted set, allocation root, sweep recipient; the registration config freezes only once
leaves exist); sweep after cutoff. The wasm admin can migrate.
Recommendation: owner = TerpNET DAO or an ops multisig with published membership (H1 fix
enables the handoff); wasm admin = gov module; publish every staged batch's digests so anyone
can check rows against submitted proofs.

### H5. Mainnet prerequisites missing
- `terpd q hashmerchant chains` on mainnet returns `[]`. `MsgRegisterChain` is gov-only, so the
  `headstash-claim` source must be registered by proposal (included in the funding proposal).
- Mainnet app is v6.4.0. The e2e used `b2339a9c` (v6.4.0 + 8 commits, incl. "Return an empty
  vote extension when sidecar custody checks fail"). Decide whether that needs a chain upgrade
  before payouts.
- Payout sudo requires vote-extension quorum (`quorum_fraction` 0.667): two-thirds of voting
  power must run the hashmerchant sidecar reachable to the lane.

## Medium
- **M1 ctx.** Not derived on-chain, so signatures replay across deployments with the same ctx.
  `deploy/render.py` derives `ctx = sha256("terp-hs-ctx-v1"||chain_id||0||contract||0||label)`
  with the top two bits cleared (valid Pallas element).
- **M2 relay batch atomicity.** A front-run or bad message fails the whole DeliverTx; the relay
  re-simulates and drops, but after tries >= 2 items are marked failed "relay". Users must be
  able to resubmit (they can: dedupe is by identity and status is per request).
- **M3 sweep ordering.** Sweep only after the last staged batch is paid (runbook step 9).
- **M4 prover provenance.** `browser_prove.rs`, `prove_witness_json`, `verify_witness_proof` live
  in `/tmp/zk-standalone/halo2` and are not in this worktree's `circuits/halo2`. Commit them
  before mainnet so the vk hash `94d43261...` is reproducible from git.

## Low
- **L1** An eligible attacker can register a victim's key_com under the attacker's own
  signature (burning the attacker's allocation); the victim picks a new secret.
- **L2** Relay `hits` and `status` maps have no eviction. Restart policy or TTL needed.
- **L3** Relay trusts `X-Real-IP` from 127.0.0.1; nginx must `proxy_set_header X-Real-IP $remote_addr`.
- **L4** Amount cap applies per staged batch only.
- **L5** WAVS PoP hashes the pubkey only.

## Info
- I1 Relay k256 recover likely rejects high-s; contract host recover normalizes. Dedupe is by identity.
- I2 EVM `v` accepted in {0,1,27,28}. Solana uses `verify_strict`. Prefixes
  `headstash-register-v1` / `headstash-register-sol-v1` are domain separated.
- I3 Relay check order: body cap, parse, ctx, IP rate, Sol identity rate, verify, EVM identity
  rate, registered query (30 s cache), dedupe, queue. Defaults: flush 6 s, batch 25, queue
  2000, 5/identity/10 min, 30/IP/min.
- I4 Relay default bind 127.0.0.1:8791 collides with kms on groot-wan; production uses another port.
- I5 Nullifiers keyed `{root_id}:{nf}`.
- I6 The current eligibility tree contains `0x000...000` and `0x...dead`; the in-progress rebuild
  excludes burn, zero and contract addresses.

## Parity and timing
See `tools/headstash/docs/mainnet-deploy-runbook.md` "Verification evidence".

## Addendum: fixed-denomination notes (spec rev 5, 2026-10-26)

Scope: `circuits/halo2/src/note_membership.rs` (vk_id 3), `epoch/src/denominations.rs`,
`docs/headstash-denominations-spec.md`, and the vk_id 2 circuit it replaces.

### N1 (critical, vk_id 2): Poseidon tags and Merkle layer constants are free advice
In `no_ecdsa.rs` the key-commitment, leaf and nullifier tags and the per-layer Merkle
constants are loaded with `assign_advice` and never constrained to constants. The prover
chooses them. With a free nullifier tag one secret yields unlimited distinct nullifiers for
the same leaf, i.e. unlimited claims of one allocation. vk_id 2 must not be deployed for a
campaign holding funds. Fix (vk_id 3): every tag and layer cell goes through
`region.constrain_constant`. Test `pinned_tags_reject_a_forged_nullifier`: the same circuit
with pinning off accepts a forged-tag nullifier, with pinning on (production) it fails.

### N2 (by design): registration links an identity to all its notes
The eligibility tree is public and the contract needs each note class to build the claim
leaf, so registration reveals identity, note count and class vector. `key_com` values are
Poseidon commitments to independent 255-bit secrets and reveal nothing; leaf positions are
public but the proof hides which leaf is opened. Privacy rests entirely on claim-side
unlinkability. Mitigation: spec section 6 (fresh destinations, random order and delay,
relayer fees, Penumbra diversified addresses). Padding the registration with decoy notes
does not help because the eligibility tree already publishes the note list.

### N3 (medium): note-count and class-vector fingerprints
Max holder has 177 notes; 15 identities have a note count nobody else has, 61 a count shared
by fewer than 10, 895 a unique class vector. If an observer groups claims (same block,
same relayer session, same IP, linked destinations) these identities are re-identified. A
5000 TERP holder with 177 notes owns 2.3% of class 11 (7,676 notes, 2,859 holders).
Mitigation: per-note random delays, never more than one note per identity per block, and
the claim page should warn large holders. Open: cap notes per identity by adding a 10k or
20k class (reduces max count but creates a small class; not recommended at this size).

### N4 (low): front-running
Registration: the message is signed over `H(list)`; a relayer can only submit it as is.
Claims: destination is a public input bound by the proof, a copied proof pays the same
destination, and the nullifier makes a second copy fail. No change needed.

### N5 (low): griefing
A third party cannot register for an identity without its signature. A sidecar can delay
or drop chunks; since every chunk carries the full signed list any sidecar can resubmit.
A wrong first chunk cannot pin a different list because the contract stores the list hash
from the signature, not from the sender.

### N6 (medium if missed): cross-note replay and duplicate secrets
Distinct secrets give distinct nullifiers. If a buggy client reused a secret for two
notes, both leaves would share one nullifier and one note would be unclaimable. Contract
must reject duplicate `key_com` within a list and globally. The nullifier tag stays v1; the
`ctx` must include the campaign/contract so a vk_id 2 nullifier cannot collide with or be
replayed in the vk_id 3 campaign.

### N7 (medium if missed): class tampering
Eligibility leaf commits the class, the claim leaf commits `DENOMS[class]`, the circuit
binds `amount` into the leaf and exposes it. The contract must take the amount from its own
table (never from the message) at registration and check `amount` in `DENOMS` at claim.
`prove_note` refuses non-class amounts; `wrong_amount_or_root_fails` covers mismatch.

### Proposed mitigations / phase-2 checks
1. Do not deploy vk_id 2 for new funds; register vk_id 3 (hash 34d11437...5398).
2. Contract: amount from table, `DENOMS` membership at claim, duplicate key_com rejection,
   ctx bound to contract address and campaign.
3. Claim page: per-note delays and fresh destinations by default; large-holder warning.
