# Headstash limits benchmark (cw_headstash 1e7092d2, terpd v6.4.0)

Local single-node chain (127.0.0.1), block max_gas 50,000,000, max_bytes 16,777,216, timeout_commit 500 ms, Apple M1 Pro (shared host, load ~20).
Real proofs (vk_id 3, K=11). M = measured, X = extrapolated. Raw data: `limits-benchmark-full.json` (fill to 127,850 leaves / 1,735 anchors)
and `limits-benchmark-12k.json` (11,776 leaves / 90 anchors, block packing, multi-claim, prove). Scripts: `e2e/bench_limits.py`, `e2e/bench_probe_full.py`, `e2e/bench_wasm_prove.mjs`.
Mainnet consensus params (read-only, `/consensus_params` on a synced morocco-1 node at 127.0.0.1, height 24,123,126, 2026-10-27; the chain-registry public RPCs were unreachable): **max_bytes 16,777,216, max_gas 50,000,000** — identical to the local bench chain; bytes are not binding (claim block ≤ 0.69 MB).

## After: ANCHORS map (cw_headstash 43a8e5d8, commit e976867)
`Map<seq, Anchor>` + `Map<root, seq>` + meta `Item`; paginated `registration_anchors`. Bench: `BENCH_HOME=/tmp/headstash-bench-anchorfix FILL_TARGET=16384 ANCHOR_TARGET=2100 BENCH_STOP_AFTER=claims_per_block e2e/bench_limits.py` (16,359 leaves / 2,103 anchors). Raw data: `limits-benchmark-anchormap.json`.
| metric | before (1e7092d2, Item<Vec>) | after (43a8e5d8, map) |
|---|---|---|
| accept_anchor gas | 149k at 1 → 4.78M at 1,735; **fails at 1,735** (131,090 B > 128 KiB value) | 149,467 at 1 → 149,728 at 2,103 (flat) M |
| ClaimNote gas | 262k + 256 × anchors (≈706k at 1,735; >400k past ~540) | 263.5k at 10, 263.6k at 1,003, 263,613 latest / **263,604 against the first anchor at 2,103** (flat) M |
| claim gas_wanted (dapp/relay/e2e) | 400k | 300k |
| claims per block @ 300k gas_wanted | 125 @ 400k | 50M / 300k = **166** X; measured: 250/250 fee-granted claims at ~303k gas_wanted (1.15 × used), 113 in one block (34.3M wanted, 29.8M used, FinalizeBlock 976 ms) M |
| registration | — | 32-note chunk 10.27M (first chunk 10.67M); 4 chunks/tx 41.3M ok, 5 rejected (> block gas) M |
| fee-grant total, 98,841 notes @ 0.05 uthiol/gas | ~2,995 THIOL (≈4,180 at 1,000 anchors) + accept_anchor Σ ≈ 210 THIOL | registration 321k + claim 264k = 585k gas/note → **~2,891 THIOL**, independent of anchor count (≤ 3,336 THIOL if every grant is spent to its cap: 12M/32 + 300k); accept_anchor 149.7k × 1,735 ≈ 13 THIOL X |

## 1. Claims per tx / block (M, 90 anchors)
| metric | value |
|---|---|
| gas per ClaimNote (single-msg tx) | 285,151 median (264.6k at 10 anchors) M |
| claim tx bytes | 4,208 M |
| multi-claim tx: 10 / 50 / 172 claims | 2.21M / 10.75M / 36.79M gas used (≈214k/claim), 686 KB, FinalizeBlock 1.50 s M |
| tx with gas_wanted 50,000,001 | rejected (code 41) M |
| 250 single-claim txs @ gas_wanted 400k | 125 per block (gas_wanted sum = 50M binds), 35.6M used, 526 KB, FinalizeBlock 1.07 s M |
| max per block, tight gas_wanted (~290k) | ~172 X |
Binding limit: **block gas (gas_wanted sum)**. Time: 1.5 s FinalizeBlock for a full claim block < 4.2 s. Bytes: 0.69 MB << max_bytes.

## 2. Claim gas vs anchor count (M up to 388; claim proofs used grants capped at 400k gas)
| anchors | leaves | claim gas |
|---|---|---|
| 10 | 2,409 | 264,600 M |
| 49 | 7,260 | 274,520 M |
| 89 | 12,160 | 284,730 M |
| 109 | 14,720 | 289,850 M |
| 169 | 22,400 | 305,320 M |
| 228 | 30,080 | 320,440 M |
| 388 | 50,560 | 361,470 M |
| 588 | 76,160 | >400,158 (out of gas at the 400k grant cap) M |
| 1,000 | — | ~521,000 X |
| 4,400 | — | ~1.39M X — **unreachable**, see below |
Fit: claim ≈ 262k + 256 gas × anchors (linear, R² ≈ 1). Junk-proof claim at 1,735 anchors: 161.6k gas before verify fails (latest/oldest/unknown root identical → the cost is loading/deserialising ANCHORS, not a scan position).
accept_anchor gas: 149k at 1 anchor → 4.78M at 1,735 (≈2.67k gas/anchor) M.
**Hard limit (M):** at 1,735 anchors accept_anchor fails: `Value too big. Tried to write 131090 bytes to storage, limit is 131072` (~75.5 B/anchor). With a 3,430-block interval that is ~1,735 × 4 h ≈ 290 days, not 2 years; after that no new roots can be accepted. `registration_tree` query costs ~1.0M gas at 1,735 anchors (120k at 90).

## 3. Registration gas per chunk (M)
| notes/chunk | first chunk gas | continuing chunk gas | gas/note (cont.) |
|---|---|---|---|
| 1 | 1,016,759 | 633,605 | 633,605 |
| 8 | 3,194,814 | 2,803,959 | 350,495 |
| 16 | 5,687,732 | 5,291,742 | 330,734 |
| 32 | 10,669,175 | 10,268,846 | 320,901 |
Per tx: 1/2/3/4 × 32-note chunks = 10.6/20.5/30.7/41.3M gas; 5 chunks runs out of gas at 50M. Per block: 4 chunks = 128 notes, ~41M gas, 178 KB, FinalizeBlock 1.21–1.40 s M.

## 4. Tree fill curve (M to 127,850 leaves)
| leaves | 128-note block gas | FinalizeBlock | path query median | claim gas (anchors) |
|---|---|---|---|---|
| 2.5k | 40.94M | 1.24 s | 8.3 ms | 264.6k (10) |
| 22.4k | ~41.0M | ~1.23 s | 5.5 ms | 305.3k (169) |
| 36.7k | 41.33M | 1.23 s | — | — |
| 50.6k | ~41.0M | — | 6.6 ms | 361.5k (388) |
| 71.4k | 40.96M | 1.27 s | 7.3 ms | >400k (588) |
| 106.1k | 41.34M | 1.40 s | 8.7 ms | >400k (787) |
| 127.9k | single-note 629–645k | — | 9.6 ms | >400k (1,030) |
Registration gas is flat in fill level (depth-17 frontier with stored NODES: O(depth) per leaf). Path query latency 5–10 ms flat. Extrapolation to 131,072: registration & path unchanged (fixed depth); claim gas depends only on anchor count (formula above), not on leaves. Registration_path query gas ~199k at 11.8k leaves.

## 5. Prove time (M)
| prover | time per claim |
|---|---|
| native (CLI, warm, serial) | 0.13–0.19 s (median 0.167 s); 0.39 s median with 4 parallel provers under load |
| wasm in node v26 (single-thread, browser prover) | 3.28–3.39 s, proof 2,752 B |
| relay (register endpoint) | 1.5 s serial; up to ~20 s queued at 20 concurrent |

## Derived (X)
- Full blocks: 20,571 blocks/day (4.2 s) × ~172 claims = **~3.5M claims/day**; 98,841 notes drain in **~575 blocks ≈ 40 min** of full blocks (at ≤100 anchors). At 1,000 anchors (~521k gas) ~96 claims/block → ~1,030 blocks ≈ 72 min.
- Registration of 98,841 notes: 3,089 chunks × 10.27M = 31.7G gas ≈ 635 full blocks (~44 min).
- Fee grants at 0.05 uthiol/gas: registration 321k + claim 285k = 606k gas/note → 30,300 uthiol; ×98,841 = **~2,995 THIOL** (≈4,180 THIOL if claims average 1,000 anchors). Plus accept_anchor calls (~2.7k × n gas each: Σ to 1,735 ≈ 4.2G gas ≈ 210 THIOL).
- Block capacity share: registration + claims ≈ 60G gas = ~1,200 full blocks = 0.008 % of 2 years (15.0M blocks); bursts saturate blocks for ~1–1.5 h total.

## Bottlenecks and fix proposals (1–3 implemented in e976867 + follow-up; see "After" above)
1. **(fixed: map + paged query)** **ANCHORS `Item<Vec>` (binding):** 128 KiB storage value limit at 1,735 anchors bricks accept_anchor (~290 days at the 3,430-block interval); claim gas grows 256/anchor and query gas grows unbounded. Fix: `Map<u64, Anchor>` + `Map<root, u64>` (O(1) root check), keep a bounded ring buffer (e.g. last N roots) or only the latest root plus roots used by unclaimed proofs; or raise the interval (≥ 3× → still < 2y). Expected claim gas then ~262k flat.
2. Block gas is the claim limit (not time/bytes): set claim gas_wanted ≈ 300k (not 400k) in the dapp/relay to fit ~165 claims/block instead of 125.
3. Registration: always use 32-note chunks (1-note chunks cost 2× per note); max 4 chunks/tx.
4. Browser prove 3.3 s is fine; relay serial at 1.5 s/request queues under concurrency — add workers if bursty.
