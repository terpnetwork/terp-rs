# 02 — Scoping sequence

Target: vk_id 3, `NoteMembershipCircuit`, Halo 2 on the Pallas base field,
proved with the IPA verifier on Vesta. K = 11
(`tools/headstash/circuits/halo2/src/note_membership.rs:43`). The floor
planner allocates `2^11 = 2048` rows. The k19 lab measured **1,598** used
rows, 6 advice and 6 fixed columns, 2,752-byte proofs, verifying-key blob
hash `34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398`
(`wip/k19-lab:tools/headstash/docs/k19-lab-benchmark.md`, section 2 table,
row "vk3 claim"; hash also at `note_membership.rs:638` and
`contracts/headstash/cw-headstash/src/notes.rs:22-26`). This pass did not
re-run keygen.

The circuit is not `circuit.rs` (the K=19 personal-sign circuit) and not
`no_ecdsa.rs` (vk_id 2, depth 16, unpinned tags). Those files matter only
as the bugs vk_id 3 was written to close.

## The relation, in the order `synthesize` builds it

`NoteCircuitImpl::<true>::synthesize`
(`note_membership.rs:136-172`) is a straight line. The scoping sequence
follows that line. Later batches should too: a lemma per step, composed
in this order, so a failed goal points at one call.

```text
secret, ctx, amount, dest_lo, dest_hi   ← advice, unconstrained except as below
tag_kc  = pin(personalization("terp-hs-keycom-v1"))
key_com = Poseidon_L2(tag_kc, secret)
tag_nf  = pin(personalization("terp-hs-nf-v1"))
nf      = Poseidon_L3(tag_nf, secret, ctx)
tag_lf  = pin(personalization("terp-hs-note-leaf-v2"))
leaf    = Poseidon_L3(tag_lf, key_com, amount)
node    = leaf
for layer in 0..17:
    bit   = boolean witness                         // (position >> layer) & 1
    sib   = fresh advice
    L, R  = swap(node, sib, bit)
    tag   = pin(personalization("terp-hs-distro-crh-v1"))
    lc    = pin(Fp::from(layer))
    node  = Poseidon_L4(tag, lc, L, R)
constrain_instance [ctx, node, nf, dest_lo, dest_hi, amount] at rows 0..5
```

`pin` is `fixed::<true>`, which assigns advice and then
`region.constrain_constant(cell, value)` (`note_membership.rs:186-203`).
The constant column is `rc_b[0]`, enabled at `note_membership.rs:124`.

## Component order

The brief's six components, with the step above that discharges each one.
Per-component soundness, risks, and Lean gaps are in `03-components.md`.

| Order | Component | Where it is discharged | Public? |
| --- | --- | --- | --- |
| 1 | Poseidon permutation and sponge | Every `poseidon(...)` call. One chip config, four input lengths. | No. Outputs are advice, copied onward. |
| 2 | Merkle path selector / swap | The loop, 17 times. `CondSwapChip` on `advice[0..5]`. | No. The root advice is public; the bits and siblings are not. |
| 3 | Nullifier derivation | The single `L=3` hash of `(tag_nf, secret, ctx)`. | The output cell is public row 2. |
| 4 | Range and boolean checks | Only the swap flag. No range check on secret, amount, siblings, or destination. | The flag is not public. |
| 5 | Public-input binding | Six `constrain_instance` calls, rows 0..5. | Yes. This is the whole verifier interface. |
| 6 | Destination binding | Rows 3 and 4 are the raw destination limbs. They enter no hash. | Yes. Binding to the payer is the contract, not a gate. |

Registration (ECDSA on EVM, ed25519 on Solana) is outside this sequence.
`notes.rs:427-450` checks the signature over
`headstash-register-notes-v3:{ctx}:{list_hash}:{n}:{reward_addr}` before
the first chunk appends leaves. The circuit never sees the signature,
the eligibility path, or `reward_addr`.

## Domain separation inside the hashes

All four hashes are `P128Pow5T3`: width 3, rate 2, `R_F = 8`, `R_P = 56`,
S-box `x^5` (`zakura-common/crates/halo2_poseidon/src/p128pow5t3.rs:8-24`).
`ConstantLength<L>` puts `L * 2^64` in the capacity element and pads the
rate with zeroes (`halo2_poseidon/src/lib.rs:398-411`). Different `L` do
not share a permutation. The ASCII tag is a second, explicit domain
separator inside the message.

| Hash | L | Blocks `(L+1)/2` | Capacity | Tag string | Call site |
| --- | --- | --- | --- | --- | --- |
| key commitment | 2 | 1 | `2 * 2^64` | `terp-hs-keycom-v1` | `note_membership.rs:150-151` |
| nullifier | 3 | 2 (one zero pad) | `3 * 2^64` | `terp-hs-nf-v1` | `note_membership.rs:152-153` |
| note leaf | 3 | 2 | `3 * 2^64` | `terp-hs-note-leaf-v2` | `note_membership.rs:154-155` |
| Merkle CRH, ×17 | 4 | 2 (no pad) | `4 * 2^64` | `terp-hs-distro-crh-v1` | `note_membership.rs:158-164` |

Permutation count: `1 + 2 + 2 + 17*2 = 39`. That is a structural count of
`Pow5Chip::permute` calls, not a measured row count. Each permutation is
8 full-round rows plus 28 partial-round rows (56 partial rounds, two per
row) inside one region (`pow5.rs:238-258`).

The leaf tag changed from vk_id 2's `terp-hs-claim-leaf-v1`
(`distro_poseidon.rs:254-259`) to `terp-hs-note-leaf-v2`
(`distro_poseidon.rs:262-269`). A depth-16 vk_id 2 leaf does not open in
this tree even before the depth and the pinning differ. The nullifier tag
did not change: both circuits use `terp-hs-nf-v1`
(`no_ecdsa.rs:34`, `note_membership.rs:47`). `ctx` is what keeps a vk_id 2
nullifier from equaling a vk_id 3 nullifier for the same secret. See
finding F6.

## What the contract adds around the relation

`claim_note` (`direct.rs:229-249`) never accepts a public-input buffer from
the message. `check_claim` rebuilds 192 bytes and passes those to
`proof_instance_verify`:

| Limb | Bytes | Circuit cell | Contract source |
| --- | --- | --- | --- |
| ctx | 0..32 | row 0, the same cell hashed into the nullifier | `campaign.ctx`, required equal to `direct_ctx(chain_id, contract)` at bind (`direct.rs:49-61`, `direct.rs:118-120`) |
| root | 32..64 | row 1, the node after 17 CRHs | must be `anchor_known` (`direct.rs:205-207`, `registration.rs:668-673`) |
| nullifier | 64..96 | row 2 | `canonical32`, then `verify_nullifier_stateful` (`direct.rs:203-204`, `direct.rs:242`, `headstash.rs:289-299`) |
| dest_lo, dest_hi | 96..160 | rows 3 and 4 | `pack_destination(info.sender)` (`direct.rs:64-77`, `direct.rs:85-86`) |
| amount | 160..192 | row 5, the same cell hashed into the leaf | `u64` little-endian, and `DENOMS_UTERP.contains` (`direct.rs:79-83`, `direct.rs:208-210`) |

The footer public-input count is 6 (`note_membership.rs:44`, written by
`note_circuit_blob` at `note_membership.rs:361-372`). The on-chain blob is
pinned twice: `validate_note_campaign` requires `campaign.vk_hash` to equal
the compiled `NOTE_VK_HASH` (`notes.rs:141-149`), and `bind_circuit`
requires `sha256(stored blob)` to equal that same hash (`direct.rs:125-128`).

## Tree the root is a root of

The contract's claim tree is append-only, depth 17 for vk_id 3
(`registration.rs:18-19`, `registration.rs:39-45`). `append_leaf_with`
walks all `frontier.len()` levels, which is the stored depth, and hashes a
missing right child as `pallas::Base::ZERO`
(`registration.rs:905-917`). The circuit always hashes 17 levels and uses
whatever sibling the witness supplies; the honest path query returns 32
zero bytes when `node_at` answers `None` (`registration.rs:316-318`,
`registration.rs:365-368`).

So the anchored root is the depth-17 root under the same CRH the circuit
computes, with implicit empty siblings equal to zero. An empty slot is not
a leaf the prover can open unless they know a Poseidon preimage of that
zero-sibling subtree. That is a collision assumption, not a missing boolean
constraint. It is lemma M4 in `03-components.md`.

Leaves enter only through `append_claim_leaf` (`registration.rs:574`). The
two callers are legacy `insert_registration` (`registration.rs:564`), which
`legacy_only` rejects when `vk_id == 3` (`registration.rs:488-493`), and
`register_notes` (`notes.rs:390-395`), which checks the eligibility opening
and the signature first.

## Secondary circuit, not in the sequence

`wip/k19-lab` compares four designs. vk_id 3 is the only one in this
sequence. The other three (in-circuit ECDSA, raw secp256k1 key, blind
two-step registration) sit on a foreign-field chip the lab itself called
unsound before a later rewrite (benchmark doc, section 0 and the
"Sound foreign-field chip" addendum). Formalizing those is batch 8, and it
does not gate batches 0–6.

## Illustrative top-level statement

Not a proof. The shape a later Lean file should refuse to weaken:

```lean
-- Public input layout of NoteMembershipCircuit, rows 0..5.
structure NotePublic where
  ctx : Fp
  root : Fp
  nullifier : Fp
  destLo : Fp
  destHi : Fp
  amount : Fp

/-- Knowledge extracted from a satisfying vk_id 3 witness.
    `destLo`, `destHi` are carried so the statement stays in bijection
    with the six instance cells. They are not hashed. -/
def NoteRelation (pub : NotePublic) : Prop :=
  ∃ secret : Fp,
    ∃ bits : Fin 17 → Bool,
      ∃ siblings : Fin 17 → Fp,
        let keyCom := poseidonConstLen 2
          ![tagKeycom, secret]
        let leaf := poseidonConstLen 3
          ![tagLeaf, keyCom, pub.amount]
        pub.nullifier = poseidonConstLen 3
          ![tagNf, secret, pub.ctx]
        ∧ pub.root = merkleFold leaf bits siblings

-- Intentionally absent: pub.amount ∈ denoms, pack(sender) = (destLo, destHi),
-- pub.ctx = directCtx chain contract, anchorKnown pub.root.
-- Those are ContractObligations, composed outside NoteRelation.
```
