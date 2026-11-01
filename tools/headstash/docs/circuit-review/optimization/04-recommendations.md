# 04 — Recommendations

Priority is "does this change K, columns, degree, or the instance ABI, and is the soundness story the one already deployed?" Row savings inside K=11 are priority none.

| Pri | Action | Rows / cols / degree / K | Verify gas | Prove time | Why this order |
|---|---|---|---|---|---|
| 1 | Ship vk_id 3 as it is. Keep `NOTE_K = 11`, `P128Pow5T3`, the four pinned tags, and the six raw instances. | 1,598 rows, 6 advice, D=6, K=11 | 10 units, ~34.9k gas, 2,752 B | ~3.3 s wasm, keygen included | The circuit is already on the right side of every cliff that moves gas or the browser. The 444 unused rows are the cheap kind of waste. |
| 2 | Keep registration signatures out of the circuit. | avoids the K=15/K=20 designs | stays at 10 units | stays at 3.3 s | `06-k15-comparison.md`. A sound fixed-base secp claim is 24,584 rows, K=15, 5,088 B, 5–27 s in the browser. |
| 3 | If a later campaign needs more than `2^22` leaves, plan K=12 explicitly. Depth 22 is the derived ceiling (`rows = 2,008`). Depth 23 is 2,090 and does not fit. | K=12 doubles params (~131 KB → ~262 KB) and the PK | still 10 units (proof grows ~64 B, still under 3,072) | about 2× wasm, including keygen | Capacity planning only. Production has 98,841 notes and fits depth 17 with 444 rows spare. Do not bump K "for headroom" on the current tree. |
| 4 | If verify gas ever dominates the claim tx, design a **new** VK whose single instance is a hash of the six limbs, and change `direct.rs` in the same patch. | +~121 rows, still K=11 | 5 units instead of 10, about −17k gas on a ~193k tx | one more hash inside 3.3 s | Real gas, small user-visible win, ABI break. Not a silent patch to `34d11437…`. |
| 5 | Do not implement O1–O5, O8–O10. | see `03-opportunities.md` | unchanged | unchanged or worse | They spend review time and a VK hash on rows the prover already ignores. |

No code change is recommended on this branch. The optimization result is the decision to leave the constraint system alone, with the accounting that justifies it.

## What would change the recommendation

- A measured browser profile that shows gate assignment, rather than MSM and keygen, as the majority of the 3.3 s. Nothing in the Halo2 cost model or the k19 numbers suggests that, and this review did not re-profile wasm.
- A product requirement to bind the secp256k1 key inside the claim. That is a different circuit (`06-k15-comparison.md`), not a tighter layout of this one.
- A tree deeper than 22 on this chip. Then K=12 is mandatory, and the work is a new VK plus new params, still not a rearrangement of the Poseidon rows.
