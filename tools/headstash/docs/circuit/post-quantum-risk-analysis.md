# Post-Quantum Risk Analysis: Headstash Commitment Tree & Inclusion Sets

> **Status:** Analysis & Recommendation — **public inclusion set decision accepted**
> **Date:** 2026-07-15 (decision 2026-07-20)
> **Scope:** `circuit/src/note/commitment.rs`, `circuit/src/tree.rs`, `circuit/src/note/nullifier.rs`,
> `circuit/src/circuit/headstash_merkle_tree.rs`, `circuit/src/circuit/note_commit.rs`,
> `circuit/src/distro_poseidon.rs`
> **Context:** The headstash circuit is on `zk-mvp` branch — the commitment scheme and Merkle tree
> construction are being designed **now**. This document captures why and how to avoid retroactive
> quantum vulnerability before the first genesis commitment is ever broadcast.
>
> **Product decision (2026-07-20):** Public Headstash **inclusion / distribution** trees use
> **Poseidon-v1** (`ADR-POSEIDON-DISTRO-TREE`, domain `poseidon-v1`). Sinsemilla remains only for
> legacy recovery of pre-Poseidon public trees and (for now) private Orchard-style note commit.
> See `docs/plans/spectrum/ADR-POSEIDON-DISTRO-TREE.md`.

---

## 1. The Question

The headstash circuit uses Sinsemilla commitments on the Pallas curve (ECDLP-based) for:

- **Note commitments** (`CommitDomain`): binds a note's contents (value, recipient, denomination,
  eligibility key, randomness) into a Pallas curve point, blinded by a trapdoor scalar.
- **Merkle tree hash** (`HashDomain`): compresses leaves and internal nodes into a 254-bit field
  element via Sinsemilla's lookup-optimized group hash.
- **Nullifier derivation**: uses the note commitment point directly in a scalar multiplication:
  `k * (PRF_nf(nk, rho) + psi) + cm.0` — the commitment point is added as a curve point.

All three derive collision resistance or computational binding from the **elliptic curve discrete
logarithm problem (ECDLP)** on the Pallas curve. Shor's algorithm solves ECDLP in polynomial time
on a sufficiently large fault-tolerant quantum computer.

The question is: **What is the actual risk to the commitment tree and inclusion sets? What should
we mitigate, when, and how?**

---

## 2. What Shor Actually Breaks in Headstash

It is important to be precise about what a quantum adversary can and cannot do, because the answer
determines where we spend mitigation effort.

### 2.1 Binding of Note Commitments — Broken

`NoteCommitment::derive()` calls
`sinsemilla::CommitDomain::commit(message, &rcm)` which produces a Pallas curve point. The binding
property says: given a commitment point `C`, it is computationally infeasible to find
`(m, r) != (m', r')` such that `Commit(m, r) = Commit(m', r')`. This holds under ECDLP.

A quantum adversary with Shor can extract the discrete log of `C` with respect to the Sinsemilla
generator bases. This means they can **open the commitment to any message they choose** by solving
for an appropriate trapdoor. Concretely, they can forge a note commitment that passes verification
as belonging to any (value, recipient, esk, rho, rcm) combination they desire.

**Impact on inclusion sets:** The leaf of the Merkle tree is `ExtractedNoteCommitment` — the
x-coordinate of the Pallas point. If the binding of the original commitment is broken, the leaf is
meaningless as a binding commitment. The adversary can claim the leaf represents a different note
than the one that was actually committed.

### 2.2 Collision Resistance of MerkleCRH — Broken

`MerkleHashOrchard::combine()` calls
`sinsemilla::HashDomain::hash(level_bits || left_bits || right_bits)` which also depends on ECDLP
(Sinsemilla's collision resistance reduces to the ECDLP in the random oracle model).

A quantum adversary can find collisions in this hash function. This means they can forge Merkle
inclusion paths — given a desired root, they can construct a path from a leaf of their choosing
that hashes to that root.

**Impact on inclusion sets:** Combined with broken commitment binding, an adversary can forge
a complete spend: pick arbitrary note contents, compute a commitment, find a Merkle path from that
commitment to any historical root, and submit a valid spend proof. The consensus layer accepts the
proof because it only checks (a) the nullifier is unused, (b) the Merkle path verifies against
a known root, (c) the ZK proof validates.

### 2.3 Nullifier Integrity — Broken by Association

`Nullifier::derive()` computes:
```
k * (PRF_nf(nk, rho) + psi) + cm.0
```
where `cm.0` is the Pallas point from `NoteCommitment`. If the commitment binding is broken and
the adversary can pick an arbitrary `cm.0`, they can derive a corresponding nullifier. Since
nullifiers are the double-spend prevention mechanism, this allows the adversary to spend the same
note twice (or spend notes that never existed).

### 2.4 Privacy of Note Contents — Not Directly Broken by Shor

The note contents (value, recipient, esk, etc.) are encrypted in `TransmittedNoteCiphertext` via
symmetric encryption using a key derived from ECDH (KA_Orchard). The on-chain inclusion tree
stores only the `ExtractedNoteCommitment` x-coordinate — not the note plaintext.

Shor breaks the ECDH key agreement, so if an adversary recorded ciphertexts on-chain and later
obtains one party's private key via Shor, they can decrypt. But this is a **harvest-now-decrypt-later**
(HNDL) privacy risk, not a consensus soundness risk. It is real but separate from the inclusion
set integrity question.

**The inclusion tree itself does not leak note contents.** It only leaks:
- That a note with this commitment exists
- That it is at position P in the tree
- The Merkle path structure (included in proofs)

### 2.5 What Is Already PQ-Safe in Headstash

Several components already use symmetric/hash-based primitives and are unaffected by Shor:

| Component | Primitive | Status |
|---|---|---|
| `prf_nf(nk, rho)` | Poseidon P128Pow5T3 | PQ-safe |
| `recp_to_fp(addr)` | Poseidon P128Pow5T3 | PQ-safe |
| `hdkf_pallas(esk, rho)` | Poseidon P128Pow5T3 | PQ-safe |
| `prf_pallas_m(fdi, esk, v, nd)` | Poseidon P128Pow5T3 | PQ-safe |
| `RandomSeed::psi()` / `RandomSeed::rcm()` | Blake2b-based PRF (PrfExpand) | Hash-based, PQ-safe |
| `EligibleSk` derivation (Poseidon) | Poseidon P128Pow5T3 | PQ-safe |

**The Poseidon infrastructure already exists and is wired in.** The `halo2_poseidon` crate is
patched into the workspace at `../zcash/halo2/halo2_poseidon`. The `test-press` crate has
`circuits/posiedon.rs` exercising Poseidon gates.

---

## 3. The Retroactive Attack Timeline

This is the critical concern that makes "ship now, fix later" dangerous:

1. **Today:** Genesis tree is built with Sinsemilla commitments. Notes are committed on-chain as
   `ExtractedNoteCommitment` values. Merkle roots are posted to the chain state.
2. **N years from now:** A fault-tolerant quantum computer with ~4000 logical qubits is available.
   Shor runs in hours on a single note commitment.
3. **Attack:** The adversary replays historical Merkle roots from the chain. For each root, they
   forge a spending proof: they pick arbitrary note contents, compute a Sinsemilla commitment,
   find a Merkle path from that commitment to the historical root, derive a nullifier, and submit
   a valid Halo2 proof that the chain's zk-wasmvm verifier accepts.
4. **Result:** The adversary drains funds from any note that was ever part of the inclusion set.
   They do not need to know who owned the note, what its value was, or any private key. They only
   need the public commitment and the Merkle root.

**The entire historical set of commitments becomes forgeable retroactively.** No amount of
post-quantum hardening of new notes protects old commitments that were already on-chain.

---

## 4. Options Analysis

### Option A: Nip at the Bud — Replace Sinsemilla with Poseidon2 Now

Replace the three Sinsemilla-dependent components with Poseidon2 (symmetric hash, no curve points)
**before** the first genesis tree is deployed or any note commitment is broadcast.

**What changes:**

```
NoteCommitment (current, ECDLP):
  CommitDomain::new(NOTE_COMMITMENT_PERSONALIZATION)
    .commit(bits(message), &rcm)
    → pallas::Point

NoteCommitment (proposed, PQ):
  Poseidon2::new(NOTE_COMMITMENT_PERSONALIZATION)
    .hash([nd, v, fdi, recp, esk_to_base, rho, psi, rcm])
    → pallas::Base  (a field element, not a point)
```

```
MerkleCRH (current, ECDLP):
  HashDomain::new(MERKLE_CRH_PERSONALIZATION)
    .hash(level || left || right)
    → pallas::Base

MerkleCRH (proposed, PQ):
  Poseidon2::new(MERKLE_CRH_PERSONALIZATION)
    .hash([level, left, right])
    → pallas::Base
```

```
Nullifier (current, ECDLP):
  k * (prf_nf(nk, rho) + psi) + cm.0    // cm.0 is a pallas::Point

Nullifier (proposed, PQ):
  Poseidon2::new(NULLIFIER_PERSONALIZATION)
    .hash([nk, rho, psi, cm])           // cm is now pallas::Base
```

**Effect on data types:**
- `NoteCommitment` changes from `pallas::Point` to `pallas::Base`
- `ExtractedNoteCommitment` becomes the identity function (it was already `pallas::Base` — the
  x-coordinate extraction is now unnecessary)
- `MerkleHashOrchard` stays `pallas::Base` — the internal representation doesn't change, only the
  hash function
- `Nullifier` stays `pallas::Base` — the derivation changes but the type doesn't

**Effect on proof size / circuit cost:**
- Poseidon2 uses ~320 multiplication gates per permutation with optimized custom gates
- Sinsemilla uses ~10 lookup bits per step, heavily optimized for the Pallas curves
- For a single note commitment + 32-level Merkle path, Poseidon2 adds roughly 20-30% more
  constraints compared to Sinsemilla's highly optimized lookup gates
- This is manageable within the `K=17` (131k row) circuit budget

**What stays the same:**
- The secp256k1 FFA chips for eligibility key verification
- The Poseidon-based `prf_nf`, `recp_to_fp`, `hdkf_pallas` (already PQ)
- The `EligibleSk` → `nk` derivation flow
- The overall circuit architecture, proving system (Halo2), and verification pipeline
- Smart contract interfaces (cw-headstash manifold) — the tree root type is still `pallas::Base`

**Dependencies:**
- `halo2_poseidon` is already patched at `../zcash/halo2/halo2_poseidon`
- Poseidon2 is a minor parameter change from Poseidon (increased rounds, tweaked MDS) and the
  crate likely exposes both
- The workspace already depends on `halo2_poseidon`

**Cost:** Estimated 1-2 weeks for circuit redesign, test vector regeneration, and integration
testing. All on the `zk-mvp` branch where changes are expected and uncontracted with external
stakeholders.

### Option B: Ship Sinsemilla, Add Quantum Recoverability Later

Keep the current Sinsemilla construction. Before the first genesis tree, modify the note format to
support eventual recovery (similar to Zcash ZIP 2005 / Ironwood). This means:

- Add a version byte to the note structure
- Derive `rcm` deterministically from note contents through a hash chain (so the commitment is
  already binding via hash preimage even though Sinsemilla adds ECDLP binding on top)
- If quantum computers arrive, deploy a recovery protocol: users open their commitments (revealing
  the preimages), and the protocol mints equivalent notes in a new PQ pool

**Problems with this approach:**
- The old Merkle tree roots remain on-chain and forgeable. The recovery protocol only helps if
  users actively migrate **before** the quantum attack. If they wait, the adversary forges spends
  faster than users can recover.
- Recovery requires a new ZK proof (opening the old commitment + proving to a new PQ pool).
  That's multiple months of additional circuit work, plus contract deployment, plus user
  coordination.
- The HNDL privacy risk remains for encrypted note data. Quantum recoverability for value (ZIP
  2005) does not address decryption of historical ciphertexts.
- This pattern makes sense for Zcash because they have millions of already-committed notes in
  production. It does not make sense for a new design that has committed zero notes.

### Option C: Hybrid — Poseidon2 for Merkle Tree, Keep Sinsemilla for Note Commitment

Replace the Merkle tree hash with Poseidon2 but keep the Sinsemilla `CommitDomain` for note
commitments. The Merkle tree would use field-element leaves from Poseidon2 hashes, while note
commitments remain curve-point-based.

**Problems with this approach:**
- The Merkle tree's collision resistance is restored, but the note commitment binding is still
  broken. An adversary can forge the contents of a commitment and then (because the tree is PQ)
  cannot forge an inclusion path. But they can still equivocate the commitment itself.
- The nullifier derivation still uses `cm.0` as a point, so nullifier integrity remains broken.
- This is a partial fix that addresses inclusion path forgery but not commitment equivocation.
  It reduces the attack surface but does not eliminate it.

**Not recommended.** The cost delta between fixing all three vs. fixing only the tree is small
(the main work is re-plumbing the circuit, not the hash function choice itself). A partial fix
that leaves the critical binding property broken is not worth the complexity.

---

## 5. Recommendation: Nip at the Bud (Option A)

Replace all three Sinsemilla-dependent components with Poseidon2 before the first genesis tree is
deployed. The rationale:

### 5.1 Zero Historical Exposure

If no Sinsemilla commitment is ever broadcast, there is nothing to retroactively break. The first
genesis tree root will use a hash-based commitment scheme that is quantum-safe. This is the only
way to guarantee "not retroactively breakable" — build the PQ scheme from block 0.

### 5.2 The Infrastructure Is Already Present

- `halo2_poseidon` is already in the workspace at `../zcash/halo2/halo2_poseidon`
- `poseidon::P128Pow5T3` with `ConstantLength` variants is already imported and used in `spec.rs`
- The `test-press` crate already has `circuits/posiedon.rs` with working Poseidon gates
- The same Poseidon parameters used for `prf_nf()` and `recp_to_fp()` can be reused or slightly
  adapted for the commitment and Merkle CRH roles

### 5.3 The Circuit Changes Are Localized and Well-Understood

The changes touch exactly three files:
- `note/commitment.rs` — replace `sinsemilla::CommitDomain::commit()` with `Poseidon2::hash()`
- `tree.rs` — replace `sinsemilla::HashDomain::hash()` with `Poseidon2::hash()` in `combine()`
- `nullifier.rs` — replace `k * (prf_nf + psi) + cm.0` with `Poseidon2::hash([nk, rho, psi, cm])`
- `circuit/headstash_merkle_tree.rs` and `circuit/note_commit.rs` — update the gadget calls to
  use Poseidon2 chip instead of Sinsemilla chip

The data types (`NoteCommitment`, `ExtractedNoteCommitment`, `MerkleHashOrchard`, `Nullifier`) all
store `pallas::Base` — the field element type does not change. Only the operation that produces
it changes.

### 5.4 The Cost of Changing Later Is Higher

| Scenario | Effort | Risk |
|---|---|---|
| Change now (Option A) | 1-2 weeks | Low — on `zk-mvp` branch, no prod commitments |
| Ship + recovery layer (Option B) | 3-4 months (recovery circuit + two trees + contract changes + migration UX) | Medium — relies on users migrating before quantum attack |
| Ship and accept (no change) | 0 | High — entire inclusion set forgeable once Shor-capable quantum exists |

### 5.5 The Proving Cost Tradeoff Is Acceptable

Sinsemilla's 10-bit lookup-based approach is very efficient for Halo2, but Poseidon2's arithmetic
gates are also efficient and well-studied. The difference is approximately:

- Sinsemilla leaf hash: ~2,800 constraints (via lookups + ECC point ops)
- Poseidon2 leaf hash: ~3,800 constraints (via arithmetic gates)
- Sinsemilla Merkle path (32 levels): ~89,600 constraints
- Poseidon2 Merkle path (32 levels): ~121,600 constraints

The total increase is manageable within a `K=17` circuit (131,072 rows). If proving time becomes
a bottleneck, the circuit size can be increased to `K=18` (262,144 rows) which is still compatible
with the on-chain verifier constraints.

---

## 6. Concrete Migration Plan

### Phase 1: Replace Note Commitment (commitment.rs)

**Current:**
```rust
let domain = sinsemilla::CommitDomain::new(NOTE_COMMITMENT_PERSONALIZATION);
domain.commit(
    iter::empty()
        .chain(nd.to_le_bits().iter().by_vals())
        .chain(v.to_le_bits().iter().by_vals())
        .chain(fdi.to_le_bits().iter().by_vals())
        .chain(recp.to_le_bits().iter().by_vals())
        .chain(esk.to_le_bits().iter().by_vals())
        .chain(rho.to_le_bits().iter().by_vals().take(L_ORCHARD_BASE))
        .chain(psi.to_le_bits().iter().by_vals().take(L_ORCHARD_BASE)),
    &rcm.0,
)
.map(NoteCommitment)
```

**Proposed:**
```rust
// Already have poseidon hash infrastructure in spec.rs
// Use Poseidon2 with domain tag for commitment separation
let cm = poseidon::Hash::<
    _, poseidon::P128Pow5T3, poseidon::ConstantLength<8>, 3, 2
>::init().hash([
    nd, v, fdi, recp, esk_pallas, rho, psi, rcm.0,
]);
NoteCommitment::from_field_element(cm)
```

**Changes:**
- `NoteCommitment` becomes `pallas::Base` instead of `pallas::Point`
- `ExtractedNoteCommitment` becomes a no-op identity wrapper (instead of `extract_p`)
- The commit domain constant is replaced with a Poseidon domain tag
- The trapdoor `rcm` is still used as a hash input (not a scalar multiplier) — blinding
  is achieved by including the random input in the hash, not by EC point blinding

### Phase 2: Replace Merkle CRH (tree.rs)

**Current:**
```rust
let domain = HashDomain::new(MERGE_CRH_PERSONALIZATION);
MerkleHashOrchard(domain.hash(
    iter::empty()
        .chain(i2lebsp_k(level.into()).iter().copied())
        .chain(left.0.to_le_bits().iter().by_vals().take(L_ORCHARD_MERKLE))
        .chain(right.0.to_le_bits().iter().by_vals().take(L_ORCHARD_MERKLE)),
).unwrap_or(pallas::Base::zero()))
```

**Proposed:**
```rust
let level_fe = pallas::Base::from(level.into());
let hash = poseidon::Hash::<
    _, poseidon::P128Pow5T3, poseidon::ConstantLength<3>, 3, 2
>::init().hash([level_fe, left.0, right.0]);
MerkleHashOrchard(hash)
```

**Changes:**
- The `MerkleHashOrchard` type and `EMPTY_ROOTS` precomputation change their hash function
- All test vectors (empty roots, Merkle paths, anchor computations) are regenerated
- `i2lebsp_k` and `L_ORCHARD_MERKLE` can be deprecated
- The `Hashable::combine()` signature doesn't change — only the body

### Phase 3: Replace Nullifier Derivation (nullifier.rs)

**Current:**
```rust
let k = pallas::Point::hash_to_curve("z.cash:Orchard")(b"K");
Nullifier(extract_p(&(k * mod_r_p(nk.prf_nf(rho) + psi) + cm.0)))
```

**Proposed:**
```rust
let nf = poseidon::Hash::<
    _, poseidon::P128Pow5T3, poseidon::ConstantLength<4>, 3, 2
>::init().hash([nk_derived, rho, psi, cm_field]);
Nullifier(nf)
```

**Changes:**
- The `k` constant point and `mod_r_p` bridge can be removed
- No more Pallas point scalar multiplication in nullifier derivation
- The nullifier is a pure hash output, collision resistant under symmetry assumptions

### Phase 4: Update Circuit Gadgets

The in-circuit versions in `circuit/headstash_merkle_tree.rs` and `circuit/note_commit.rs`:
- Replace `SinsemillaChip` usage with `PoseidonChip` where commitment/hash is computed
- The `LeafHashChip` can be simplified — canonicity checks for bit decompositions are replaced
  with field element range checks that Poseidon handles natively
- The `EccChip` parameter for `HashDomain` is no longer needed (Poseidon doesn't produce points)
- `sinsemilla::MessagePiece` / `Message` construction is replaced with direct field element
  assignment to Poseidon state

### Phase 5: Test Vector Regeneration

All test vectors in `test_vectors/`:
- `commitment_tree.rs` — empty roots, anchor values
- `merkle_path.rs` — authentication path hashes
- `note_encryption.rs` — commitment values (note: encryption format stays the same)
- `keys.rs` — only affected if related to commitment derivation

Generate new test vectors from a reference implementation and verify against the circuit.

---

## 7. What We Lose by Dropping Sinsemilla

It is worth naming the tradeoffs explicitly:

| Property | Sinsemilla | Poseidon2 |
|---|---|---|
| Lookup efficiency | 10-bit table lookups (highly optimized for Pallas) | Arithmetic gates (no lookups) |
| Constraint count per hash | ~70 constraints per 10 bits of message | ~320 constraints per permutation |
| Bit-decomposition flexibility | Any bit length via message pieces | Fixed field element width |
| Proof size | ~5KB (with Halo2) | ~5KB (same system, same curve) |
| Proving time | Baseline (highly optimized) | ~1.2-1.5x baseline (est.) |
| Quantum soundness | **Broken by Shor** | **Safe (symmetric assumptions)** |
| Maturity | Battle-tested in Zcash Orchard (2022+) | Battle-tested in numerous STARK-based systems |
| On-chain verifier impact | None (halo2_proofs) | None (same verified Halo2 verifier) |

The proving time increase is real but acceptable for the MVP. If performance becomes critical
later, Poseidon2 gates can be further optimized with custom lookup-based S-box implementations.

---

## 8. What We Should NOT Change (Yet)

- **Note encryption (KA_Orchard):** The ECDH-based key agreement protects privacy, not soundness.
  HNDL privacy is a real concern but is orthogonal to inclusion set integrity. It can be addressed
  separately with hybrid KEM or PQ KEM (ML-KEM) in a future upgrade. Changing it now would affect
  the `TransmittedNoteCiphertext` format, the note encryption/decryption code in `note_encryption.rs`,
  and the `pczt` transaction format — all of which are larger in scope.

- **Spend authorization (RedPallas):** The spend authorization signature (RedDSA/RedJubjub) is
  also ECDLP-based on the Pallas curve. This is a privacy/anonymity concern (adversary can forge
  authorizations), but it does not affect the inclusion set soundness directly. The consensus
  layer can be adapted to accept alternate signature schemes in a future upgrade.

- **Key agreement / DiversifyHash:** Curve-based key agreement is only relevant for privacy of in-
  flight notes. It does not affect the integrity of committed notes in the tree.

**The priority is: soundness of the inclusion set > privacy of note authors > HNDL encryption.**
Fix the first now, the second in the medium term, the third when ML-KEM integration for note
encryption is feasible.

---

## 9. Summary

| Question | Answer |
|---|---|
| Is the current Sinsemilla-based commitment tree retroactively breakable by quantum computers? | Yes |
| Does this affect consensus soundness (theft of funds)? | Yes — broken binding + collision resistance enables forged spends |
| Does the inclusion tree itself leak note contents? | No — only the commitment x-coordinate is stored |
| Are any current components already PQ-safe? | Yes — Poseidon-based prf_nf, recp_to_fp, hdkf_pallas |
| Should we ship with Sinsemilla and recover later? | **No** — recovery requires months of additional work, leaves a window of forgeable historical commitments, and relies on user migration before attack |
| Should we replace with Poseidon2 now? | **Yes** — the cost is 1-2 weeks on `zk-mvp`, the infrastructure already exists, and no genesis commitment has been broadcast |
| Is the proving cost acceptable? | Yes — ~1.2-1.5x constraint increase, fits in `K=17` or `K=18` circuit |
| What about note encryption and signatures? | Defer — they affect privacy, not inclusion set soundness |

**Bottom line:** Replace Sinsemilla with Poseidon2 for note commitments, Merkle CRH, and nullifier
derivation before the first genesis tree is deployed. This eliminates retroactive quantum risk at
the inclusion set level at a cost that is minimal compared to the alternative of cleanup after
deployment.