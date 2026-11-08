# 02 — Row, column, and lookup accounting

## Why the total is 1,598 and not the sum of region heights

`SimpleFloorPlanner` places each region at the first row where **every column that region touches** is free (`halo2_proofs/src/circuit/floor_planner/single_pass.rs:95-106`). The rectangle is `row_count` tall on all of those columns, including rows where a particular column is not written. Regions on disjoint columns share rows.

Summing region heights gives 1,639. That over-counts the five witness loads (they share row 0), the tag regions on advice 5, and the layer constants on advice 4. The height of the circuit is the maximum column end. Advice 0, 1, and 2 are in every Poseidon region and in every swap, so they are the critical path.

Placement, in `synthesize` order (`note_membership.rs:141-171`), tracking only the columns that move the maximum:

| Step | Regions | Rows added on advice 0 |
|---|---|---:|
| `load` secret. The ctx, amount, dest_lo, and dest_hi loads use other columns and sit on row 0 beside it. | 5 regions, 1 row of height | 1 |
| `key_com`, `ConstantLength<2>` | init 1 + add 3 + perm 37 | 41 |
| `nullifier`, `ConstantLength<3>` | init 1 + 2×(add 3 + perm 37) | 81 |
| `leaf`, `ConstantLength<3>` | same | 81 |
| each of 17 layers | swap 1, then a `ConstantLength<4>` hash of 81. The CRH tag is advice 5. The layer index is advice 4. Neither extends advice 0. | 82 |
| **Total** | | **1 + 41 + 81 + 81 + 17×82 = 1,598** |

Closed form for this same chip, floor planner, and prefix (key commitment, nullifier, leaf), at depth `d ≥ 1`:

```text
rows(d) = 204 + 82·d
```

Check: `204 + 82·17 = 204 + 1,394 = 1,598`.

The 204 is `1 + 41 + 81 + 81`. The 82 is one swap plus one 2-permutation hash. Permutations are `5 + 2d` (39 at depth 17). Selector-active rows are 1,460; the other 138 rows are the secret load, 20 initial states, 39 permutation output rows, and 78 non-selector rows inside the absorb regions (`39 × 2`). `1,460 + 138 = 1,598`.

Column ends from the same placement:

| Column | End row | Notes |
|---|---:|---|
| Advice 0, 1, 2 | 1,598 | Critical path |
| Advice 3, `rc_a[0..3]`, `rc_b[0..3]`, `s_full`, `s_partial` | 1,598 | Present in every permutation rectangle, which ends the circuit |
| `s_pad_and_add` | 1,561 | Last absorb ends before the last 37-row permutation |
| Advice 4 | 1,518 | Last layer constant. The final hash does not use this column. |
| `q_swap` | 1,517 | Last swap, then the last hash |
| Advice 5 | 20 | Three prefix tags + 17 CRH tags. Idle for the other 1,578 rows. |

Constants on `rc_b[0]`: 97 cells (`01-inventory.md`). They are written into the same column the permutation already occupies, and 97 is far below 1,598, so they do not stick out past the critical path.

## Lookups

Zero lookup arguments, zero lookup columns, zero table rows. The boolean check does not need one. A lookup argument in this backend adds permuted columns and a product column (`halo2_proofs/src/plonk/lookup.rs`) and would be paid on the full domain. Seventeen booleans do not amortize that.

## Blinding and the K=11 headroom

Advice query count is the number of distinct rotations of each advice column (`circuit.rs:1322-1334`). The pad-and-add gate reads advice 0, 1, and 2 at `prev`, `cur`, and `next` (`pow5.rs:161-169`). That is 3. No gate uses a wider rotation.

```text
blinding_factors = max(3, max_advice_queries) + 1 + 1 = 5
```

(`circuit.rs:1679-1703`: the inner `max(3, ·)` covers the permutation polys, then one multiopen evaluation, then one spare).

Usable rows (`dev.rs:529`, and the same formula in `plonk/keygen.rs` when the prover poisons the tail):

```text
usable = 2^K - (blinding_factors + 1) = 2^K - 6
```

| K | Domain | Usable rows | 1,598 fits? |
|---:|---:|---:|---|
| 10 | 1,024 | 1,018 | no, short by 580 |
| 11 | 2,048 | 2,042 | yes, **444 rows free** |
| 12 | 4,096 | 4,090 | yes |

`minimum_rows` (`circuit.rs:1708-1716`) is `5 + 1 + 1 + 1 = 8`. That is the smallest legal domain, not a second reservation on top of the 6-row tail. The tail is the 5 blinding rows plus the `l_last` row. Rows `[2042, 2048)` are unusable. Rows `[1598, 2042)` are usable and empty.

`note_k_is_minimal` (`note_membership.rs:494-499`, `:560-562`) is the empirical form of the K=10 rejection: `MockProver` returns an error when a region is placed past `usable`. This review did not re-run it.

Headroom in hashes: 444 rows is `444 / 82 ≈ 5.4` extra Merkle layers before K=12. `rows(22) = 204 + 1,804 = 2,008` (34 rows of slack). `rows(23) = 2,090` (48 rows over 2,042). Depth 22 is the derived K=11 ceiling for this chip. Depth 17 is what 98,841 notes require (`2^16 = 65,536` is too small; `note_membership.rs:5-6`, `:40`). The earlier estimate in `shared-registration-tree-design.md:109-121` (depth 20–21 as the K=11 edge, ~1,530 rows at depth 17) used "~37–40 rows per permutation" and under-counted the critical path. The 1,598 figure is the one that matches both this placement and the k19 lab table.

## Degree, quotient, permutation

`D = 6`.

```text
quotient_poly_degree = D - 1 = 5          # domain.rs:95
extended_k = 14                             # 2048·5 = 10240, and 2^14 = 16384
permutation chunk = D - 2 = 4              # permutation.rs:18-20
equality columns = 6 advice + 3 rc_b + 1 instance = 10
permutation sets = ceil(10 / 4) = 3
```

`D = 9` still has `extended_k = 14` (`2048·8 = 16384`) but commits 8 quotient polynomials instead of 5. `D = 5` drops the FFT to `extended_k = 13` and raises the set count to `ceil(10/3) = 4`. The current degree is the low end of the `extended_k = 14` band, not an under-spent FRI bracket. Details in `03-opportunities.md`.

## What 1,598 does not include

- The 6 unusable rows.
- The 444 empty usable rows. They are still in every MSM.
- Grand-product columns (3), quotient pieces (5), and selector-combination columns (3, derived). Those are prover columns or fixed columns, not extra circuit rows.
- The sha256 eligibility path and the ECDSA/ed25519 registration check. Both are contract-side (`notes.rs:427`, `direct.rs` does not verify a signature).
