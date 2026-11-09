# 01 — Prior art

Catalog of Lean developments that a vk_id 3 proof could actually import.
Versions were read from the default branch on 2026-10-31. None of these
trees were built on this host (`lake exe cache get` / `lake build` are
forbidden here; see `00-methodology.md`).

## Ironwood (reuse this first)

- Repo: https://github.com/zcash/ironwood
- Book: https://zcash.github.io/ironwood/formal-verification/guide.html
- Scope page: https://zcash.github.io/ironwood/formal-verification.html
- Audit notes (ZKSecurity, 2026-10-05): https://blog.zksecurity.xyz/posts/auditing-formal-verifications/

Ironwood is a Lean 4 + Mathlib development of the Orchard / Ironwood Action
circuit on the Clean Halo 2 embedding, plus a Halo 2 verifier
knowledge-soundness argument in the algebraic group model. The circuit port
is from `orchard@0.14.0` and `halo2_gadgets-0.5.0`, and the module docs cite
Rust line numbers. The build they treat as the proof is `lake build --wfail`,
after `lake exe cache get`. That matches the skill.

The verifier and the circuit are not yet one theorem. The book says the
Action circuit, the SNARK, and the ledger games "don't quite" fit together.
Fixture dumps (`Zcash/Circuits/Fixtures/*.json`) pin the layout; some of
those dumps have no regenerate-and-diff pipeline. A vk_id 3 effort should
copy the trust-boundary habit (`Zcash.TrustBoundary`, `#`-style census of
assumptions) and should not claim "the proof verifies" until a zakura
verifying key is re-derived inside Lean and compared to
`note_membership::NOTE_VK_HASH`.

### Primitives that match this circuit

| Need | Ironwood module | What it already states |
| --- | --- | --- |
| Pallas field | `Zcash/Circuits/Specs/Pallas.lean` (imported by CondSwap) | The Pasta base field the circuit assigns into. |
| Poseidon P128Pow5T3 permutation | `Zcash/Circuits/Poseidon/Permute.lean`, `Pow5.lean`, `Pow5Theorems.lean`, `Rounds.lean`, `Constants.lean` | Width-3, x^5, full and partial rounds, round constants. |
| Sponge / `ConstantLength<L>` | `Zcash/Circuits/Poseidon/Hash.lean`, `HashTheorems.lean`, `SpongeTheorems.lean` | Capacity `L * 2^64`, block count `(L+1)/2`, pad with zeroes, absorb-permute-squeeze. `ConstantLength.value` is generic in `L`. |
| Conditional swap + boolean | `Zcash/Circuits/Utilities/CondSwap.lean` | `SwapSpec`: the flag is boolean and the outputs are the swapped pair. Soundness and completeness are filled (the file's `soundness` / `completeness` proofs, not `sorry`). Ported from `halo2_gadgets-0.5.0` `cond_swap.rs`. |
| Boolean / range helpers | `Zcash/Circuits/Utilities/LookupRangeCheck.lean`, `DecomposeRunningSum.lean` | Lookup range checks. The note circuit does not use a lookup table; only the swap bit is boolean. |
| Halo 2 custom-gate embedding | `Verified-zkEVM/clean` via `import Clean.Halo2` | Gates, selectors, copy constraints, configure-order. |
| Verifier knowledge soundness | `Zcash/Snark/` (see the book) | AGM extractor: accepting transcript yields a witness or a discrete-log break. This is the assumption batch 7 records, not a proof we re-do. |

`SpongeTheorems.lean` states the pad-and-add effect as field addition on the
rate and an unchanged capacity element. That is the same equation as
`Pow5Chip`'s `"pad-and-add"` gate in zakura
(`zakura-common/crates/halo2_gadgets/src/poseidon/pow5.rs:160-184`).

`Hash.lean` `ConstantLength.capacity` is `(L * 2^64 : Nat)` inside `Fp`,
matching `halo2_poseidon`:

```398:402:crates/zakura-common/crates/halo2_poseidon/src/lib.rs
    fn initial_capacity_element() -> F {
        // Capacity value is $length \cdot 2^64 + (o-1)$ where o is the output length.
        // We hard-code an output length of 1.
        F::from_u128((L as u128) << 64)
    }
```

(The path above is under `/Users/returniflost/abstract/terp-core/`. The
worktree does not contain `zakura-common`.)

### What Ironwood does not give us

- A note-membership circuit. Its top-level circuit is the Orchard Action
  (`Zcash/Circuits/Action/`), whose hashes are Sinsemilla plus a Poseidon
  nullifier PRF, not `terp-hs-note-leaf-v2`.
- secp256k1. `Zcash/Circuits/Ecc/` is Pallas / Pasta incomplete addition.
- The contract boundary (CosmWasm `proof_instance_verify`, anchors, denomination table).
- A proof that zakura's vendored gadgets equal `halo2_gadgets-0.5.0`. The
  gate text we read matches; the line numbers do not. See questions.md.

## dregg (methodology, not gadgets)

- Repo: https://github.com/emberian/dregg
- Assurance contract: https://github.com/emberian/dregg/blob/main/docs/ASSURANCE.md
- Kernel: https://github.com/emberian/dregg/blob/main/docs/KERNEL.md
- Apex cited by the kernel doc: `metatheory/Dregg2/Circuit/CircuitSoundness.lean`
  (`lightclient_unfoolable`), and `metatheory/Dregg2/AssuranceCase.lean`.

dregg's Lean tree proves an authenticated cell-program kernel: authority,
conservation, and a light client that cannot be fooled, with `#assert_axioms`
restricted to `propext`, `Classical.choice`, and `Quot.sound`. The crypto
floor described in `GOAL-VERIFIED-SYSTEM.md` is ML-DSA, ML-KEM, X25519, HKDF,
and SHA-256.

No Pallas Poseidon, no Halo 2 custom gate, and no Pasta Merkle path showed
up in that tree. The reusable pieces are the proof hygiene (axiom census,
spec file that loses to the Lean file when they disagree, interface
obligations explicitly left to the circuit) and the shape of a top-level
soundness theorem that quantifies over an adversarial transcript.

Searched and not found inside dregg: a Poseidon permutation, a PLONKish
constraint semantics, or a depth-17 Merkle argument. Recorded in
`.herdr/questions.md`.

## Ethereum Foundation / verified-zkEVM

Organization: https://github.com/Verified-zkEVM
Overview: https://github.com/Verified-zkEVM/Overview and https://verified-zkevm.org/resources/

| Repo | URL | Use for vk_id 3 |
| --- | --- | --- |
| clean | https://github.com/Verified-zkEVM/clean | The DSL Ironwood's circuits are written in. PLONK, AIR, and R1CS front ends. This is the embedding, not a Poseidon proof. |
| ArkLib | https://github.com/Verified-zkEVM/ArkLib | Interactive oracle reductions, sum-check, FRI/WHIR, Spartan, Merkle as a vector commitment. Docs: https://verified-zkevm.github.io/ArkLib/. Useful if someone later wants a different arithmetization. It is not a Halo 2 custom-gate library. |
| zkLean | https://github.com/Verified-zkEVM/zkLean | R1CS, aimed at Jolt-style zkVMs. Wrong arithmetization for this PLONKish circuit. |
| VCVio | https://github.com/Verified-zkEVM/VCVio | Game-based crypto. The place a Poseidon collision or discrete-log break would be named, in the Ironwood style. |
| rust-lean | https://github.com/Verified-zkEVM/rust-lean | Hax / Aeneas extraction. Subprojects include RISC Zero Merkle inclusion, Plonky3 FRI, and Mersenne31 / KoalaBear field arithmetic. Different fields, different hashes. Not a drop-in for Pallas `P128Pow5T3`. |
| CompPoly, leanerVM, riscv-zkvm | under the same org | zkVM / polynomial tracks. No claim-circuit overlap. |
| ethereum/EthLib (listed from the verified-zkEVM resource page as the shared Ethereum primitive library) | https://github.com/ethereum/cryptography-specs (EthLib) | BLS12-381, KZG, XMSS. No Pasta Poseidon. |

### Adjacent, not EF

- MavenRain/halo2-formal: https://github.com/MavenRain/halo2-formal
  Proves `LookupRangeCheck` soundness and completeness only. The note circuit
  does not use that gadget. Useful as a small example of a Halo 2 gate proof,
  superseded for us by Ironwood's `LookupRangeCheck.lean`.
- reilabs/proven-zk: https://github.com/reilabs/proven-zk
  Gnark R1CS extraction. Different stack. Not used.

## Primitive map for this circuit

| Component | Existing Lean | Gap |
| --- | --- | --- |
| Pallas `Fp` | Ironwood `Specs/Pallas` | Confirm zakura `pasta_curves` prime matches that spec. Expected to match; still a diff, not an assumption we skip in batch 0. |
| Poseidon x^5, `R_F = 8`, `R_P = 56`, width 3 | Ironwood `Poseidon/Pow5*.lean`, `Rounds.lean`, `Constants.lean` | Round-constant vectors must be compared to `halo2_poseidon` `fp::ROUND_CONSTANTS` and `MDS`. |
| `ConstantLength` sponge, L ∈ {2, 3, 4} | Ironwood `Poseidon/Hash.lean` `ConstantLength.value` | Generic in `L`. Need the in-circuit theorem that the chip's regions implement `value`, not only the value-level function. `SpongeTheorems.lean` is the value-level pad-and-add spec. |
| Boolean conditional swap | Ironwood `Utilities/CondSwap.lean` `SwapSpec` | Column assignment in the note circuit uses six advice columns shared with Poseidon. The gate proof transfers; the copy-constraint wiring into the next hash does not exist yet. |
| Merkle path of 17 independent swaps | Not a named theorem | Composition of `SwapSpec` with `ConstantLength<4>`. Straightforward once both sides are lemmas. Do not look for an Orchard Sinsemilla Merkle proof and relabel it. |
| Public-instance binding | Clean Halo 2 instance queries | Small. Six `constrain_instance` cells. |
| Halo 2 verifier | Ironwood `Zcash/Snark/` | Assumption, pinned by a VK fixture. |
| SHA-256 eligibility tree, ECDSA registration | None of the above (dregg has SHA-256, on a different statement) | Leave outside the circuit theorem. |
| secp256k1 foreign field | None suitable | The K=19 lab rewrote the chip after an unsound `sub`. No Lean block is started for it in this plan. |

## Default where a repo was thin

dregg does not contain the hash gadgets. The EF zkLean / ArkLib / rust-lean
line does not contain this Poseidon chip. The plan does not try to extract
the Rust with Hax. The circuit is a few dozen gadget calls; a Clean port in
the Ironwood style is the smaller proof.
