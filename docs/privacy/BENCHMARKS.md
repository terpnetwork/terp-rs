# Privacy products — benchmarks

Sections are owned by workstream: [A] and [B]. Each workstream edits only its own section, insert-only.

## [A] Private auctions + DAO zk-voting — baseline (round 6, 2026-11-10)

**Machine:** groot-wan (Apple M1 Pro, 10 cores, 16 GiB). **Commit:** terp-rs `f7d42332` on `feat/mint-desk-v98`, plus the uncommitted working tree.
**Times:** UTC (box zone Africa/Dakar = UTC+0).
**Guest build (no Docker):** `ledger/bench/a/build-a.sh`, the native equivalent of B's `optimize-b.sh`:
- rustc 1.91.0, the optimizer image's RUSTFLAGS, per-package `--locked` build;
- then `wasm-opt -Os --enable-bulk-memory --enable-sign-ext --disable-simd`, with **local wasm-opt 132** (the image has 116).
- Artifacts are in `ledger/bench/a/`. The cosmwasm-vm instance load below is the capability check; `cosmwasm-check` was not run.

### Contract size (optimized wasm; chain limit 800 KiB = 819,200 B)

| contract | bytes | KiB | % of 800 KiB | sha256 | note |
|---|---:|---:|---:|---|---|
| svg_auction | 491,487 | 480.0 | 60% | `0954be9d6e32d8015ef97d829f28cb21fc8b426a3c645218617dd3a0d52eca6a` | round 6 (seller view, memo label, zap-1 payment gate), built 17:14Z. B's pre-edit build: 443,434 B. |
| cw_vote_ceremony | 513,431 | 501.4 | 63% | `606160abb61b853d09f9ae4fe5ca25f188a46e1bdc9acda70dbff6ce56608a2e` | source unchanged vs B's 518,266 B build. The −0.9% is the wasm-opt 132 vs 116 delta (control). |

- svg-auction grew by ~48 KB (+11%). Most of it is `zap1-verify` (zap-1 leaf hashing for the payment memo) plus the payment-client check.
- After the size measurement, two clippy-only edits landed (a type alias and an `#[allow]` on `QueryMsg`); codegen is unchanged. The rebuild
  to confirm was blocked: the disk is full (see PROGRESS [A]).
- **Rebuild 18:30Z (after the disk was freed):** svg_auction is **491,487 B (unchanged)**, sha256 `42356995a131f1bf32d4ff9f360f5756bc5eeb8281451b10a9ffae0735c1d7f6`.
  The hash differs from 17:14Z because of the clippy-only source edits. `cosmwasm-check` (fork `packages/check`, same capability list as [B]) passes: `ledger/bench/a/svg_auction.check.txt`.

### Proof generation / verification — valar `vote_proof` (ZKP #2)

- **Circuit:** voting-circuits 0.10.0 (git 68d69c6d, pinned by vote-sdk 5f23ec18) on the zakura Halo2 fork (`crates/voting-circuits-zakura`).
  **K = 11, 11 public inputs** (footer prefix `[0,11,2,0]`).
- **Harness:** `tests/suite/terp-orch/tests/privacy_bench.rs` (`#[ignore]`).
- **Run:** `BENCH_WASM_DIR=ledger/bench/a BENCH_OUT=… cargo test -p terp-orch --features privacy --release --test privacy_bench -- --ignored --nocapture`.
- **Method:** in-process, host native, release build.
  - Prove = `build_vote_proof_from_delegation`, which includes witness building and 16 ElGamal shares. Median of 5 ballots.
  - Off-chain verify = the host dispatch wasmvm runs (`AnyVerifyingKey::try_from(blob)` → `verify`). Median of 15.
  - In-VM = wall time of the whole `ZkCastVote` execute in cosmwasm-vm. This uses the cold path: no circuit cache, so the VK is deserialized from the blob each call.

| metric | value |
|---|---:|
| keygen: params + keygen_vk + keygen_pk + chain blob (ms, once) | 976.4 |
| prove `build_vote_proof_from_delegation` (ms, median of 5) | 772.3 |
| prove min / max (ms) | 760.9 / 774.5 |
| verify off-chain, host dispatch (`AnyVerifyingKey`, ms, median of 15) | 31.3 |
| proof bytes | 11008 |
| public inputs | 11 × 32 B = 352 B |
| ballot ciphertext bytes (16 ElGamal shares) | 1024 |
| stored circuit blob bytes (params ‖ cs ‖ vk ‖ footer) | 222503 |
| `proof_instance_verify` scheduled units | 23 |
| in-VM `ZkCastVote` wall time (ms, cold VK path, incl. RedPallas + storage) | ~33 |

### Gas (cosmwasm-vm metering on the optimized wasm above)

Same method and columns as [B]:
- `vm_gas` is CosmWasm gas;
- `sdk_vm` = vm_gas / 140,000;
- `sdk_kv_w` = Σ(2,000 + 30 × bytes) over keys written;
- `sdk_est` is a **lower bound** (no KV reads, callee execution or tx ante).

The contract runs in the VM; its queries are answered by a querier stub (voting power, `CircuitInfo` / `Circuit` blob, payment inclusion).
- auction harness: `svg-auction/src/private_tests/vm_gas.rs`;
- run: `BENCH_WASM_DIR=ledger/bench/a cargo test -p svg-auction --release --lib vm_gas -- --ignored --nocapture`.

| contract | call | vm_gas | sdk_vm | kv_w | kv_bytes | sdk_kv_w | sdk_est (lower bound) |
|---|---|---:|---:|---:|---:|---:|---:|
| svg-auction | instantiate | 172335691 | 1231 | 2 | 233 | 10990 | 12221 |
| svg-auction | ReceiveNft → CreatePrivateAuction | 467737218 | 3341 | 3 | 632 | 24960 | 28301 |
| svg-auction | PlacePrivateBid (2 FROST bind sigs, 144 B envelope) | 840970982 | 6007 | 3 | 1494 | 50820 | 56827 |
| svg-auction | FinalizePrivateAuction (6/6 FROST + PaymentInclusion query) | 1277012612 | 9122 | 3 | 2275 | 74250 | 83372 |
| svg-auction | DisclosePrivateRelease | 580606958 | 4148 | 1 | 1753 | 54590 | 58738 |
| svg-auction | ReclaimPrivateBid (5/6 FROST) | 806016377 | 5758 | 2 | 2172 | 69160 | 74918 |
| svg-auction | ReclaimPrivateBid (solo, after grace) | 696956748 | 4979 | 2 | 2172 | 69160 | 74139 |

| contract | call | vm_gas | sdk_vm | kv_w | kv_bytes | sdk_kv_w | sdk_est (lower bound) | VM wall (ms) |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| cw-vote-ceremony | instantiate (zk module, pinned circuit key) | 388019877 | 2772 | 3 | 620 | 24600 | 27372 | 0.9 |
| cw-vote-ceremony | ZkPropose | 808841268 | 5778 | 3 | 1169 | 41070 | 46848 | 0.5 |
| cw-vote-ceremony | ZkCastVote #1 (Halo2 k=11 verify + RedPallas) | 8350567697 | 59647 | 3 | 2544 | 82320 | 141967 | 33.2 |
| cw-vote-ceremony | ZkCastVote #2 (Halo2 k=11 verify + RedPallas) | 8350561027 | 59647 | 3 | 2544 | 82320 | 141967 | 34.7 |
| cw-vote-ceremony | ZkCastVote #3 (Halo2 k=11 verify + RedPallas) | 8350560567 | 59647 | 3 | 2544 | 82320 | 141967 | 37.5 |
| cw-vote-ceremony | ZkCastVote #4 (Halo2 k=11 verify + RedPallas) | 8350560797 | 59647 | 3 | 2544 | 82320 | 141967 | 33.0 |
| cw-vote-ceremony | ZkCastVote #5 (Halo2 k=11 verify + RedPallas) | 8350561487 | 59647 | 3 | 2544 | 82320 | 141967 | 32.9 |
| cw-vote-ceremony | ZkSubmitTally (FROST 2-of-3 committee) | 812072976 | 5801 | 2 | 1190 | 39700 | 45501 | 0.3 |
| cw-vote-ceremony | ZkExecute | 672710427 | 4806 | 1 | 862 | 27860 | 32666 | 0.2 |

Reading it:
- **Cast is dominated by the Halo2 verify host call.** The scheduled cost is `2,700M + 200M × units` with units = 1 + ⌈11,008/1024⌉ + 11 = 23, so 7.3G of the 8.35G VM gas.
  - ≈ **142k SDK gas per ballot**, lower bound.
  - The proof is 11,008 B, under vote-sdk's 15 KiB limit.
  - Optimization levers: proof size (units ∝ KiB), batching (the verify-batch host path), and a warm circuit cache (wall time only; gas is scheduled).
- **Auction:**
  - Bid ≈ 57k SDK gas: two FROST bind checks, plus a 144 B envelope and commitments stored.
  - Finalize ≈ 83k: 6/6 FROST, plus the payment-client query (the client's own execution, ~3k SDK gas per B's table, is not included).
  - Reclaim ≈ 74–75k. KV writes are ~90% of every auction row (sealed envelope and release ciphertext stored on chain; the test release carries a 400 B ciphertext). Storing only `blake2b(ovk_ciphertext)` and emitting the ciphertext as an event is the first size/gas lever.
- **Tally** (FROST 2-of-3 committee) ≈ 46k; **execute** ≈ 33k.

## [B] Light client, seams, committee — baseline (round 6, 2026-11-10)

**Machine:** groot-wan (groot.local): Apple M1 Pro, 10 cores, 16 GiB, macOS 27.2; Docker 29.8.0 (aarch64).
**Commit:** terp-rs `f7d42332` on `feat/mint-desk-v98`, plus uncommitted working tree (B's light-client crates are untracked; nothing committed).
**Guest build:** `terpnetwork/optimizer-arm64:0.17.0-rust1.91` (rustc 1.91.0, wasm-opt 116). Built per package (`cargo build --release --lib --target wasm32-unknown-unknown --locked -p <pkg>`), using the image's RUSTFLAGS (`-C link-arg=-s --allow-undefined -simd128 getrandom unsupported`), then `wasm-opt -Os --enable-bulk-memory --enable-sign-ext --disable-simd`. Script: `ledger/bench/optimize-b.sh`. Artifacts and `checksums.txt` are in `ledger/bench/artifacts/`.
- The docs pin rustc 1.97.1, but the `-rust1.97` image is not built locally (`make build-arm64-local`). These are 1.91 numbers.
- Per-package builds avoid feature unification. This differs from `bob .` in `terp-optimize.sh`, which builds the whole workspace.
**Host:** rustc 1.98.0-nightly (2026-06-06). cosmwasm fork `adace180b`.
**Times:** UTC (box zone Africa/Dakar = UTC+0).

### Contract size (optimized wasm; chain limit 800 KiB = 819,200 B)

| contract | owner | bytes | KiB | % of 800 KiB | limit | sha256 | note |
|---|---|---:|---:|---:|---|---|---|
| cw_ics08_wasm_zap1 | B | 300,397 | 293.4 | 37% | OK | `28c291e6c3fdc3eaf331f3df5096476553a15fa5dfddf2cc1bc4294827e3a02b` | existing zap-1 08-wasm client (clean, unchanged) |
| cw_lc_aggregator | B | 451,795 | 441.2 | 55% | OK | `5e5067207679c5a2f8194607113df125ec71b1744300cbe16dd96238110d7d4f` | light client: preconf aggregator |
| cw_lc_registry | B | 435,697 | 425.5 | 53% | OK | `4a6f1f9cc740b9f12d8ed07edfa7145b12166d32a35b5d6987473474ad9b0f0c` | light client: operator registry + k-of-n tip + hm-vext quorum |
| cw_lc_rotation | B | 486,284 | 474.9 | 59% | OK | `b2c90081e7dfe8a456754a0e05a4cca0cb4c74a41a3272c238e241c602f21095` | light client: dynamic committee rotation |
| cw_lc_zcash | B | 414,836 | 405.1 | 51% | OK | `e8559a28e06ac368ea80beb63edd860ebe56b84c20528feacd3426592ee6bd96` | light client: IBC v2 client + zap-1 payment inclusion (new) |
| cw_private_dex | B | 348,375 | 340.2 | 43% | OK | `eb1cc16d428099129244ffba8b32194294092910096495551b736d9e01562c99` | seams: private dex (terp-seams) |
| cw_vote_ceremony | A | 518,266 | 506.1 | 63% | OK | `066d5487b848f795d0b2bcb03ce6c0b3d0983ccbe286c3d0277c56725a0ac4d0` | built 2026-11-10T17:03Z; source unchanged since 16:57:53Z (current at build) |
| svg_auction | A | 443,434 | 433.0 | 54% | OK | `fe1f6ba7aa5ca182bdcd9710149aa98f6bbcc95705dfdb0ff87e9eb5c5ffaa45` | built 2026-11-10T17:03Z from source before A's 17:06Z edit; rebuild blocked (Cargo.lock stale, A in flight) |

`cosmwasm-check` passed for all of them, with these capabilities: iterator, staking, stargate, bulk_memory, ibc2, hash_blake, hash_poseidon, bn254, redpallas, cosmwasm_1_1..cosmwasm_3_0. Without `--available-capabilities`, the image's checker rejects `cosmwasm_3_0`.

### Gas (cosmwasm-vm metering on the optimized wasm above)

cw-multi-test has no gas meter, so the contract under test runs in `cosmwasm-vm` (the fork, `mock_instance_with_options`). Its dependencies run natively in a cw-multi-test `App`, which answers the VM's wasm queries.
- **Harness:** `contracts/light-clients/cw-lc-zcash/tests/vm_gas.rs` (`#[ignore]`).
- **Run:** `BENCH_WASM_DIR=… BENCH_OUT=… cargo test -p cw-lc-zcash --release --test vm_gas -- --ignored --nocapture`.

How to read the columns:
- `vm_gas` is CosmWasm gas: wasm execution plus host imports, including ed25519, blake3 and signature verification.
- `sdk_vm` = vm_gas / 140,000 (the wasmd multiplier).
- `sdk_kv_w` = Σ(2,000 + 30 × bytes) over keys written.
- `sdk_est` = sdk_vm + sdk_kv_w. It is a **lower bound**. It leaves out:
  - KV reads (1,000 + 3/byte each);
  - the callee's execution for cross-contract queries;
  - tx ante, signature and byte costs (~25–40k on a Cosmos SDK tx).
- Committee: 4 operators of weight 1 → 4 shares, threshold 3.

| contract | call | vm_gas | sdk_vm | kv_w | kv_bytes | sdk_kv_w | sdk_est (lower bound) |
|---|---|---:|---:|---:|---:|---:|---:|
| cw-lc-registry | instantiate | 208084403 | 1487 | 2 | 282 | 12460 | 13947 |
| cw-lc-registry | RegisterOperator | 291574161 | 2083 | 1 | 355 | 12650 | 14733 |
| cw-lc-registry | RegisterOperator | 291574161 | 2083 | 1 | 355 | 12650 | 14733 |
| cw-lc-registry | RegisterOperator | 291574161 | 2083 | 1 | 355 | 12650 | 14733 |
| cw-lc-registry | RegisterOperator | 291574161 | 2083 | 1 | 355 | 12650 | 14733 |
| cw-lc-registry | SetOperatorWeight | 345995552 | 2472 | 1 | 355 | 12650 | 15122 |
| cw-lc-registry | RegisterClient (4 attestors) | 426956160 | 3050 | 1 | 614 | 20420 | 23470 |
| cw-lc-registry | ReportNodeState #1 (ed25519) | 615493607 | 4397 | 1 | 463 | 15890 | 20287 |
| cw-lc-registry | ReportNodeState #2 (ed25519) | 615566747 | 4397 | 1 | 463 | 15890 | 20287 |
| cw-lc-registry | ReportNodeState #3 (ed25519) | 615639887 | 4398 | 1 | 463 | 15890 | 20288 |
| cw-lc-registry | ReportNodeState #4 (ed25519) | 615713027 | 4398 | 1 | 463 | 15890 | 20288 |
| cw-lc-registry | query ConsensusTip (4 reports) | 613001934 | 4379 | 0 | 0 | 0 | 4379 |
| cw-lc-registry | query NodeStates | 731823939 | 5228 | 0 | 0 | 0 | 5228 |
| cw-lc-rotation | instantiate | 330081888 | 2358 | 5 | 551 | 26530 | 28888 |
| cw-lc-rotation | Bootstrap (derive 4-op committee) | 1587166012 | 11337 | 3 | 762 | 28860 | 40197 |
| cw-lc-rotation | AckEpoch #1 | 322661643 | 2305 | 1 | 141 | 6230 | 8535 |
| cw-lc-rotation | AckEpoch #2 | 322876923 | 2307 | 1 | 141 | 6230 | 8537 |
| cw-lc-rotation | AckEpoch #3 | 322733403 | 2306 | 1 | 141 | 6230 | 8536 |
| cw-lc-rotation | AckEpoch #4 | 322805163 | 2306 | 1 | 141 | 6230 | 8536 |
| cw-lc-rotation | ActivateEpoch (FE01 FROST 3-of-4) | 769484429 | 5497 | 3 | 826 | 30780 | 36277 |
| cw-lc-rotation | Bootstrap (derive 4-op committee) | 1587166012 | 11337 | 3 | 762 | 28860 | 40197 |
| cw-lc-rotation | AckEpoch #1 | 774942048 | 5536 | 1 | 273 | 10190 | 15726 |
| cw-lc-rotation | AckEpoch #2 | 775157328 | 5537 | 1 | 273 | 10190 | 15727 |
| cw-lc-rotation | AckEpoch #3 | 775013808 | 5536 | 1 | 273 | 10190 | 15726 |
| cw-lc-rotation | AckEpoch #4 | 775085568 | 5537 | 1 | 273 | 10190 | 15727 |
| cw-lc-rotation | ActivateEpoch (TC01 3-of-4) | 1933364295 | 13810 | 3 | 964 | 34920 | 48730 |
| cw-lc-aggregator | instantiate | 278254560 | 1988 | 3 | 477 | 20310 | 22298 |
| cw-lc-aggregator | SubmitPreconf (FE01 FROST 3-of-4, +payment_root) | 627010803 | 4479 | 2 | 527 | 19810 | 24289 |
| cw-lc-aggregator | SubmitPreconf (FE01 FROST 3-of-4) | 634162726 | 4530 | 2 | 465 | 17950 | 22480 |
| cw-lc-aggregator | Confirm (walks 51..=53 vs registry tip) | 671567561 | 4797 | 1 | 452 | 15560 | 20357 |
| cw-lc-aggregator | query BestHeader | 321684518 | 2298 | 0 | 0 | 0 | 2298 |
| cw-lc-aggregator | SubmitPreconf (TC01 3-of-4, +payment_root) | 1347375159 | 9625 | 2 | 707 | 25210 | 34835 |
| cw-lc-aggregator | SubmitPreconf (TC01 3-of-4) | 1369431389 | 9782 | 2 | 645 | 23350 | 33132 |
| cw-lc-zcash | instantiate | 249876402 | 1785 | 2 | 406 | 16180 | 17965 |
| cw-lc-zcash | VerifyPaymentInclusion (zap-1, 1-step path) | 677702733 | 4841 | 2 | 733 | 25990 | 30831 |
| cw-lc-zcash | query PaymentInclusion | 426147475 | 3044 | 0 | 0 | 0 | 3044 |
| cw-lc-zcash | query LatestHeader (proxy) | 198138657 | 1416 | 0 | 0 | 0 | 1416 |
| cw-private-dex | instantiate | 172989810 | 1236 | 3 | 258 | 13740 | 14976 |
| cw-private-dex | CreatePool | 281659957 | 2012 | 2 | 336 | 14080 | 16092 |
| cw-private-dex | SettleSwap (mock_verify: host recompute + nullifier, no proof) | 481207752 | 3438 | 3 | 281 | 14430 | 17868 |
| cw-private-dex | query QuoteExactIn | 169097496 | 1208 | 0 | 0 | 0 | 1208 |

Notes:
- **FE01 vs TC01 verification:** SubmitPreconf costs ~627M VM gas with FE01 (one Schnorr check, 64-byte sig) and ~1.35G with TC01 at 3-of-4 (3 partial checks). TC01 grows linearly in t; FE01 stays flat. Use FE01 for the hot path.
- **payment_root:** carrying it in the header costs about +1.9k SDK gas of KV per preconf.
- **ZK verify (seams / private-dex):** the measured SettleSwap is the `mock_verify` path. The real `proof_instance_verify` host cost follows the VM gas table:
  - VM gas = 2,700M + 200M × units, where units = 1 + ⌈proof_bytes/1024⌉ + ⌈instance_bytes/32⌉;
  - e.g. a 1.6 KB proof with 10 field instances gives 13 units → 5.3G VM gas ≈ **37.9k SDK gas** on top of the mock-path numbers.

### Timing — FROST (FE01) and TC01 committee signing

- **Harness:** `crates/frost_escrow/tests/timing.rs` (`#[ignore]`).
- **Run:** `cargo test -p frost_escrow --release --test timing -- --ignored --nocapture`.
- **Method:** median of 15 runs, in-process (no network), host native (not wasm). Per-signer columns are the work one operator does.

| scheme | t-of-n | keygen/DKG total (µs) | round1 commit / signer (µs) | round2 sign / signer (µs) | aggregate / combine (µs) | verify (µs) | sig bytes |
|---|---|---:|---:|---:|---:|---:|---:|
| FE01 FROST-ed25519 | 2-of-3 | 1996 | 66.9 | 89.7 | 170.0 | 176.5 | 64 |
| TC01 k-of-n secp256k1 partials | 2-of-3 | 160 | – | 48.6 | 124.9 | 138.8 | 134 |
| FE01 FROST-ed25519 | 3-of-4 | 4614 | 66.4 | 106.4 | 185.6 | 175.7 | 64 |
| TC01 k-of-n secp256k1 partials | 3-of-4 | 213 | – | 48.1 | 186.3 | 203.0 | 200 |
| FE01 FROST-ed25519 | 4-of-5 | 8879 | 68.4 | 123.3 | 201.9 | 178.0 | 64 |
| TC01 k-of-n secp256k1 partials | 4-of-5 | 267 | – | 48.4 | 245.9 | 269.8 | 266 |
| FE01 FROST-ed25519 | 7-of-10 | 54947 | 68.5 | 171.6 | 248.5 | 178.0 | 64 |
| TC01 k-of-n secp256k1 partials | 7-of-10 | 535 | – | 48.4 | 437.2 | 485.3 | 464 |
| FE01 FROST-ed25519 | 11-of-16 | 205381 | 67.6 | 235.7 | 311.2 | 178.4 | 64 |
| TC01 k-of-n secp256k1 partials | 11-of-16 | 857 | – | 48.3 | 685.6 | 747.6 | 728 |
| FE01 FROST-ed25519 | 22-of-32 | 1537203 | 67.3 | 412.5 | 485.7 | 179.5 | 64 |
| TC01 k-of-n secp256k1 partials | 22-of-32 | 1711 | – | 48.6 | 1380.4 | 1540.0 | 1454 |

Reading it:
- **FE01 DKG is O(n²)** and is the only expensive step (22-of-32 ≈ 1.5 s in-process). It runs once per rotation. That fits under `dkg_timeout_blocks` = 200.
- **FE01 signing:** each operator spends ≲ 0.5 ms per header. Aggregation is ≲ 0.5 ms. Verification is flat at ~0.18 ms, and the signature stays 64 bytes.
- **TC01** verification and signature size grow linearly in t: 1,454 B and 1.5 ms at 22-of-32.

### Proof generation / on-chain proof verification

- **B has no circuits of its own.** Shielded circuits and vote circuits belong to A (valar vote-sdk). B's ZK path is `proof_instance_verify` in private-dex and seams; its cost formula is above.
- **zap-1 inclusion (no SNARK):** a Merkle path plus app_data_hash costs **~4.8k SDK gas** (VerifyPaymentInclusion row, 1-step path). Each further path level is one BLAKE2b node hash in wasm.
- **Reference: Himess/seam** (a Zcash light client on EVM; it uses no committee).
  - Its README reports ~576k EVM gas per header for execution, of which Equihash is 361k. A RISC Zero payment proof took 43.1 s (STARK) plus 34.2 s (Groth16 wrap) on an RTX 4090, and proving on a CPU is impractical.
  - Those are useful upper references for a trust-minimised mode where headers are checked in the contract rather than attested by the committee.
