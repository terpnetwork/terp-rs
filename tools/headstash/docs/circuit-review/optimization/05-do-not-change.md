# 05 — What not to change

Each item is an invariant of vk hash `34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398` (`note_membership.rs:638`, copied into the contract at `notes.rs:22-26`). Changing it is a new circuit, a new gov store, and a new campaign, not a cleanup.

## Pinned tags and layer indices

`fixed::<PIN>` with `PIN = true` copy-constrains every personalization and every layer index (`note_membership.rs:150-164`, `:186-204`). vk_id 2 left those cells as free advice. A prover could substitute a nullifier tag and produce another nullifier for the same secret. That is finding N1 (`tools/headstash/docs/security-review-2026-10-26.md`, and the regression `pinned_tags_reject_a_forged_nullifier` at `note_membership.rs:566-585`).

The row cost of the pins is zero on the critical path. Tags land on advice 5, which ends at row 20 (`02-accounting.md`). Unpinning them saves no K and restores the unlimited-nullifier bug.

Do not "correct" the nullifier string to `terp-hs-nullifier-v1` from `headstash-denominations-spec.md:66`. The hash that is actually constrained is `terp-hs-nf-v1` (`note_membership.rs:47`, `no_ecdsa.rs:243-244`). The spec sentence is stale. The VK commits to the code.

The other strings are `terp-hs-keycom-v1`, `terp-hs-note-leaf-v2`, and `terp-hs-distro-crh-v1` (`note_membership.rs:46-48`, `distro_poseidon.rs:103`).

## The Poseidon parameters

`P128Pow5T3`: width 3, rate 2, 8 full rounds, 56 partial rounds, S-box `x^5` (`halo2_poseidon/src/p128pow5t3.rs:17-24`). `ConstantLength` capacity is `L << 64` (`halo2_poseidon/src/lib.rs:398-401`). The off-circuit tree builder uses the same function (`distro_poseidon.rs:232-242`, `note_membership.rs:217-231`). A faster in-circuit hash that does not match `poseidon_distro_crh` proves membership in a different tree than the one registration appended.

## Depth 17 for this campaign

98,841 notes do not fit in `2^16` leaves (`note_membership.rs:5-6`). Reducing depth is not an optimization. Raising it past 22 on this chip forces K=12 (`02-accounting.md`).

## Six public inputs, in this order

`[ctx, root, nullifier, dest_lo, dest_hi, amount]` (`note_membership.rs:167-169`). The contract rebuilds those bytes itself (`direct.rs:7-14`, `:85-94`) and passes them to `proof_instance_verify` (`direct.rs:160-166`). The prover does not supply the instance vector on chain. That is what makes destination binding and the amount check hold:

- `dest_lo` / `dest_hi` are `pack_destination(info.sender)` (`direct.rs:63-77`, `:224`). The circuit only copy-constrains the witness to the instance. Removing either limb, or hashing them inside the circuit without changing the contract, pays the wrong account or fails every honest proof.
- `amount` is both a leaf input and an instance, so the contract can enforce the denomination cap (`direct.rs:208-214`) on the same value the Merkle leaf commits to.
- `ctx` for the direct path is `sha256("terp-hs-direct-ctx-v1" || 0 || chain_id || 0 || contract)` with the top two bits cleared (`direct.rs:49-60`). A proof from another contract or another chain fails the instance check.
- `nullifier` is stored and rejects a second spend (`direct.rs:220-223`). It depends on `secret` and `ctx`, not on the root, so the same note cannot be replayed against a later anchor. That is the comment at `direct.rs:11-12`.
- `root` must already be an accepted anchor (`direct.rs:205-207`). The circuit does not know which roots the contract has stored.

Do not pack these into one instance on this VK (opportunity O7). Do not add the eligibility root, the signature, or the identity to the instance list. Those are registration-time facts. The claim proof is deliberately unlinkable to the identity that registered the `key_com`.

## Equality on the state columns, and the 3-rotation window

The sponge copies prior-round state into the next region (`pow5.rs:315-324`, `:516-519`). Those columns have to be equality-enabled. The pad gate's `prev/cur/next` queries are why `blinding_factors = 5` (`circuit.rs:1679-1703`). A gate that reads `Rotation(-2)` or `Rotation(2)` increments `num_advice_queries` past 3, raises blinding, and shrinks usable rows (`2^K - blinding - 1`). The current window is the widest one that does not tax the usable height.

## The split between circuit and contract

Keep these out of the circuit:

| Check | Where it lives | Why it stays there |
|---|---|---|
| EIP-191 or ed25519 registration signature | `notes.rs` `verify_signer` (~`:427`) | A sound in-circuit secp check is the K=15 circuit, ~15× the rows. v1 publishes `key_com` at registration and proves knowledge of `secret` at claim. |
| Denomination table and per-class cap | `direct.rs:208-214` | The leaf is only inserted for `DENOMS[class]`. Repeating the table in-circuit costs rows and a second list to keep in sync. |
| Anchor membership | `direct.rs:205-207` | The circuit proves "this root is the fold of this path". The contract proves "this root was accepted". Both are required. |
| Nullifier uniqueness | `direct.rs:220-223` | A proof can be recomputed. The set is what makes it one-time. |
| Payee | `pack_destination(info.sender)` | Bech32 stays a string function. The circuit sees two field elements. |
| Canonical 32-byte instance limbs (`high byte ≤ 0x3f`) | `direct.rs:97-101` | Rejects non-canonical encodings before verify. |

## Stock chip structure

Do not replace `Pow5Chip`'s 2-partial packing, the separate `add_input` region, or `CondSwapChip` in order to fill advice 5. Advice 5 being almost empty is a consequence of tagging on a side column. Filling it means a second hash lane, and the Merkle chain is serial (opportunity O4).

Do not disable `enable_equality` on `rc_b` as a drive-by. One of those columns is unused for copies, and dropping it does not remove a permutation set (opportunity O3).

## Proof and param constants the contract already stored

`NOTE_K = 11`, `NOTE_PROOF_BYTES = 2752`, blob size 135,023 bytes of which 131,140 are params (`onchain-verify-benchmark.md:9-11`, `note_membership.rs:639`). `proof_instance_verify` is metered from those sizes. A shorter proof that is not exactly this IPA transcript will not verify under the stored key, and a longer one that crosses 3,072 bytes moves the gas unit.

The direct-claim body string `headstash-note-claim-v2:` (`note_membership.rs:241-248`) is outside the constraint system. It is what wallets and the lane display. Changing the circuit does not require changing it; changing it does not change the VK. Leave it stable so verifiers and indexers keep one encoding.
