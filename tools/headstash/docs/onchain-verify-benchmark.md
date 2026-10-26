# On-chain verification benchmark (vk_id 3 note circuit)

Measurement only. The production design is unchanged: the headstash lane verifies proofs
off-chain. Nothing here is wired into cw-headstash.

## Setup

- Chain: local headstash e2e chain `headstash-mvk-e2e` (zk wasmvm build `/tmp/terpd-b2339a9c-build/terpd`), 127.0.0.1 only, Apple M1 Pro, 1 validator.
- Circuit: vk_id 3 note-membership blob, 135,023 bytes, sha256 `34d11437…5398` (= `NOTE_VK_HASH`).
  Split as 131,140 B params + 3,883 B vk body (cs + vk + 80-byte footer) and stored with
  `tx wasm store-full-circuit params.bin vk_body.bin --k 11 --circuit-type 0 --curve-type 0`.
  Genesis `circuit_upload_access` is `Everybody` on this chain, so no gov step or allowlist was needed.
  The chain assigned `zk_id = 1` / `param_id = 1` (the lane's "vk_id 3" is a lane-side id).
- Contract: `contracts/headstash/cw-zk-bench` (bench-only crate, not in the production build),
  wasm sha256 `89095b87…d96f`, 253,413 B, cosmwasm-check passes. Calls `deps.api.proof_instance_verify`
  and `deps.api.proof_instance_batch_verify`. `Baseline` takes the same calldata but does not verify.
- Proofs: `zk-headstash` example `bench_note_proofs` – 40 real 2,752-byte proofs from 40 distinct notes
  (distinct secrets, 1/2/5/10 TERP), instances = 6 limbs × 32 B (ctx, root, nullifier, dest lo, dest hi, amount).
- Driver: `tools/headstash/e2e/zk_bench.py`. Raw results: `onchain-verify-benchmark.json`.
  Two full runs gave identical gas to the unit.

## Results (SDK gas, `gas_used` per tx)

| proofs | baseline (no verify) | batch_verify | verify gas | verify gas / proof | total / proof |
|---:|---:|---:|---:|---:|---:|
| 1 (single `proof_instance_verify`) | 157,718 | 192,625 | 34,907 | 34,907 | 192,625 |
| 1 | 157,718 | 192,902 | 35,184 | 35,184 | 192,902 |
| 2 | 197,318 | 266,190 | 68,872 | 34,436 | 133,095 |
| 4 | 276,520 | 412,767 | 136,247 | 34,062 | 103,192 |
| 8 | 434,997 | 705,994 | 270,997 | 33,875 | 88,249 |
| 16 | 751,823 | 1,292,322 | 540,499 | 33,781 | 80,770 |
| 32 | 1,385,524 | 2,465,023 | 1,079,499 | 33,734 | 77,032 |

- Marginal cost per extra proof in a batch (16 → 32): **73,294 gas** (≈33.7k verify + ≈39.6k calldata/JSON/tx bytes).
- Fixed tx overhead ≈ 119.6k gas.
- **Max proofs per tx under a 50M block gas limit: ≈ 680** (extrapolated: (50,000,000 − 119,615) / 73,294).
  Not executed: the fixture has 40 proofs, and this local genesis has `max_gas = -1`. A 680-proof tx would be about 2.7 MB of base64 JSON;
  tx-size and mempool limits at that size were not tested.
- Batch verify is all-or-nothing and charged as the sum of the singles. The host schedules it before the backend runs: per proof,
  units = 1 + ceil(proof/1024) + ceil(instances/32) = 10, and cost = 2,700 µs + 10 × 200 µs = 4.7 ms-equivalent.
  That works out to ≈33.6k SDK gas, which matches the measurement.
- For comparison, the off-chain lane design costs about 311,740 gas per note in the stage/payout chunk.
  On-chain verify would add about 73k gas per proof (verify plus calldata) on top of the claim logic.

### Circuit store

| tx | gas_used | payload |
|---|---:|---:|
| store-full-circuit (params + vk body) | 10,242,506 | 135,023 B (131,140 + 3,883) |
| store bench wasm | 1,878,506 | 253,413 B |

Fee paid: 3,000,000 uterp (the gas limit was set to 60M, and only 10.24M was used). The deposit could not be read:
`q wasm circuit-deposit` returns "unknown query path" on this terpd build, and `circuit-info` is not a CLI command on it.

### Negatives (all txs succeed; the host returns `false`, and the gas charged is the same as a valid verify)

| case | result | gas_used |
|---|---|---:|
| forged proof (1 bit flipped) | false | 192,626 |
| wrong root (another note's root in limb 1) | false | 192,626 |
| wrong public input: 1 TERP note's amount limb set to 2 TERP (same-set swap) | false | 192,626 |
| wrong nullifier (limb 2 from another note) | false | 192,626 |
| valid proof against another note's instances | false | 192,626 |
| batch of 8 with one forged proof | false (whole batch) | 705,995 |
| unknown zk_id 999 | tx error code 29 (`CircuitInfo … No such code: 999`) | 191,274 |

Note: a first attempt used ctx as the "wrong public" limb. All 40 notes share ctx, so that case correctly returned `true`
and was replaced with the amount swap above.

### Lane e2e: same-set amount swap

`neg_same_set_amount_swap` was added to `tools/headstash/e2e/run.py`. It takes a proven 1 TERP note body and resubmits it as 2 TERP;
both amounts are in the campaign set. The lane rejects it with `proof did not verify`.
Full run: 35 passed, 0 failed (`run-1793054153`).

## Not measured

- Block execution wall time. The node log has minute resolution and block intervals stayed at about 1.04 s, so per-tx execution time was not observable.
  As a proxy, a native halo2 verify of one note proof takes about 1.6 ms on the M1 Pro (range 1.56–4.32 ms over 8 proofs, from `verify_secs`),
  against the 4.7 ms the gas schedule assumes.
- A real 50M-gas block, which would need a genesis change and more than 680 fixture proofs.
- Circuit deposit amount (query missing on this build).
- Gov / allowlist upload path (this genesis allows `Everybody`).
