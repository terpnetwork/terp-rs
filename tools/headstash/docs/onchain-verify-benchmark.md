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

## Verification time (wall clock)

Measured on 2026-10-26 between 19:07 and 19:19 groot-wan local time (UTC-4). This was the local headstash chain on 127.0.0.1:46667 with one validator, timeout_commit 1s. It used the cw-zk-bench contract, the vk3 membership circuit (zk_id 1 after re-store) and the real proofs in /tmp/zkbench/fx. Each tx went in its own block, with 5 reps per row; the table shows medians. Harness: `tools/headstash/e2e/zk_time.py`, summarised by `zk_time_summary.py`.

Load averages during the runs were 3.5 to 6.3 (1-min). The kms-dev mainnet node used about 75% of one core. Expect this to inflate the numbers by roughly 10-30%, and it is the main source of the jitter.

Methods:
- **(a) In the node**, using the stock libwasmvm (sha 034be1fd) with CometBFT and SDK telemetry turned on:
  - `wasm_contract_execute` is the delta of the SDK telemetry sum for each tx, so it is exact: one execute per window.
  - `finalize_block` is from `cometbft_abci_connection_method_timing_seconds{method="finalize_block"}`, sampled every 100 ms. Blocks whose sample was ambiguous were dropped, which leaves n of 2 to 5.
  - The JSON log only has 1-second timestamps, so it could not be used.
- **(b) Host function.** This is a scratch, local-only libwasmvm built from the exact pinned sources: terp-core ec436048 libwasmvm plus cosmwasm 416d8c60, using a `[patch]` to path. It adds `Instant` timing around `load_verifying_key_for_zkid`, prepare and `backend.verify`/`batch_verify`, plus a log line naming the cache tier that served the VK.
  - It lived in /tmp/zkbench/wvm and was never committed. The stock dylib was restored afterwards.
  - Gas was identical to stock, which shows the build is semantically the same.

| tx | gas | (a) wasm execute ms, stock | (a) finalize_block ms, stock | (a) verify ms = batch - baseline | (b) host verify ms | (b) host total ms | (b) ms / proof |
|---|---|---|---|---|---|---|---|
| baseline n=1..32 | 157.8k-1.39M | 0.4-2.0 | 2-8 | - | - | - | - |
| verify (single) | 192.7k | 13.0 (8-35) | 16.1 | ~12 | 23.4 | 23.8 | 23.4 |
| batch 1 | 193.0k | 22.5 | 24.9 | 21.8 | 22.2 | 23.8 | 22.2 |
| batch 2 | 266.2k | 16.3 | 27.6 | 15.5 | 28.8 | 29.1 | 14.4 |
| batch 4 | 412.8k | 32.6 | 35.2 | 32.2 | 44.2 | 45.1 | 11.1 |
| batch 8 | 706.1k | 67.5 | 64.6 | 65.5 | 78.7 | 87.4 | 9.8 |
| batch 16 | 1.29M | 124.1 | 126.3 | 123.5 | 131.3 | 132.9 | 8.2 |
| batch 32 | 2.47M | 249.4 | 252.3 | 248.6 | 257.9 | 261.2 | 8.1 |

With the instrumented dylib, wasm execute was 24.9 / 30.1 / 49.0 / 91.5 / 139.8 / 264.1 ms for batches 1-32. That is within noise of stock, plus up to about 6%.

Cold vs warm, and caching:
- The VK is cached between txs inside the node: pinned, then a memory LRU, then the fs cache (deserialize), then a split-file rebuild from the param+vk files.
- **First call after a node restart**, 3 runs on the instrumented build: tier=fs. The VK load took 20.3-23.3 ms and backend verify took 7.7-8.0 ms, for a host total of 28-31 ms. Wasm execute was 31-43 ms, and 46.7 ms on stock.
- **Every later call**: tier=memory, with VK load at 0.3-0.7 ms.
- Surprisingly, **warm single verify is slower in the backend: 21-23 ms against about 7.8 ms cold**. A batch costs roughly 14 ms fixed plus about 7.6-7.9 ms per proof. The fixed part looks tied to the memory-tier VK path, not to the proof itself. Stock single-verify samples are bimodal (8-13 ms or 22-35 ms), which fits the same pattern. This is worth looking into: the per-call overhead seems to be avoidable.
- The circuit is not pinned: no pinned-tier hits.

Comparison with the gas schedule: the schedule prices one proof at a 4.7 ms equivalent (about 33.7k gas). The measured marginal cost is about 7.9 ms per proof under load 4-6, and 22 ms for a warm single call. So at this load, verification is underpriced by about 1.7x per proof in batches and about 4.7x for a single call. The native example verifier measured about 1.6 ms per proof earlier. The gap of roughly 5x against in-node is not explained yet. Candidates are threading or rayon differences, the halo2 build, and machine load.

Not measured: per-proof time under a quiet machine; DeliverTx split by phase (ante versus msg), because the SDK has no per-msg timer and the log resolution is 1 s; and multi-validator or production hardware.
