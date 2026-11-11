# 03 — Optimization opportunities

Scored against the baseline in `00-methodology.md`. "Rows saved" is critical-path rows (advice 0). A saving that leaves the total above 1,018 does not change K, the 2,048-point MSMs, the 2,752-byte proof, or the 10 verify units.

Verify gas for the current proof: `1 + ceil(2752/1024) + ceil(192/32) = 1 + 3 + 6 = 10` units ≈ 34.9k gas (`onchain-verify-benchmark.md:26`, `:40`). The next proof-size bucket starts at 3,072 bytes (+320 bytes). Instance gas moves only if the instance count moves.

Browser time is dominated by in-wasm keygen plus MSMs at K=11. Measured single-thread prove, keygen included, is about 3.2–3.4 s (`frontend-onchain-spec.md:227`; `headstash-denominations-spec.md` records 3.35 s). Native prove in the k19 lab is 0.036 s median. None of those runs is repeated here.

## O1. Cut rows inside K=11 (any local packing)

Examples that were checked and do **not** reach K=10:

| Change | Rows saved | New total | Still above 1,018? |
|---|---:|---:|---|
| Delete all 17 swap rows (boolean check moved elsewhere) | 17 | 1,581 | yes |
| Delete all 20 initial-state rows (reuse one sponge state) | 20 | 1,578 | yes |
| Both | 37 | 1,561 | yes |
| Drop depth from 17 to 16 | 82 | 1,516 | yes, and `2^16` leaves cannot hold 98,841 notes |
| Repack partial rounds 4-per-row instead of 2-per-row, saving 14 rows × 39 perms | 546 | 1,052 | yes, short of K=10 by 34 rows |

K=10 needs at least 580 rows gone. The 39 permutation rectangles alone are `39 × 37 = 1,443` rows, which is already more than 1,018. Getting under 1,018 means deleting about 16 permutations (`580 / 37`), around 8 Merkle layers, or replacing the hash. Depth 9 is the greatest depth whose `rows(d) = 204 + 82d` fits in 1,018 (`204 + 82·9 = 942`, `204 + 82·10 = 1,024` which is over). Production depth is 17.

**Savings:** 0 columns, 0 degree, K unchanged for every change that preserves depth and `P128Pow5T3`.
**Soundness:** a 4-partial round gate is a new relation. The current 2-partial gate is the audited halo2 gadget. Rewriting it to save 546 rows that do not change K is risk for no gas or proof-size win.
**Verify gas:** unchanged.
**Prove time:** assignment and gate evaluation get cheaper by the fraction of rows removed. The MSMs stay size 2,048. That fraction is noise next to keygen. Not worth a VK migration (`NOTE_VK_HASH` is pinned, `notes.rs:22-26`).

## O2. Spend degree headroom (D = 6 → 7, 8, or 9)

At `n = 2048`:

| D | Quotient commitments `D-1` | `n·(D-1)` | `extended_k` | Perm chunk `D-2` | Sets for 10 equality columns |
|---:|---:|---:|---:|---:|---:|
| 5 | 4 | 8,192 | 13 | 3 | 4 |
| **6** | **5** | **10,240** | **14** | **4** | **3** |
| 7 | 6 | 12,288 | 14 | 5 | 2 |
| 8 | 7 | 14,336 | 14 | 6 | 2 |
| 9 | 8 | 16,384 | 14 | 7 | 2 |
| 10 | 9 | 18,432 | 15 | 8 | 2 |

`extended_k` is flat from 6 through 9. Raising degree does not grow the quotient FFT. It adds one proof point and one quotient MSM per extra degree. Dropping to 5 (only possible if the `x^5` S-box is split into lower-degree steps, which adds rows and an advice cell) halves the quotient domain and adds a fourth grand-product column.

Permutation-chunk arithmetic at D=9 saves one product column (3 sets → 2) and adds three quotient commitments. Net **+2** committed polynomials, +64 proof bytes, still inside the 3,072-byte gas bucket.

**Savings:** no rows, no K change. Proof +32 bytes per degree above 6.
**Soundness:** a higher-degree custom gate is a new constraint. The existing degree-6 gates are the stock Poseidon chip.
**Verify gas:** unchanged until the proof crosses 3,072 bytes. Five extra quotient points would be +160 bytes (2,912 total), still 10 units.
**Prove time:** same extended domain, more quotient MSMs. A loss.
**Verdict:** do not spend the headroom. `degree.md`'s "round up to the bracket top" is the FRI/KZG-chunk rule, where chunks are a power of two. This IPA backend commits exactly `D-1` pieces (`domain.rs:95`, `vanishing/verifier.rs:65`). Extra degree is not free here; only the FFT size is flat.

## O3. Drop equality on columns that are never copied

`rc_b[2]` is equality-enabled for the whole `rc_b` array (`pow5.rs:76-81`). Padding only writes rate indices `0..2` (`pow5.rs:334-341` with `RATE = 2`), and `rc_b[0]` is the constants column. `rc_b[2]` is round constants only (`assign_fixed`, no copy). Disabling equality on it takes the set from 10 columns to 9.

`ceil(9/4) = 3`. The set count does not move. The next cliff is 8 columns. The other nine equality columns are actually copied: six advice columns (sponge I/O, swap outputs, instance copies, constant copies) and `rc_b[0]`, `rc_b[1]` (padding on the length-3 hashes).

**Savings:** none at the commitment cliff.
**Soundness:** forking `Pow5Chip` to special-case one column can desync the gadget from the host hash.
**Verify gas / prove time:** unchanged.
**Verdict:** do not fork the chip.

## O4. A second Poseidon lane (wider circuit, fewer rows)

Two permutations side by side need a second state (3 advice), a second `partial_sbox`, and their own selectors, roughly +4 advice columns and another equality-set. Rows could fall by about half of the 1,443 permutation rows if the two Merkle children were hashed independently, which they are not: each layer depends on the previous layer. The 17 CRHs are a chain. Width does not parallelize a chain.

The only parallel work is inside one hash (the three state lanes), and `Pow5Chip` already lays those across columns of one row.

**Savings:** no critical-path rows off a chain of 17 dependent hashes.
**Soundness:** a new chip.
**Prove time:** more columns means more MSMs at the same K. A loss.
**Verdict:** do not widen.

## O5. Fuse swap + absorb into one custom gate

`cond_swap.rs:254` has an explicit TODO: "optimise shape of gate for Merkle path validation". A fused gate could absorb the 1-row swap into the 3-row absorb that already follows it. That saves **17 rows** (1,598 → 1,581).

The swap currently uses advice 4, which the absorb does not. A fused gate either takes a rotation (and must not exceed 3 advice queries, or blinding grows and usable rows shrink) or pulls the bit into a column the absorb already has. Either design is a new selector. At degree 6 it gets its own fixed column (`00-methodology.md`). Seventeen uses do not pay for a new relation when the rows do not cross K (`gates.md`).

**Savings:** 17 rows, K unchanged, +1 fixed column in the VK.
**Soundness:** the boolean check has to stay in the fused gate. Dropping it lets a non-bit `s` mix the children arbitrarily, which forges paths. The stock gate's three polynomials are the whole argument (`cond_swap.rs:265-282`).
**Verify gas:** unchanged.
**Prove time:** 17 fewer assigned rows. Not measurable against a 2,048-point MSM.
**Verdict:** leave the TODO. Revisit only if a future depth is within 17 rows of the usable ceiling. Depth 22 (`rows = 2,008`, slack 34) is the first point where 17 rows matter, and production is depth 17.

## O6. Replace the hash (different width, rate, or Poseidon2)

A rate-4 hash could absorb `(tag, layer, left, right)` in one permutation instead of two, cutting 17 permutations (about `17 × 40 = 680` critical-path rows) and landing near `1,598 - 680 = 918`, which fits K=10. Round counts for a width-5 Poseidon are not the `P128Pow5T3` numbers, so 918 is an order-of-magnitude sketch, not a layout.

That is a different hash. Every registered leaf and every anchor was built with `P128Pow5T3`, `ConstantLength`, and the tags in `01-inventory.md`. A new hash is a new tree, a new VK, and a new campaign. It is not an optimization of vk_id 3.

**Verify gas:** K=10 still has a 2,752−64 ≈ 2,688 byte proof (one fewer IPA round). `ceil(2688/1024) = 3`. Units stay 10.
**Prove time:** domain 1,024 is a real 2× MSM win, paid for by re-registering 98,841 notes.
**Verdict:** out of scope for this VK. Recorded so a later campaign does not rediscover it as a drop-in patch.

## O7. Collapse the six instances with an in-circuit hash

One instance instead of six changes units from `1+3+6 = 10` to `1+3+1 = 5`, about half of 34.9k gas, roughly 17k gas per claim. The claim transaction's non-verify overhead is about 158k (`onchain-verify-benchmark.md:26`, baseline 157,718). The user's bill moves by about 10%.

The circuit would hash the six values and expose the digest. That is another `ConstantLength` hash (a length-6 input is 3 permutations, about 121 critical-path rows: init 1 + 3×40). `1,598 + 121 = 1,719`, still K=11. The contract would have to build the same digest. Fiat-Shamir compression inside the proof is not available: instances are fixed before proving.

**Soundness:** a mismatched personalization between contract and circuit accepts proofs the contract thinks it checked, or rejects honest ones. This is the same class of bug as a wrong tag. The six raw limbs are the ABI `direct.rs:7-14` and `direct.rs:85-94` already test against forgeries (`onchain-verify-benchmark.md:55-64`).
**Verdict:** not for this VK. The gas win is real and small, and the ABI break is not.

## O8. Add an in-circuit denomination check

A 12-way membership test on `amount` (one-hot, or a degree-12 polynomial that does not fit under D=9) would reject a non-denomination leaf inside the proof. Registration only inserts denomination leaves (`notes.rs:1-4`), and `direct.rs:208-210` rejects other amounts before verify. An honest root has no such leaf. The check duplicates the contract and spends rows or a lookup.

**Verdict:** do not add it. Keep the contract check. It is cheap and it is what the payout uses to find the cap (`direct.rs:211-214`).

## O9. Move destination parsing into the circuit

`pack_destination` allows at most 62 bytes, split 31/31 so each limb is a canonical Pallas element (`no_ecdsa.rs:277-291`, mirrored in `direct.rs:63-77`). Bech32 decoding in-circuit is a byte circuit this VK does not have. The contract already binds the payee by passing the packed sender as the instance. Pulling the parser in would add rows, columns, and a second implementation of the pack.

**Verdict:** keep the pack on the contract side.

## O10. Lookup-based boolean or range checks

`techniques.md`: a lookup pays for itself when one table replaces many constraints, and the table height is charged in every piece that touches it. The only range-like constraint is `s(1-s)=0`, already inside the swap row. `amount` is a public input, not a range. There is no 8-bit decomposition.

**Verdict:** adding a lookup is a new column family and a strictly larger VK.

## O11. Defer the Merkle path to the K=15 key-ownership circuit

That is `deferral.md`: a second proof whose marginal cost has to beat the work it replaces. The Merkle path is the bulk of vk_id 3 and is already native Poseidon. Moving it next to a secp foreign-field chip replaces 1,598 Poseidon rows with the same Poseidon rows **plus** the secp rows. The four-design table is the measurement. See `06-k15-comparison.md`.

**Verdict:** the cheap claim stays in vk_id 3. Key ownership, if it is ever required, is a separate proof, not a bigger version of this one.

## Summary

| Id | K | Proof bytes | Verify units | Browser prove | Take |
|---|---|---|---|---|---|
| O1 local row cuts | 11 | 2,752 | 10 | ~3.3 s | no |
| O2 raise degree | 11 | +32 per degree | 10 | slightly slower | no |
| O3 drop one equality | 11 | 2,752 | 10 | same | no |
| O4 second lane | 11 | larger | 10 | slower | no |
| O5 fuse swap | 11 | 2,752 | 10 | same | no, unless depth is within 17 rows of 2,042 |
| O6 new hash, K=10 | 10 | ~2,688 | 10 | ~2× MSM, new campaign | not this VK |
| O7 one instance | 11 | 2,752 | 5 | +one hash | not this VK |
| O8–O10 extra checks | 11 | ≥2,752 | 10 | slower | no |
| O11 defer to K=15 | 15 for that proof | 5,088 | 12-class | 5–27 s | separate design, not a vk3 patch |
