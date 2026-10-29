# K=19 ECDSA lab: vk3 vs ECDSA vs raw key vs blind two-step

Branch `wip/k19-lab`. Built on Terp zakura-halo2 only. Hardware: groot-wan (M1 Pro, 10 cores, 16 GB).
Raw data: `docs/k19-lab-benchmark.json`.
Harness: `circuits/halo2/src/lab_bench.rs`. Prove and verify are the native median of 5 runs.
Peak RSS is the whole test process. In the raw-key ladder and reg rows it also includes K=19 MockProver runs.

## 0. Critical finding: the secp256k1 chip is UNSOUND

`FpChip::add` and `FpChip::sub` only range-check their witnessed output. Nothing ties that output to a±b.
`enforce_equal` is `sub` followed by `enforce_zero`, so it is vacuous.
`foreign_mul_native` and `normalize` are each a single equation mod the Pallas modulus with a free quotient `u`/`k`. A free quotient does not imply an integer identity, so they need the CRT mod-2^t carry check as well.

Demonstration: the test `blind_k19::tests::blind_forgery_accepted_unsound_sub` uses a cfg(test) hook (`FORGE_EQ`/`FORGE_SUB`) that only changes which witness is assigned. With it:
- a WRONG sk, plus a nullifier the attacker computed, is **accepted** by MockProver;
- the same input is rejected with honest witness generation.

Consequence: anyone can claim for any eligible pubkey, with unlimited fresh nullifiers. This affects the K=19 ECDSA circuit, which uses the same mul/sub/div, and every variant below.
**None of the ECDSA or raw-key numbers here represent a sound circuit.** A proper CRT foreign-field implementation will add rows, and the size of that increase is unmeasured.

Other weaknesses found:
- In `double_point`, "two" and "three" are loaded with `load_private` and are not pinned.
- The Montgomery offset point is likewise loaded with `load_private`.
- sk is not range-checked against n.
- z1 and z2 are host bools.

## 1. Threat models

- **vk3 (K=11).** A public registration step reveals who registered. The anonymity set is only the set of registrants and grows as more people register.
- **K=19 ECDSA.** No registration step. The anonymity set is the full eligibility list.
  - Parity gap: the current leaf includes a per-user `rseed_com`, which effectively requires pre-registration.
  - Its nullifier comes from nk/rseed, not from the key. Note that a nullifier derived from an ECDSA signature would be non-unique, and one derived from pk would deanonymize.
- **Raw key (`blind_k19`, design 4).** No registration step. sk is a private witness and the circuit proves sk·G = pk.
  - leaf = Poseidon(leaf-tag, Poseidon(pkcom-tag, pk.x, pk.y), amount).
  - The Merkle tree has depth 17, uses the vk3 tags, and has the same public layout as vk3: [ctx, root, nf, dest_lo, dest_hi, amount].
  - nf = Poseidon(nf-tag, sk, ctx). This is deterministic per key and is not linkable to pk.
  - **Cost:** the user must export the raw private key out of the wallet into the prover. This is a UX cost and a key-exposure risk; hardware wallets cannot do it at all.
- **Blind two-step (design 3).** Step 1 is the registration circuit; step 2 is the unchanged vk3 claim. Details in section 4.

## 2. Results (measured)

Gas figures are an ESTIMATE from the host meter: units = 1 + ⌈proof/1024⌉ + ⌈inst/32⌉, at about 3.49k gas per unit. They were not measured on chain.

| design | K | rows | adv/fix cols | keygen s | prove s | verify s | proof B | PK MB | VK B | params MB | peak RSS GB | gas est |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| vk3 claim | 11 | 1,598 | 6/6 | 0.03 | 0.036 | 0.002 | 2,752 | 1.2 | 617 | 0.13 | 0.18 | ~34.9k (10 u) |
| K19 ECDSA (as shipped) | 19 | 215,228 | 10/10 | 54.9 | 31.4 | 1.49 | 4,800 | 638 | 1,225 | 33.6 | 3.77 | ~52.4k (15 u) |
| K19 ECDSA run at K=18 | 18 | 215,228 | 10/10 | 47.7 | **7.55** | 0.154 | 4,736 | 319 | 1,225 | 16.8 | 5.84 | ~52.4k |
| raw key, GLV | 19 | 107,916 | 9/7 | 11.6 | 23.1 | 0.74 | 4,224 | 487 | 937 | 33.6 | 4.19 | ~41.9k (12 u) |
| raw key, GLV at K=17 | 17 | 107,916 | 9/7 | 1.8 | **2.43** | 0.072 | 4,096 | 122 | 937 | 8.4 | 3.19 | ~38.4k (11 u) |
| raw key, Montgomery ladder (ea8ddf5) | 19 | 214,965 | 9/7 | 9.7 | 25.2* | 0.91 | 4,224 | 487 | 937 | 33.6 | 5.33 | ~41.9k |
| step-1 registration, GLV | 19 | 108,038 | 9/7 | 9.7 | 24.5 | 0.84 | 4,224 | 487 | 937 | 33.6 | 4.41 | ~38.4k (11 u) |

\* The ladder run overlapped a cargo build, so that timing is noisy. The row count is exact.

The K=19 timings vary widely (prove runs ranged from 23 to 54 s). This is likely memory pressure on a 16 GB machine.

## 3. Speedups and attempts (deltas)

1. **Right-size K.** This was the single biggest win.
   - The ECDSA circuit's rows (215k) fit in K=18: prove dropped from 31.4 s to 7.55 s (−76%), verify from 1.49 s to 0.154 s, and PK from 638 MB to 319 MB.
   - Raw-key GLV (108k rows) fits in K=17: prove dropped from 23.1 s to 2.43 s (−89%).
   - In both cases the honest proof verified and a wrong public input was rejected.
   - The production `circuit::K` was NOT changed; this was done via the `LAB_K19_K` and `BLIND_K` overrides.
2. **Fixed-base sk·G instead of ECDSA verify.** Rows went from 215k to 108k (−50%), and the circuit fits in K=17 instead of K=18. That confirms "fixed-base is much cheaper": 7.55 s → 2.43 s when each runs at its best K.
3. **Montgomery ladder (revived from headstash@ea8ddf5).** This was a regression: 215k rows versus 108k for GLV, because each bit costs an add plus two doubles. Kept only as the `KEYPAIR_LADDER` switch.
4. **Not attempted, deliberately:** micro-optimising the secp EC loops (Shamir/joint GLV, cheaper select, the zakura `orbits` MSM). It makes no sense to optimise an unsound gadget; the foreign-field arithmetic has to be rewritten first.

## 4. Blind two-step spec (design 3)

**Step 1, registration** (`REG_MODE`, which reuses `blind_k19`).
- Private: sk, pk, eligibility position and path, amount, secret.
- Public: [ctx, elig_root, reg_nf, relayer, reg_leaf].
- reg_leaf = note_leaf(key_commitment(secret), amount). This is exactly a vk3 leaf, appended to the registration tree.

**Step 2:** the vk3 claim (K=11), unchanged, against the registration-tree anchor.

Spec points:
- **Double registration.** reg_nf = Poseidon("terp-hs-regnf-v1", sk, ctx_reg).
  - It is deterministic per key and unlinkable to reg_leaf, because it uses a different domain tag and an independent secret.
  - It is separate from the claim nf tag.
  - It needs sk in-circuit. A signature-based reg_nf is malleable or non-unique, and a pk-based one deanonymizes. The alternative is a PLUME-style wallet nullifier.
- **Allocation binding.** The eligibility leaf commits to (pk_com, amount). reg_leaf binds the same `amount` cell. The vk3 claim exposes amount and destination.
- **Replay / ctx.** ctx is a public input of both steps, and reg_nf is per ctx_reg.
- **Front-running.** `relayer` (the registrant's own address or a relayer address) is a public input bound into the proof. If a mempool observer swaps it, the proof is rejected (tested). The step-2 destination is bound as in vk3.
- **Signed message.** None in the raw-key design. If an ECDSA variant is used, sign a fixed domain message plus ctx_reg plus relayer. A reusable public signature is fine only because it never leaves the prover: it must stay a private witness.
- **Leakage.** The registration tx reveals that *someone* eligible registered, the timing, and the amount if denominations are distinguishable. Mitigations: fixed denominations, batching registrations, a delay between step 1 and step 2, and using relayers.
- **Tests.** Honest proof accepted. Rejected: wrong sk, wrong position, wrong secret (reg_leaf mismatch), relayer swap.
- **Bloat versus monolithic raw key:** two proofs, two VKs (937 B + 617 B), about 38.4k + 34.9k ≈ 73k gas against about 38–42k, and an extra registration tx with a tree append. In return, step 2 stays K=11 (0.036 s, browser-friendly), and only step 1 is heavy (2.4 s at K=17 if right-sized; not separately measured at K=17).

## 5. Browser and wasm

Not measured: no wasm package was built in the lab. At K ≥ 17 the PK is 122–638 MB serialized and native peak RSS is 3.2–5.8 GB. That is at or above the wasm32 4 GiB address space, and `host-crypto` pulls in the libsecp C library.
vk3 (K=11) is browser-viable, per the existing `e2e/bench_wasm_prove.mjs` flow (not re-run here).
The heavy step (K=19, or step 1) is not viable in the browser as built. K=17 raw-key might be with a streamed or compact PK, but that is unmeasured.

## 6. Remaining ideas

- Rewrite FpChip as proper CRT: mod-2^t carries plus the native check, constrained add/sub, pinned constants, and an sk < n check. Then re-measure.
- Shamir/joint GLV for ECDSA, and the zakura `orbits` MSM backend for the prover.
- Wasm build at K=17.
- Real gas measurement via cw-zk-bench on a local chain (127.0.0.1).
- Note that verify CPU at K=19 (0.7–1.5 s) is far above what the flat gas meter prices (about 5 ms-equivalent): a DoS / mispricing risk.
