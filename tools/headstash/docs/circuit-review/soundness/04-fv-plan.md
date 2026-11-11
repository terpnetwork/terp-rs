# 04 — Batched FV plan

The goal is one Lean theorem, `NoteRelation`, for vk_id 3, plus a separate
theorem that `claim_note` feeds `proof_instance_verify` exactly the six
elements that theorem talks about. The Halo 2 verifier stays an assumption
unless batch 0 finds a real diff in `halo2_proofs`.

Rules for every batch, from the skill (`00-methodology.md`):

- Human statement first. The statement in `02-scoping.md` is the one. Do
  not edit it to match a partial proof.
- One lemma per `have`. `sorry` is allowed only inside a batch that is not
  claiming done, and `lake build --wfail` must be clean before the batch
  merges.
- `lake exe cache get` before `lake build`. Never compile Mathlib.
- Do the work off this host. This host does not run Lake or Cargo.
- Lean files live in a new project when implementation starts. They do not
  land in `tools/headstash/` as part of this scoping pass.

Estimates are person-weeks for one engineer who already reads Lean and
Halo 2. They assume Ironwood's Poseidon proofs are importable. The "if the
diff is dirty" column is the slip.

## Batch 0 — correspondence, before any import

Independent. Blocks every reuse.

Diff, do not re-prove:

| Artifact | Against |
| --- | --- |
| `zakura-common/crates/halo2_gadgets/src/poseidon/pow5.rs` | `halo2_gadgets-0.5.0` `poseidon/pow5.rs`, the revision named in Ironwood `CondSwap.lean` and the Poseidon module docs |
| `zakura-common/crates/halo2_gadgets/src/utilities/cond_swap.rs` and `utilities.rs` `bool_check` / `ternary` / `range_check` | same tag, `utilities/cond_swap.rs`, `utilities.rs` |
| `zakura-common/crates/halo2_poseidon/src/lib.rs` `ConstantLength` and `p128pow5t3.rs` | the poseidon crate those gadgets pin |
| `pasta_curves` Pallas prime | Ironwood `Zcash/Circuits/Specs/Pallas.lean` |
| zakura `halo2_proofs` IPA verifier entry (`verify_proof`, Blake2b, `Challenge255`) | the Halo 2 revision Ironwood's `Zcash/Snark/` models |

Known before the diff: the swap-gate polynomials and the `ConstantLength`
capacity formula match what Ironwood states. Line numbers in `utilities.rs`
do not match 0.5.0 (zakura's `bool_check` is at line 170, Ironwood's comment
says 133–143). A line drift is not a semantic drift. The diff still has to
be done; this pass did not run it.

Also in this batch, on a machine that is allowed to build: export the
vk_id 3 verifying key and the fixed-column assignments, and record
`NOTE_VK_HASH` (`note_membership.rs:638`) as the fixture pin. K=11 keygen
is the measurement the k19 lab timed at 0.03 s. It is still a Cargo build,
so it is not part of this scoping pass.

**Done when:** a short note says either "polynomials match, import
Ironwood" or lists the hunks that do not. Verifier diff result is a
yes/no on batch 7.

**Estimate:** 1 week. Dirty diff: 2–3 weeks to characterise, and batches
2 and 7 change size.

## Batch 1 — spec pins

Depends on batch 0.

State, as Lean definitions with no circuit:

- `Fp` equal to Ironwood's Pallas, or a proved ring-isomorphism if the
  developments must not share a type.
- `P128Pow5T3` rounds `(8, 56)`, S-box `x^5`, and the round-constant and
  MDS vectors. Prove the imported vectors equal the bytes in
  `halo2_poseidon`'s `fp::ROUND_CONSTANTS`, `MDS`, and `MDS_INV` by
  reflexivity (`decide` / `native_decide` on the finite table, not by
  rederiving Poseidon).
- The four tag field elements, computed from the ASCII strings at
  `note_membership.rs:46-48` and `distro_poseidon.rs:103`.
- `poseidonConstLen` for `L = 2, 3, 4` as Ironwood's `ConstantLength.value`.

**Done when:** `#check poseidonConstLen` works for those three lengths and
a known test vector from `note_leaf` (`note_membership.rs:217-220`) matches
`distro_poseidon::note_registration_leaf`
(`distro_poseidon.rs:262-269`). The Rust test
`contract_leaf_helper_matches_circuit_leaf` (`note_membership.rs:507-515`)
is the correspondence target.

**Estimate:** 1–2 weeks if constants match. 4–6 weeks if the MDS tables
differ and have to be re-ported.

## Batch 2 — permutation chip

Depends on batches 0 and 1. Parallel with batch 4.

Import or re-establish:

- full-round gate (`pow5.rs:92-112`) implements one spec round,
- partial-round gate (`pow5.rs:114-158`) implements two spec rounds, with
  the `MDS_INV` rewrite,
- pad-and-add (`pow5.rs:160-184`) implements `Sponge.AddInput.Spec`,
- initial capacity is the constant from batch 1, not advice.

Ironwood's `Pow5Theorems.lean` and `SpongeTheorems.lean` are the source.
The note circuit shares one `Pow5Config` across 39 permutations
(`note_membership.rs:125-131` and the call count in `02-scoping.md`). The
lemma has to be "every region that enables `s_full` / `s_partial` /
`s_pad_and_add` is one of these hashes", which is a configure-order fact
about `synthesize`, not a fact about the gate in isolation.

**Done when:** for an arbitrary satisfying assignment of the chip, the
squeezed cell equals `ConstantLength.value` on the absorbed message.
Zero `sorry`.

**Estimate:** 2–4 weeks if batch 0 is clean. 8–12 weeks if the partial-round
gate has to be proved again.

## Batch 3 — the four hashes

Depends on batch 2.

Four lemmas, each a direct instance of batch 2:

| Lemma | L | Message |
| --- | --- | --- |
| `keyCom_spec` | 2 | pinned `terp-hs-keycom-v1`, `secret` |
| `nullifier_spec` | 3 | pinned `terp-hs-nf-v1`, `secret`, `ctx` |
| `leaf_spec` | 3 | pinned `terp-hs-note-leaf-v2`, `key_com`, `amount` |
| `crh_spec` | 4 | pinned `terp-hs-distro-crh-v1`, pinned layer, left, right |

Each pinning subgoal is `constrain_constant`
(`note_membership.rs:186-203`), one `have` per call site. There are
`3 + 17*2 = 37` constant pins (three tags once, plus per layer the CRH
tag and the layer index). They are the same proof seventeen times for the
CRH tag. Write it as a loop lemma, not 17 copies.

**Estimate:** 2–3 weeks. Ironwood's `ConstantLength` is already generic in
`L`, so this batch is instantiation and the pin lemmas, not a new sponge.

## Batch 4 — swap

Depends on batch 0. Parallel with batches 1–3.

Instantiate `Zcash.Circuits.CondSwap.SwapSpec` on the columns
`advice[0..5]` chosen in `note_membership.rs:132`. Ironwood enables
equality only on column `a` inside the chip (`cond_swap.rs:241` in the
zakura file, and Ironwood's `configure`). The note circuit enables
equality on all six advice columns first (`note_membership.rs:119-121`).
That is a superset. The soundness proof still applies. The extra equalities
are what let Poseidon copy `a'` and `b'` into the next absorb. Those copies
are a lemma in this batch:

> the cells passed as `left` and `right` to `poseidon::<4>` are the
> `a_swapped` and `b_swapped` cells of that layer's swap.

**Estimate:** 1 week if `SwapSpec` imports. 3 weeks if the column
superset forces a small generalisation of Ironwood's `permutationColumns`
theorem.

## Batch 5 — `NoteRelation`

Depends on batches 3 and 4.

The composition in `03-components.md`, steps 1–5. One theorem:

```lean
theorem note_relation_of_satisfying_witness
    (asn : SatisfyingAssignment NoteCircuit) :
    NoteRelation (publicOf asn) := by
  -- have's, in the order synthesize runs, no sorry
```

`NoteRelation` is the statement in `02-scoping.md`. It does not mention
denominations, anchors, or senders.

Poseidon collision resistance appears here as a **hypothesis on the
theorem that says "the opened leaf was appended"**, not on
`note_relation_of_satisfying_witness`. The information-theoretic theorem
is "the prover knows a preimage". The computational theorem is "that
preimage is one of the leaves `append_claim_leaf` stored". Keep them
apart, in the Ironwood `SpecOrBreak` style. Do not prove collision
resistance.

**Estimate:** 4–6 weeks. Most of the time is the 17-layer induction and
the shared-cell bookkeeping for `secret`, `ctx`, and `amount`, not new
algebra.

## Batch 6 — contract boundary

Depends on batch 5 for the public-input layout. The model of `direct.rs`
can be written in parallel with batches 2–4; the glue waits for batch 5.

Model only these functions, as pure Lean on bytes:

- `direct_ctx` (`direct.rs:51-61`),
- `pack_destination` (`direct.rs:64-77`),
- `amount_limb` (`direct.rs:79-83`),
- `instances` (`direct.rs:85-95`),
- the predicate in `check_claim` (`direct.rs:185-226`): vk id, bound
  circuit, cutoff, `canonical32` on root and nullifier, `anchor_known`,
  denomination table, class cap, nullifier unset, then verify,
- `claim_note` pays `info.sender` and passes that string to `instances`
  (`direct.rs:240`).

Theorem shape:

```lean
theorem claim_note_verifies_note_relation
    (st : State) (msg : ClaimNoteMsg) (sender : Address)
    (h : checkClaimSucceeds st msg sender) :
    ∃ pub : NotePublic,
      instancesAre st msg sender pub
      ∧ -- the host verified a proof of NoteRelation pub
        True -- replaced by the batch-7 assumption, not by a sorry
```

Registration (`notes.rs` `register_notes`, `registration.rs`
`append_leaf_with`) is a second theorem in this batch: every appended
leaf equals `poseidonConstLen 3 ![tagLeaf, keyCom, denom amount]`, the
index is the leaf count at append time, and unoccupied siblings are zero.
That is the computational half of "opened leaf was appended", under the
collision hypothesis from batch 5.

Out of this batch: gas, fee grants, the registration hook, owner sweep,
the SHA-256 eligibility tree's internal hash (it is an assumption that
`register_notes` checks an opening; the opening function can be specified
without proving SHA-256).

**Estimate:** 2–3 weeks for `direct.rs`. 2 more weeks for the frontier
induction. No Poseidon work remains if batch 3 is done.

## Batch 7 — verifier assumption, not a proof

Depends on batch 0's yes/no.

If the IPA verifier matches what Ironwood models, the theorem is an
assumption record:

> If `verify_proof` accepts the pinned vk_id 3 key on this 192-byte
> instance, then the extracted assignment satisfies the gates batch 5
> talks about, or the extractor produced a discrete-log break.

Cite `Zcash/Snark/` and the book's own caveat that the circuit layer and
the SNARK layer are not yet composed. Composing them for this key is a
multi-month project that does not change claim integrity beyond what the
fixture pin already checks (the VK hash). Do it only if batch 0 says the
verifier diverged, or as a later explicit decision.

**Estimate if the diff is clean:** 2 days to write the assumption and the
fixture check. **If the diff is dirty:** not estimated here; it becomes
its own plan.

## Batch 8 — do not start for vk_id 3

The K=19 ECDSA circuit, the raw-key circuit, and the blind two-step
registration on `wip/k19-lab` need a foreign-field chip whose `add` / `sub`
actually constrain `a ± b`. The lab doc records that the old chip did not
(`wip/k19-lab:tools/headstash/docs/k19-lab-benchmark.md`, section 0) and
describes a later CRT replacement. This pass did not read that replacement
(it is not on this branch). No Lean batch is scheduled. Starting one now
would formalize a moving gadget and would not strengthen vk_id 3.

When that chip is frozen, the reusable blocks from this plan are batches
1–4 (the same Poseidon and the same swap). The secp scalar range checks
need Ironwood's `LookupRangeCheck` or a new CRT argument. Budget, as a
placeholder only: 3–6 months after the chip stops changing. Not on the
vk_id 3 critical path.

## Dependency graph

```text
B0 correspondence
├─ B1 spec pins ── B2 chip ── B3 four hashes ──┐
├─ B4 swap ────────────────────────────────────┼─ B5 NoteRelation ─ B6 contract
└─ B7 verifier assumption (record only) ───────┘
B8 secp        (independent, not scheduled)
```

B1 and B4 can run together after B0. B6's model can be drafted during B2
and glued after B5.

## Total

| Path | Calendar |
| --- | --- |
| vk_id 3 circuit theorem, clean diff (B0–B5) | about 12–18 person-weeks |
| plus contract boundary (B6) | about 4–5 person-weeks |
| verifier re-proof (B7 dirty) | not in this total |
| secp (B8) | separate, unscheduled |

A clean import of Ironwood is the difference between a quarter and a
half-year. Batch 0 is the decision.

## Proof sketch the first implementer should paste above the theorem

Stage 1 of the skill, frozen so the tactics do not drift the claim:

The satisfying assignment copies six advice cells onto the instance
column. The ctx, nullifier, root, and amount cells in that list are the
cells the hashes consumed, because `synthesize` puts those `AssignedCell`s
into the `public` array (`note_membership.rs:167-170`). Each swap gate
forces its flag into `{0,1}` and its outputs to be the ordered pair. Each
CRH region is a `ConstantLength<4>` sponge of the pinned tag, the pinned
layer, and that ordered pair. Induction on the layer index turns the
seventeen parents into `merkleFold`. The key-commitment and nullifier
regions are `ConstantLength<2>` and `ConstantLength<3>` on the same secret
cell. Nothing in this argument ranges a destination limb or a denomination.
Those predicates are applied to `publicOf` by the contract theorem, after
extraction.
