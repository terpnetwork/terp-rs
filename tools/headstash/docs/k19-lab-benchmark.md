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


## Sound foreign-field chip (commit 02764a5)

The old secp256k1 chip was unsound: its `sub` output was only (non-strictly) range-checked, native
cells were unconstrained, and constants were free witnesses. A prover could pass `enforce_equal`
with a key it did not own (`blind_forgery_accepted_unsound_sub`). It has been replaced.

### Arithmetic
* Elements: 4 x 64-bit limbs plus a native (Pallas) cell; a compose gate binds native = sum l_i 2^(64i).
* Every limb is lookup range-checked (10-bit table, strict) when it is created: 64 bits per limb, 70 bits per quotient limb, 80 bits per offset carry.
* mul: witness q, r with a*b = q*p + r. For each of the 7 limb positions k:
  sum_{i+j=k} a_i b_j - q_i p_j - r_k + c_{k-1} = 2^64 c_k, where c_k = c'_k - 2^79 and c'_k is range-checked. The top carry is forced to 0. There is also a native-modulus check, a*b - q*p - r = 0 mod Pallas.
* add/sub: a +- b = r + q*p with a boolean q, small carries c' in {0,1,2} and the same per-limb chain.
* assert_canonical (x < m): a borrow chain checks m-1-x >= 0 with range-checked difference limbs.
* div_checked: b*inv = 1 (non-zero divisor). It is used for add_point (dx) and ECDSA s^-1. Doubling keeps plain div because 2y != 0 on secp256k1.
* Constants are pinned through constrain_constant on every limb: two/three are replaced by additions, and the ladder offset, generator tables and ECDSA one are pinned.
* select binds cond, limbs and native. GLV k = k1 + k2*lambda mod n is now constrained through the sound Fq ops.
* sk < n and canonical pk.x, pk.y < p are enforced in prove_key_pairing.

### Soundness argument (sketch)
Every limb is range-checked, so |a_i b_j|, |q_i p_j| < 2^134. Each column sum is < 2^137, and each per-limb equation is < 2^145, far below the Pallas modulus (about 2^254). The limb equations therefore hold over the integers rather than mod Pallas. Telescoping them with a zero top carry gives the integer identity a*b = q*p + r, so r = a*b mod p (up to a non-canonical representative < 2^256). Canonicity is imposed where uniqueness matters (sk, pk, the nullifier input). The native check is redundant but cheap. Add and sub follow the same argument with q in {0,1}. With pinned constants and a bound select, no free witness remains in the EC formulas except lambda, which the mul identity fixes.

### Negative tests (MockProver, K=19; forge hooks are #[cfg(test)] only)
| case | result |
|---|---|
| blind_forgery_rejected (eq-sub forge, wrong sk) | rejected |
| forged add / sub / mul output | rejected / rejected / rejected |
| mul quotient out of range | rejected |
| mul remainder limb out of range | rejected |
| unpinned constant | rejected |
| sk >= n (sk+n) | rejected |
| bad GLV decomposition (k1+1) | rejected |
blind_soundness_suite also passes: the honest claim is accepted and 9 tamper cases are rejected.

### Before / after (raw-key GLV keypair, native, groot-wan, median of 5)
| | unsound (rawkey_glv_k19) | sound (rawkey_glv_sound_k19) |
|---|---|---|
| rows | 107,916 (fits K=17) | 453,787 (smallest K=19) |
| prove median | 23.1 s @K19 (2.43 s @K17) | 28.2 s |
| verify median | 0.74 s | 1.14 s |
| PK / VK | 486.5 MB / 937 B | 503.3 MB / 969 B |
| proof | 4,224 B | 4,640 B |
| peak RSS | 4.19 GB | 4.18 GB (single-run, /usr/bin/time) |

Not measured: sound ECDSA, the ladder and two-step registration overflow K=19 (NotEnoughRowsAvailable), and K=20 was not run. The wasm32 prover was not built for the sound chip.


## Fixed-base raw-key claim (w=8 window table, ccd2cb3 / 3959095 / audit 15b8d21)
sk·G = Σ_{w<32} T_w[d_w] from a fixed lookup table keyed (window, digit) carrying x/y limbs: 4 digit-decomposition regions, 32 table lookups, 31 incomplete adds, no doublings. Adds 1..30 use plain div; the last add keeps div_checked. Rows: 28,484 (all div_checked) → 24,584 (plain div), smallest K=15.

| K | rows | prove median | verify | PK | proof | peak RSS |
|---|---|---|---|---|---|---|
| 15 | 24,584 | 0.94 s | 26 ms | 46.1 MB | 5,088 B | 1.70 GB |
| 16 | 24,584 | 1.57 s | 35 ms | 92.3 MB | 5,152 B | 2.02 GB |
| 17 | 24,584 | 3.30 s | 64 ms | 184.6 MB | 5,216 B | 3.57 GB |

Native, groot-wan, 10 threads, median of 3.

Wasm (733486b, node, single thread, params+keygen in client):

| K | params | vk | pk | prove | verify | heap |
|---|---|---|---|---|---|---|
| 15 | 27.2 s | 8.4 s | 10.4 s | 26.8 s | 0.47 s | 786 MB |
| 17 | 124.9 s | 23.6 s | 47.3 s | 105.1 s | 1.6 s | 3.12 GB |

### Plain-div audit (verdict: sound, no row change)
1. Digits are unique: each d_w is range-checked to [0,256) by the table lookup, and the linear gate ties 8 digits to each 64-bit limb (sum < 2^64 << Pallas p), so d is a deterministic function of sk.
2. The lookup is one tuple (window fixed column, d copied, 5 x-limb/native, 5 y-limb/native cells) and the window column is fixed and non-zero, so the zero row is unreachable. The selected point is exactly T_w[d_w] = (d_w+2)·2^{8w}·G; the prover has no freedom in table selection.
3. Induction: if every earlier add had dx ≠ 0, λ is fixed by c·dx ≡ dy, so acc_w is the honest point a·G with a = Σ_{i<w}(d_i+2)2^{8i} ∈ [2(2^{8w}−1)/255, 257(2^{8w}−1)/255].
4. For 1 ≤ w ≤ 30, a < 2·2^{8w} ≤ t_w = (d_w+2)2^{8w}, and a + t_w < 2^{249} < n. So a ≢ ±t_w (mod n), acc ≠ ±T_w, and x_acc ≠ x_T. G has prime order, so dx ≠ 0 for every witness. Plain div with dx = 0 would need acc = T exactly (dy = 0 too, any λ), and that cannot be reached.
5. At w = 31 the offset −C·G makes acc = ±T reachable, so div_checked stays there (acc = −T only when sk = 0; acc = T is a completeness loss only).

Tests: `fb_plain_div_scalar_bound` (exhaustive bound for w = 1..30) passes. The new negative `FB_COLLIDE` sets the window-1 point to acc (dx = dy = 0, so plain div accepts λ = 0), and it is rejected by the lookup. blind_negative_suite rejects 10/10, blind_forgery_rejected rejects the forgery, and blind_soundness_suite passes (MockProver K=19).
