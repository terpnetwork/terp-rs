# 01 — Constraint and column inventory

Circuit: `NoteCircuitImpl<true>` (`tools/headstash/circuits/halo2/src/note_membership.rs`). Chips: `Pow5Chip` / `P128Pow5T3` and `CondSwapChip` from `zakura-common/crates/halo2_gadgets`. No lookup argument is registered. `ConstraintSystem::lookup` is never called from `configure`.

## Configured columns

`NoteCircuitImpl::configure` (`note_membership.rs:108-134`):

| Column | Index | Role | Equality | Who queries it |
|---|---|---|---|---|
| Instance | 0 | Public inputs, 6 rows | yes (`:110`) | `constrain_instance` only. Not queried by a gate. |
| Advice 0 | 0 | Poseidon `state[0]`; CondSwap `a` | yes (`:119-121`, again in `pow5.rs:76-81`) | prev, cur, next |
| Advice 1 | 1 | Poseidon `state[1]`; CondSwap `b` (sibling witness) | yes | prev, cur, next |
| Advice 2 | 2 | Poseidon `state[2]` (capacity); CondSwap `a_swapped` (left child) | yes | prev, cur, next |
| Advice 3 | 3 | Poseidon `partial_sbox`; CondSwap `b_swapped` (right child) | yes | cur only (partial gate, `pow5.rs:116`) |
| Advice 4 | 4 | CondSwap `swap` bit; also `dest_hi` load and per-layer constant | yes | cur only (swap gate) |
| Advice 5 | 5 | Pinned domain tags (`fixed`, `note_membership.rs:186-204`) | yes | not queried by any gate. Equality exists so `constrain_constant` can copy the cell onto the constants column. |
| Fixed `rc_a[0..3]` | 0..3 | Round constants for the current round (`pow5.rs:59`, assigned in `pow5.rs:548-559`) | no | cur, inside both round gates |
| Fixed `rc_b[0..3]` | 3..6 | Second-round constants of a partial pair (`pow5.rs:494-505`); `rc_b[0]` is also the constants column (`enable_constant`, `note_membership.rs:124`) | yes (`pow5.rs:76-81`) | cur |
| Selector `s_full` | 0 | Full S-box round | simple | `pow5.rs:83` |
| Selector `s_partial` | 1 | Two partial rounds | simple | `pow5.rs:84` |
| Selector `s_pad_and_add` | 2 | Sponge absorb | simple | `pow5.rs:85` |
| Selector `q_swap` | 3 | Conditional swap | simple | `cond_swap.rs:243` |

The k19 lab table's "6 advice / 6 fixed" is this configured set (`rc_a` plus `rc_b`). Selectors are not fixed columns until `compress_selectors`.

`Pow5Chip::configure` (`pow5.rs:55-61`) is called with `state = advice[0..3]`, `partial_sbox = advice[3]`, and the six fixed columns (`note_membership.rs:125-131`). `CondSwapChip::configure` takes `advice[0..5]` (`note_membership.rs:132`; mapping in `cond_swap.rs:239-251`).

There is no unrouted-wire split. Every advice column is equality-enabled, so every advice column joins the permutation argument. That matches the PLONKish rule that a copy is a committed product column, not a free wire (`plonkish.md`, "A copy constraint is not free").

## Gates

Spec constants (`zakura-common/crates/halo2_poseidon/src/p128pow5t3.rs:17-24`): `full_rounds = 8`, `partial_rounds = 56`, width 3, rate 2, S-box `x^5`. Both round counts are even, which `Pow5Chip::configure` requires (`pow5.rs:65-68`).

Degree below is the raw gate polynomial, selector included (`Expression::Selector` has degree 1, `circuit.rs:804`). `Constraints::with_selector` multiplies every polynomial by the selector (`circuit.rs:1043-1057`).

| Gate | Where | Selector rows per permutation (or per use) | Polynomials | Degree | What it forces |
|---|---|---|---|---|---|
| full round | `pow5.rs:92-112` | 8 (4 then 4, offsets in `pow5.rs:238-257`) | 3, one per next state word | **6** = selector × `(state + rc)^5` | `state_next = MDS · (state + rc_a)^5` |
| partial rounds | `pow5.rs:114-158` | 28 (each row is two partial rounds) | 4: S-box into `partial_sbox`, S-box of the mid state, two linear lanes for `state[1]` and `state[2]` | **6** | Two partial rounds. Only lane 0 is powered; lanes 1 and 2 stay linear. `rc_b` holds the second round's constants. |
| pad-and-add | `pow5.rs:160-184` | 1 row inside a 3-row region (selector at offset 1, `pow5.rs:312`) | 3: two rate lanes `prev + input - next`, capacity `prev - next` | **2** | Absorb one rate-sized block. Padding, when present, is a fixed cell copied into the input row (`pow5.rs:334-354`). |
| cond swap | `cond_swap.rs:256-284` | 1 per Merkle layer | 3: `a'`, `b'`, boolean | **3** | `a' = b·s + a·(1-s)`, `b' = a·s + b·(1-s)`, `s(1-s)=0`. Ternary is `utilities.rs:177-179`. Boolean is `range_check(s, 2)` (`utilities.rs:169-171`, `:234-239`), i.e. `s(1-s)`. |

The partial-round gate is already the standard 2-for-1 packing. A full round cannot use that packing: every lane is nonlinear, so the gate is one round per row.

No gate exceeds degree 6, and nothing is degree 4 or 5. The system degree is therefore 6 (`circuit.rs:1647-1674` takes the max over permutation required-degree 3, lookup required-degree, and gate degrees).

### After selector compression (derived)

`compress_selectors` (`halo2_proofs/src/plonk/circuit/compress_selectors.rs:137-185`) packs simple selectors whose rows are disjoint, without raising the system degree. Selector max-degrees are 6, 6, 2, 3 in creation order (`s_full`, `s_partial`, `s_pad_and_add`, `q_swap`).

- `s_full` starts a combination with body degree 5. A second member would push the substituted degree to at least 7, which is above 6. It stays in its own fixed column.
- `s_partial` likewise stays alone.
- `s_pad_and_add` (body degree 1) can take `q_swap` (body degree 2). They never fire on the same row. One shared fixed column. Substituted degrees become 3 (pad) and 4 (swap). Both still sit under 6.

Post-compression fixed columns: the original 6, plus 3 combination columns, **9 fixed columns in the verifying key**. The two high-degree selectors are the "degree-6 gate owns its selector" case from `gates.md`. This count was not read back from a serialized VK.

## Regions and rows, per gadget

`Hash::hash` (`halo2_gadgets/src/poseidon.rs:268-285`) absorbs `L` message words plus `k·2 - L` zero pads, where `k = ceil(L / 2)` (`halo2_poseidon/src/lib.rs:404-411`). The absorb buffer holds 2 words (`lib.rs:219-227`). It permutes when a word arrives and the buffer is already full, and `finish_absorbing` always permutes the last buffer (`poseidon.rs:168-174`, `:182-192`). So each hash does `k` permutations, each permutation is one `add_input` region plus one `permute` region.

Region heights, from the assign offsets:

| Region | Height | Columns in the rectangle | Source |
|---|---|---|---|
| `initial_state` | 1 | advice 0, 1, 2 | `pow5.rs:274-296`. Three `assign_advice_from_constant` calls. The constants themselves are written later onto `rc_b[0]` and do not enlarge this region (`single_pass.rs:118-144`; `RegionShape` ignores `constrain_constant`, `layouter.rs:345-347`). |
| `add_input` | 3 | advice 0, 1, 2; `s_pad_and_add`; plus `rc_b[i]` only when word `i` is padding | `pow5.rs:309-386` |
| `permute` | 37 | advice 0, 1, 2, 3; all six `rc_*`; `s_full`; `s_partial` | Load at row 0 (`pow5.rs:511-519`). Eight full rounds and 28 partial rows write the next state at `offset+1` (`pow5.rs:537-570`). Last write is offset 36. `8 + 28 + 1 = 37`. |
| witness load | 1 | one advice column | `note_membership.rs:175-181` |
| pinned constant | 1 | one advice column, then one constants-column cell | `note_membership.rs:186-204` |
| swap | 1 | advice 0–4 and `q_swap` | `cond_swap.rs:93-134` |

Hashes in `synthesize` (`note_membership.rs:150-165`):

| Call | `L` | Permutations | Init | Add rows | Perm rows | Critical-path rows |
|---|---:|---:|---:|---:|---:|---:|
| `key_com` = Poseidon(tag, secret) | 2 | 1 | 1 | 3 | 37 | 41 |
| `nullifier` = Poseidon(tag, secret, ctx) | 3 | 2 | 1 | 6 | 74 | 81 |
| `leaf` = Poseidon(tag, key_com, amount) | 3 | 2 | 1 | 6 | 74 | 81 |
| each Merkle CRH = Poseidon(tag, layer, left, right) | 4 | 2 | 1 | 6 | 74 | 81 |
| 17 CRHs | 4 | 34 | 17 | 102 | 1,258 | 1,377 |

`L=3` and `L=4` cost the same number of permutations. The extra word in the length-3 hash is a padding zero, not a saved round. Constant-length domain separation sits in the capacity element (`(L as u128) << 64`, `halo2_poseidon/src/lib.rs:398-401`) and costs no extra round.

Total permutations: `1 + 2 + 2 + 34 = 39`. The same count is `5 + 2·depth` at depth 17.

Active selector rows: `39·8` full + `39·28` partial + `39` pad + `17` swap = `312 + 1,092 + 39 + 17 = 1,460`. The other 138 of the 1,598 rows are wiring (initial states, the output row of each permutation, the two non-selector rows of each absorb, and the secret load). See `02-accounting.md`.

## Instance column

Six equality copies, in order (`note_membership.rs:167-169`):

| Row | Cell | Meaning |
|---:|---|---|
| 0 | `ctx` | Campaign context. For the direct path this is `direct_ctx(chain_id, contract)` (`direct.rs:49-60`). |
| 1 | Merkle root | The node after 17 CRHs. |
| 2 | nullifier | `Poseidon(terp-hs-nf-v1, secret, ctx)`. |
| 3 | `dest_lo` | Low 31 bytes of the destination string (`no_ecdsa.rs:277-291`). |
| 4 | `dest_hi` | Remaining bytes, starting at index 31. |
| 5 | `amount` | The denomination in uterp, as a field element. Not range-checked in the circuit. |

`NOTE_INSTANCES = 6` (`note_membership.rs:44`) is the same width the contract rebuilds in `direct.rs:85-94` (192 bytes, six 32-byte little-endian Pallas representations). `claim_note` verifies that buffer with `proof_instance_verify` (`direct.rs:160-166`, `:224-225`) and pays `info.sender` only after `pack_destination(info.sender)` matches those limbs.

The circuit does not hash the destination. Binding is the equality between the witness cell and the instance the contract supplies. A proof for a different payee fails verification because the instance bytes differ, which is what `onchain-verify-benchmark.md` records for a swapped amount limb and a swapped nullifier.

## What is not in the circuit

- No ECDSA, no secp limb, no lookup table, no range check on `secret`, `amount`, `ctx`, or the destination bytes.
- The Merkle index is not a public input. It exists only as 17 independent swap bits. That is the right encoding: the bits are the index.
- `amount` is constrained only by being a public input to the leaf hash. `prove_note` refuses a non-denomination (`note_membership.rs:296-298`), and the contract checks `DENOMS_UTERP` again (`direct.rs:208-210`). A non-denomination leaf is not in the registration tree, because registration inserts `note_leaf(key_com, DENOMS[class])` (`notes.rs:1-4`).
- The eligibility root `c6ffae12…` (`headstash-denominations-spec.md:58-59`) is the **sha256** registration-eligibility tree. It is not an instance of this circuit. The circuit's root instance is a Poseidon claim-tree anchor, accepted or rejected by `anchor_known` (`direct.rs:205-207`).

## Pinned constants

With `PIN = true`, every tag and every layer index goes through `constrain_constant` (`note_membership.rs:186-204`). Count: 3 tags before the loop (key-commitment, nullifier, leaf), plus per layer a CRH tag and a layer index, plus 3 constants inside every `initial_state`. At depth 17 that is `3 + 17·2 + 20·3 = 97` constants, all appended on `rc_b[0]`.

Tags the prover is not allowed to choose (`note_membership.rs:46-48`, `distro_poseidon.rs:103`):

| Role | String |
|---|---|
| key commitment | `terp-hs-keycom-v1` |
| nullifier | `terp-hs-nf-v1` |
| leaf | `terp-hs-note-leaf-v2` |
| Merkle CRH | `terp-hs-distro-crh-v1` |

`headstash-denominations-spec.md:66` writes the nullifier personalization as `terp-hs-nullifier-v1`. The circuit and `claim_nullifier` (`no_ecdsa.rs:243-244`) use `terp-hs-nf-v1`. The VK hash `34d11437…5398` commits to the code, not to that sentence. See `05-do-not-change.md`.
