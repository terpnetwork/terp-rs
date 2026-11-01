# 06 — Comparison with the K=15 four-design table

Source: `wip/k19-lab:tools/headstash/docs/k19-lab-benchmark.md`, section "FINAL: four-design comparison". Numbers below are copied from that table. They were measured on groot-wan (native median, and Chromium wasm for the K=15 rows). This review did not re-run them. The vk3 row of that table (1,598 rows, K=11, 0.036 s native, 3.3 s single-thread wasm, 2,752 B) is the same circuit this review accounts.

The lab's earlier secp chip was unsound (`FpChip::sub` range-checked an output it never tied to `a-b`). The final table is the sound replacement. The sound fixed-base raw-key claim is the "K=15, 24,584 rows" design the brief names.

## The four designs

| Design | Rows | K | Native prove | Wasm prove | Proof | Browser |
|---|---:|---:|---:|---|---:|---|
| 1. vk3 claim (this circuit) | 1,598 | 11 | 0.036 s | 3.3 s, 1 thread | 2,752 B | yes |
| 2. Sound raw-key, fixed-base window | 24,584 | 15 | 0.94 s | 27.4 / 7.3 / 5.3 s at 1/4/8 threads, ~0.86 GB | 5,088 B | yes, if the user exports the key |
| 3. Sound ECDSA, fixed-base `u1·G` plus GLV `u2·Q` | 681,032 | 20 | killed around a 25 GB footprint | not run | — | no |
| 4a. Two-step registration (fixed-base chip) | 24,706 | 15 | 2.49 s (host was swapping) | 38 / 9.6 / 6.4 s | 5,088 B | yes, once per key, same export |
| 4b. Two-step claim | 1,598 | 11 | 0.036 s | 3.3 s | 2,752 B | yes, this circuit unchanged |

Design 2 is about **15.4×** the rows of vk3 (`24,584 / 1,598`) and needs four more IPA rounds. Proof 5,088 bytes is `ceil(5088/1024) = 5`, so units are `1 + 5 + ceil(inst/32)`. With the same six limbs that is 12 units, against vk3's 10. The lab's own earlier estimate for a 4,224-byte raw-key proof was ~41.9k gas (12 units at a shorter transcript). Either way verify gas moves by one or two units. The thing that moves a lot is prove time, RAM, and the 48 MB proving key (the lab records a 48,262,377-byte cached artifact at K=15).

vk3 params in the on-chain store are 131,140 bytes (`onchain-verify-benchmark.md:10`). K=15 params in the lab's wasm table are multi-megabyte (the cached artifact is dominated by the proving key, not the 2 MB params). Circuit-store gas scales with that blob. vk3's store was 10.24M gas for 135,023 bytes (`onchain-verify-benchmark.md:49`). A K=15 key is a different gov proposal, not a tweak.

## What the row gap actually is

vk3 is 39 Poseidon permutations and 17 swaps (`02-accounting.md`). Design 2 keeps a depth-17 Poseidon path (the lab text says the raw-key circuit uses the vk3 tags and the same six public inputs) and adds a fixed-base scalar multiplication: 4 digit regions, 32 window lookups, 31 incomplete adds (`k19-lab-benchmark.md`, "Fixed-base raw-key claim"). Those foreign-field rows are the difference between 1,598 and 24,584. They buy `sk·G = pk` inside the proof.

vk3 does not have that statement. Registration shows a signature over `key_com` to the contract (`notes.rs` `verify_signer`). The claim proves knowledge of the `secret` inside that `key_com`. The anonymity set is "notes registered under this anchor and this denomination", not "the whole eligibility list". That is a product property of v1 (`headstash-denominations-spec.md`, and the lab's own threat-model section). Making the claim prove key ownership does not shrink vk3. It replaces the user's fast proof with design 2, or adds design 4a in front of an unchanged 4b.

## Degree and lookups, briefly

Design 2's soundness writeup in that file is the CRT story `techniques.md` and `sage.md` warn about: range-checked limbs, a per-limb carry identity, and a native-field check that is redundant once the integer identity holds. vk3 never enters that regime. Every constraint is a Pallas-native Poseidon round or a boolean swap. There is no quotient `u`, no limb width, and no lookup table to get wrong. The optimization pressure on vk3 is the IPA domain. The optimization pressure on design 2 was "make the foreign-field identity sound, then survive K=15".

## Implication for this review

The four-design table is the empirical version of `deferral.md`'s break-even. Moving the claim's hard part into a secp circuit costs ~15× rows, K+4, a multi-second to multi-ten-second browser prove, and a key-export UX. Keeping Poseidon in vk3 and secp out of it is the recommendation in `04-recommendations.md`. Design 4b is vk3 with the same 1,598 rows; the expensive step is registration, once per key, and it is optional relative to v1.
