Status: DEFERRED to v2 (user decision Oct 27 2026); v1 keeps vk_id 3 depth 17. Post-claim privacy via Penumbra.

# Shared registration tree: design analysis (read-only, 2026-10-27)

Branch `wip/headstash-membership-vk`. No code was changed. Paths are relative to the repo root.
C = `contracts/headstash/cw-headstash/src`, Z = `tools/headstash/circuits/halo2/src`.

## TL;DR

- **You cannot reuse the v1 circuit unchanged with a shared tree.** The leaf is
  `Poseidon(tag, key_com, amount)` and does **not** bind the campaign (Z/note_membership.rs:155).
  Only the nullifier binds ctx (Z/note_membership.rs:153). With several campaigns in one tree,
  a note registered under campaign X proves membership just as well against ctx_Y, gets a fresh
  nullifier `nf(secret, ctx_Y)`, and is paid by Y. Every registered note could claim from
  every campaign that has that denomination.
- Depth isn't the only parameter to fix before the gov store. **The leaf format is too.**
  Add a scope field: `leaf = Poseidon(TAG_v3, key_com, amount, scope)`. That is ConstantLength<4>,
  still 2 permutations, so it costs zero extra rows. The circuit constrains the leaf's scope
  to the public `ctx` limb. Public inputs stay the same 6 limbs.
- **Honest caveat:** if scope = per-campaign ctx (public at claim), sharing the tree gives
  **no extra anonymity**. The effective set stays {leaves with the same ctx and amount}, which
  is exactly the per-campaign set. A real Penumbra-style gain needs scope = a **shared pool**
  (for example per asset), with the funds pooled too. With a scope field, one circuit supports both
  setups, and the contract decides what `ctx` means.
- Recommendation: **ship the v3 leaf with scope, at depth 24 and K=12, before the gov store** (one vk).
  Keep the tree in cw-headstash for v1 (option B for the contract), or move it to a small
  registry (option C) if there is time. Migrating the contract later is cheap.
  Re-storing a circuit is a gov proposal.

## 1. What v1 actually proves

Circuit `NoteCircuitImpl::synthesize`, Z/note_membership.rs:136-173:

| value | definition | ref |
|---|---|---|
| key_com | Poseidon(TAG_KEYCOM, secret) | :151, Z/no_ecdsa.rs:239 |
| nullifier | Poseidon(TAG_NULLIFIER, secret, ctx) | :153, Z/no_ecdsa.rs:243 |
| leaf | Poseidon(TAG_NOTE_LEAF "terp-hs-note-leaf-v2", key_com, amount) | :155, :217 |
| node | Poseidon(CRH_TAG, layer, left, right), 17 layers | :157-165, Z/distro_poseidon.rs:232 |
| public | [ctx, root, nullifier, dest_lo, dest_hi, amount] | :167-170 |

Constants: NOTE_DEPTH 17 (:40), NOTE_K 11 (:43), 6 instances (:44). K=11 is asserted to be the minimum
(:562, test `note_k_is_minimal`).

- **Campaign binding.** The leaf does not bind it. The nullifier does, through ctx. The contract uses
  `ctx = sha256(tag‖chain_id‖contract)` with the top bits cleared (C/direct.rs:51-61). `bind_circuit` checks that ctx
  belongs to this contract (C/direct.rs:118-121). Today, binding comes only from the tree being private to one contract.
- **Denomination binding.** The leaf binds the amount, and the amount is also a public limb, so a note of one class can't claim as another.
  The bench confirms this: an amount swap returns false (tools/headstash/docs/onchain-verify-benchmark.md, Negatives).
  The contract checks `DENOMS_UTERP` and the campaign class cap (C/direct.rs:208-215).
- **Destination** is bound to `info.sender` (C/direct.rs:64-77, :240).
- **Nullifier storage** uses `nullifier_storage_key(campaign.root_id, nf)` (C/direct.rs:220). The nullifier
  is root-independent, so it allows one spend per (secret, ctx) across all anchors.
- **Registration** (C/notes.rs:293-320) checks the sha256 eligibility tree. It then appends
  `note_registration_leaf(key_com, amount)` (Z/distro_poseidon.rs:264) through `append_claim_leaf`
  (C/registration.rs:453-500). key_com must be unique (N6, :462).
- **Reuse with a shared tree?** Only if every leaf in the tree belongs to one scope. Multi-campaign
  sharing breaks campaign binding, as explained in the TL;DR. `key_com` uniqueness doesn't save you: one secret yields one key_com,
  but that key_com can still be spent once per ctx.

## 2. Options

Common fix for every shared variant: **leaf v3 = Poseidon(TAG_v3, key_com, amount, scope)**, with
the circuit wiring `scope` to the public ctx limb. The nullifier stays Poseidon(nf_tag, secret, scope).
The leaf scope must be set by the **registry**, never by the caller. For example, the registry computes
`scope = direct_ctx(chain_id, info.sender)`, so a campaign can only insert leaves into its own scope.

### (A) Shared registry contract, campaigns verify against registry anchors
- Trust model. The registry is a dumb append-only tree plus anchor history. Any contract may append, but only
  into its own scope. A malicious campaign can insert fake leaves only into its own scope, so it can't mint
  claims elsewhere. It **can** fill the tree or bloat anchors, which is a DoS. Mitigate with a per-leaf fee or deposit, or
  an allowlist of code_ids (for example, only the cw-headstash code hash).
- Binding. The leaf scope is the campaign ctx, and the claim contract checks `ctx == own ctx`, as at C/direct.rs:118.
  The nullifier stays per campaign.
- Amounts. These are unchanged. The leaf binds the denomination, and caps per class stay in the campaign.
- Anchors. Campaigns query the registry with `anchor_known` (a smart query, a few k gas). `min_anonymity_set` must then
  count **per-scope, per-denomination** leaves, not the global count (see §4). Today it counts the global
  leaf count (C/registration.rs:518-520).
- Anonymity gain: **none** while scope is per campaign. It is shared infrastructure only.

### (A+) Shared pool (the actual Penumbra-style variant)
- Scope = pool id (for example per asset or tokenfactory denom), shared by all campaigns of that asset. Each campaign
  **deposits amount × notes into the pool when it registers**, so every leaf is fully backed.
  The pool pays claims, and the nullifier is per pool.
- Gain: the set becomes {all leaves of that asset and denomination across campaigns}.
- Cost: campaign-level rules can't apply at claim time without revealing the campaign. That covers cutoff, sweep and burn
  of unclaimed notes (the other worker's change), per-campaign caps, and the public-path reservation (C/direct.rs:217).
  Unclaimed funds become pool-wide. This is a product decision. It can't be retrofitted onto the v1 campaign model.
- The same v3 circuit works. Only the meaning of ctx changes.

### (B) Per-campaign tree for v1, migrate later
- Trust and binding are the same as today. This is safe because the tree is private to the contract.
- If the v1 circuit keeps its leaf format (no scope), a later shared tree needs a **new circuit and a new gov store**,
  and v1 leaves can't be imported into a mixed-scope tree.
- If v1 adopts leaf v3 (scope = own ctx), the leaves are already scope-bound. A future registry can then import them,
  or new campaigns can just use the registry with the **same vk**. This is the cheap path.

### (C) Standalone registry from day one, Headstash its only user
- This is (A) with a single client. It costs one more contract, one more instantiate, and a cross-contract query per claim
  plus a cross-contract execute per registration (an extra ~30-60k gas, estimated and not measured). It also needs more e2e work.
- Benefit: no data migration later.
- Risk: a new contract on the critical path, in a branch already under review and edit.

## 3. Depth choice

Poseidon P128Pow5T3, width 3, rate 2. ConstantLength<3 or 4> = 2 permutations, and ConstantLength<2> = 1.
In-circuit permutations: key_com 1 + nf 2 + leaf 2 + 2 per layer = **5 + 2d**.
Each Pow5 permutation is about 37-40 rows (8 full rows + 56/2 partial rows + absorb).

| depth | perms | est. rows | K | leaves |
|---|---:|---:|---:|---:|
| 16 | 37 | ~1,450 | 11 (measured) | 65,536 |
| 17 | 39 | ~1,530 | 11 (measured, minimal) | 131,072 |
| 20 | 45 | ~1,760 | 11 (est., near the edge) | 1.05M |
| 24 | 53 | ~2,070 | **12** (just over 2,048, est.) | 16.8M |
| 28 | 61 | ~2,380 | 12 | 268M |
| 32 | 69 | ~2,700 | 12 | 4.3B, but `1u32 << depth` overflows (C/registration.rs:29), and index/position are u32 |

The row figures are estimates calibrated on depths 16 and 17, which fit K=11 but not K=10
(Z/note_membership.rs:562). **Measure before choosing.** Bump NOTE_DEPTH in a scratch copy and run
`minimal_note_k`. Depth 20-21 is the likely K=11 ceiling.

- **Registration gas per leaf.** The tree does d CRH hashes plus 1 leaf hash in wasm (C/registration.rs:710-730),
  plus one `NODES` write per completed subtree. Measured: depth-16 `RegisterHeadstash` was 398-404k gas
  (tools/headstash/docs/circuit/REGISTRATION-TREE.md:12, :405, :418). The Poseidon share isn't separated out. Assuming
  about 60% of that is hashing, each layer costs roughly 14k gas, so depth 24 ≈ +110k (~510k) and depth 28 ≈ +170k (~570k)
  per single registration. `RegisterNotes` batches amortize the signature cost but not the hashing.
  This is an estimate. Measure it with the e2e.
- **Browser prove.** Measured at K=11: 3.35 s in wasm including keygen (headstash-denominations-spec.md:191), and native prove 0.094 s
  (:82). At K=12, expect about 2× (~6-7 s in the browser including keygen, ~0.2 s native). The pk size (1.25 MB) roughly doubles too.
- **On-chain verify** is essentially flat. The host charges `1 + ceil(proof/1024) + ceil(inst/32)` units
  (onchain-verify-benchmark.md). An IPA proof grows by about 64 B per K, so 2,752 → ~2,816 B and still 10 units ≈ **34.9k gas**,
  ~192.6k for a full tx. But the **params grow to ~262 KB at K=12** (131,140 B at K=11), so the circuit store goes from
  10.24M gas to roughly 20M (est.). Check this against the mainnet max block gas and tx size for the gov proposal.
- Pick: **24**. It needs K=12 either way. Going above 24 costs about 14k registration gas per layer and some proving time,
  for capacity Headstash won't use. 32 is out without u64 indices.
  If keeping K=11 matters more than capacity, use depth 20 (1M leaves). That is fine for Headstash-only use but tight for "anyone's campaigns".

## 4. Anchor semantics: the real anonymity set

- Registration is **public and identity-linked**. The `RegisterNotes` tx reveals the identity (eligibility opening),
  the campaign, key_com, the class and the leaf index (C/notes.rs:269-320, register event `index`/`leaf` C/registration.rs:443-449).
  The two-step design (register, then claim from a fresh wallet) hides **which** leaf a claim spends.
  It doesn't hide that the identity registered.
- A claim reveals ctx (scope), amount, root (anchor) and the destination.
  Effective set = leaves with **the same scope AND the same amount AND an index below `anchor.leaf_count`** of the chosen anchor,
  minus leaves already linked by other means. Timing also matters: registering, then claiming soon after the next anchor, narrows the set.
- So, with scope = campaign ctx, a shared tree adds **0** to the set. Leaves of other campaigns or other denominations are public
  noise that the verifier already excludes. Only scope = shared pool (A+) enlarges the set.
- The first anchor: `accept_anchor` requires `count >= min_anonymity_set` (C/registration.rs:517-520). That count is the
  **total** leaf count, so the 12 denominations split it. A campaign with min 100 could have 2 leaves in some class.
  In a shared tree, the global count is even less meaningful. Enforce the minimum **per (scope, class)**, or at least refuse claims
  for classes whose per-scope count at that anchor is below the minimum.
- Limits: capacity is `1 << depth` (C/registration.rs:28-30, "tree full" :461). Anchors are rate-limited by
  `anchor_interval` blocks (:513-517, :536) and require a root change. **`ANCHORS` is an unbounded `Item<Vec<_>>` loaded
  on every claim** (C/registration.rs:52, :542-545). That's fine per campaign but a gas time-bomb in a shared tree.
  Use a `Map<root, AcceptedAnchor>` (with an optional expiry) in any registry.

## 5. Recommended minimal change set (before the gov circuit store)

Circuit (Z), about half a day plus measurement:
1. `TAG_NOTE_LEAF` → `"terp-hs-note-leaf-v3"`. At :155, hash `[leaf_tag, key_com, amount, ctx]` (ConstantLength<4>).
   Update `note_leaf` and `note_root` (:217-232) and `distro_poseidon::note_registration_leaf` (Z/distro_poseidon.rs:264) to take scope.
2. `NOTE_DEPTH = 24`, `NOTE_K = 12` (after `minimal_note_k` confirms it). Witness parsing expects 24 siblings (:413-420).
3. Regenerate the blob and vk hash (`NOTE_VK_HASH` :638), rebuild the browser prover (`scripts/build-browser-prover.sh`),
   and update tools/headstash/deploy/render.py:48 and the rendered proposal.

Contract (C), small, coordinated with the worker who owns cw-headstash:
4. Pass `campaign.ctx` into the leaf at C/notes.rs:319 and C/registration.rs:443 (`registration_leaf`).
5. `NOTE_DEPTH`/`MAX_DEPTH` 24 (C/registration.rs:19-21). The eligibility tree is separate. Its depth check at C/notes.rs:198
   is tied to NOTE_DEPTH, so decouple that constant (eligibility can stay 17).
6. Make `max_leaves` return u64 (C/registration.rs:28) for future safety.
7. Optional but advisable: per-class leaf counters, and a `min_anonymity_set` check per class at anchor or claim time.
   Also an `ANCHORS` map.

Not needed now: the registry contract itself. With v3 leaves, (A), (C) and even (A+) later use the **same vk**, and only the contracts change.

E2E impact: re-run the vk_id 3 suites with new fixtures. The path query and sibling arrays grow from 17 to 24, and query_path
is O(d²) reads (C/registration.rs:221), so about 2× the path query gas.
Re-measure registration gas, browser prove time and circuit-store gas, and update
onchain-verify-benchmark.md and headstash-denominations-spec.md §4/§10. The proof size, the 6-limb layout,
the direct.rs instance builder and the verify gas are unchanged.
Estimate: 1.5-2.5 days including e2e and docs. That is far cheaper than a second gov circuit proposal.

Doesn't break the current design: the two-step registration and claim, the fixed denominations, the sha256 eligibility,
direct.rs verification and the nullifier semantics all stay the same. The only behavioural change is that leaves are now
campaign-bound in the circuit rather than only by contract isolation.
