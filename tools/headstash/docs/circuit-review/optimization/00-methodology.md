# 00 — Methodology

Source: [rot256/skills `circuit-optimization`](https://github.com/rot256/skills/tree/master/circuit-optimization), tree `78d8e7c` (master at review time). Every file under that directory was fetched (`SKILL.md`, `CORPUS.md`, `references/*.md`, `scripts/README.md` and the Sage/SMT/Python scripts). `CORPUS.md` is a reading log for that skill and is not cited as a technique.

No Sage or cvc5 run. The skill's synthesizers (`scripts/synthesize.sage`, `exact_xag.sage`, `verify.smt2`, `cofactors.sage`) search small boolean or GF(2) gadgets. This circuit's only boolean is the Merkle swap bit, already the degree-2 check `b(1-b)=0`. There is no new gadget to synthesize. The host brief also forbids a workspace build and any prove at K≥20, so row counts below are from the layouter rules plus a placement simulation, reconciled to the published measurement of 1,598 rows. They were not re-derived by `MockProver`.

## Cost metric (workflow step 1)

Halo2 IPA on Pasta, as implemented in `zakura-common/crates/halo2_proofs`. The charged resources, in the order they actually move wall-clock and gas:

| Resource | What sets it here | What it does to a claim |
|---|---|---|
| Domain `2^K` | `NOTE_K = 11` (`note_membership.rs:43`). Params `Params::new(NOTE_K)` (`:262-264`). | Every advice MSM and the main FFT are size 2,048, whether or not the row is assigned. Proof bytes grow about one IPA round per extra K. |
| Committed advice columns | 6 configured advice columns, plus permutation-product columns created at keygen | One MSM and one proof-point each. Browser keygen commits them inside the wasm prove path. |
| Constraint degree `D` | `ConstraintSystem::degree` (`halo2_proofs/src/plonk/circuit.rs:1647-1674`). This circuit's max is 6. | `EvaluationDomain::new(degree, k)` sets `quotient_poly_degree = D - 1` and grows `extended_k` until `2^extended_k >= n * (D - 1)` (`halo2_proofs/src/poly/domain.rs:93-106`, called from `plonk/keygen.rs:100-102`). Quotient **commitments** are `D - 1` (linear). The quotient **FFT** jumps only when `extended_k` jumps. |
| Equality columns | 10 columns with `enable_equality` | Grand products come in chunks of `D - 2` (`halo2_proofs/src/plonk/permutation.rs:16-20`, `:125-128`). At D=6 that chunk is 4, so 10 columns are 3 product columns. |
| Filled rows | SimpleFloorPlanner start row (`halo2_proofs/src/circuit/floor_planner/single_pass.rs:95-106`) | Matter only if the total exceeds `n - (blinding_factors + 1)`. |
| Lookups | None configured | A lookup would add its own witness columns and a table height. There is nothing to trade against. |

On-chain verify does not read the row count. The host meter is `units = 1 + ceil(proof_bytes/1024) + ceil(instance_bytes/32)` (`tools/headstash/docs/onchain-verify-benchmark.md:40`). A 2,752-byte proof and 192 instance bytes are 10 units, about 34.9k gas.

`gates.md` prices a bespoke relation once for the whole system. That applies inside a shared VM. This VK is private to one circuit: a new selector is one fixed column in **this** key, committed at keygen, zero on the rows that do not use it. The break-even test from `gates.md` still holds in the form that matters here: a row saving counts only when it crosses a power of two (`gates.md`, "The break-even is a padding question").

## What was applied

- **PLONKish wiring** (`references/plonkish.md`). Copy constraints are not free. Rotations are free inside a region and are how `Pow5Chip` threads round state (`Rotation::prev/cur/next` on the pad-and-add gate). Regions do not share rows unless their column sets are disjoint. Advice columns 0–3 are the critical path; column 5 is almost empty and that emptiness is not a row win.
- **Degree** (`references/degree.md`), restated for this IPA backend rather than FRI. `D ∈ {6,7,8,9}` shares `extended_k = 14` at `n = 2048` (because `2048 * 8 = 16384`). `D = 5` drops to `extended_k = 13` and also shrinks the permutation chunk from 4 to 3, which **adds** a grand-product column (10 columns / 3 = 4 sets, versus 3 sets today). The x^5 S-box is what pins D at 6. Spending the headroom up to 9 adds quotient commitments one-for-one and does not shrink the FFT domain.
- **Selector grouping** (`references/gates.md`, `degree.md` "Selector Group Packing"). `compress_selectors` will not raise degree. The two degree-6 Poseidon selectors cannot take a partner. The degree-2 pad selector and the degree-3 swap selector can share one fixed column. Derived, not dumped from a proving key (no keygen was run).
- **Boolean ZK** (`references/boolean-zk.md`). The swap bit is a witnessed boolean with a determining check. There is no word-AND circuit, so GF(2) XAG counts and `r1cs-gf2.md` do not apply. The N1 bug (free Poseidon tags) is the skill's don't-care rule: prover freedom that reaches an output.
- **Technique catalogue** (`references/techniques.md`). Poseidon rounds are already compute-then-check (the prover supplies the next state; the gate forces it). Lookups are the wrong tool for 17 booleans. A custom higher-arity gate is a bespoke relation whose row win does not cross K.

## What was read and does not apply

| Reference | Why it does not change this review |
|---|---|
| `references/r1cs-fp.md`, `references/r1cs-gf2.md` | R1CS / GF(2) row costs. This is PLONKish IPA. |
| `references/air.md` | AIR area law. No AIR, no bus, no lookup. |
| `references/binary-extension-fields.md` | Native field is Pallas, a prime field. |
| `references/sumcheck.md` | Prover scheduling for sumcheck. Halo2 IPA is not sumcheck. |
| `references/deferral.md` | Moving work into a second proof. The K=15 designs already are that experiment; see `06-k15-comparison.md`. A co-processor is not an optimization of vk_id 3. |
| `references/smt.md`, `references/sage.md`, `scripts/*` | Certificates for a new small constraint. Not used. The CRT/RNS warnings in those files are the reason the secp chip on `wip/k19-lab` had to be rewritten; they do not apply to a Poseidon-only circuit. |
| `CORPUS.md` | Bibliography. Not a technique. |

## Baseline before any proposal

- Rows assigned: **1,598** (floor-plan simulation in `02-accounting.md`; same figure as the vk3 row of `wip/k19-lab:tools/headstash/docs/k19-lab-benchmark.md`).
- K: **11**, and it is minimal for depth 17.
- Advice / fixed configured: **6 / 6**, plus 4 simple selectors and 1 instance column.
- Degree: **6**. Quotient commitments: **5**. Extended domain: **2^14**.
- Permutation sets: **3**.
- Lookups: **0**.
- Proof: **2,752** bytes. Verify units: **10**.

Any proposal below is scored against that baseline. A change that saves rows and stays at K=11 is recorded as a prover no-op for MSM size, proof bytes, and gas.
