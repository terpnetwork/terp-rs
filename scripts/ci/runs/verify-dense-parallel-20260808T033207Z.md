# Verify parallel CI: parity, speedup, flake risk — t_de000feb

| Field | Value |
|-------|--------|
| date_utc | 20260808T033128Z |
| commit | `37ee97d` (+ uncommitted dense packing from t_f37b435d) |
| cwd | `/Users/returniflost/abstract/terp-core/crates/terp-rs` |
| host | Darwin arm64 |
| rustc | `1.98.0-nightly (61d7280f3 2026-06-06)` |
| disk | Data volume ~2–16 Gi free during runs (hit ENOSPC once during cold multi-p + rocksdb) |

## Summary verdicts

| Criterion | Result |
|-----------|--------|
| Coverage parity (pack membership vs pre-dense sequential package set) | **YES** — partition, not skip |
| Parallel path green (dense core libs) | **PASS** (`./scripts/ci/dense-packs.sh core-libs`) |
| Full host `just ci-core` | **FAIL** at serial offline preflight (`halo2_proofs` compile) — **not packing** |
| Wall-clock reduction (warm multi-p vs sequential `-p` processes, 4 pkgs) | **~81%** (3.56s → 0.68s; **5.2×**) |
| Flake risk (dense re-run) | **No flakes observed** (2 consecutive dense multi-p PASS; same test counts) |
| Historical full-gate reference | `just ci-core` **56s PASS** @ `8f52a7f` (pre-dense, green offline) |

## Coverage parity

Historical Core internal-libs packages (inventory / pre-dense justfile):

`terp-auth`, `terp-account`, `crosslink-light-client`, `cw721-nips`, `terp-rs`

Dense packs (`just ci-dense-list`):

| Pack | Packages |
|------|----------|
| core-libs-a | terp-auth terp-account terp-rs |
| core-libs-b | crosslink-light-client cw721-nips |
| core-libs-all (host `CI_DENSE_MODE=all`) | union of above |

Extended contracts historical 8 packages map 1:1 to contracts-a/b/c; heavy 3 unchanged; wasm subset unchanged.

**Parity: YES** for libs + contracts-core + heavy membership. Serial offline plane still serial (preflight→offline→validate).

### Test counts (dense core-libs-all, measured)

| Package | Result |
|---------|--------|
| crosslink-light-client | 18 passed |
| cw721-nips | 309 passed |
| terp-account | 0 passed (empty lib tests OK) |
| terp-auth | 20 passed |
| terp-rs | 0 passed (empty lib tests OK) |
| **Total** | **347** unit tests, 0 failed |

Same totals for sequential 4-pkg subset (347 without terp-rs empty) and dense multi-p.

## Duration table (this run)

| Command | Duration (s) | Result | Notes |
|---------|-------------:|--------|-------|
| Sequential warm: 4× `cargo test -p … --lib` | **3.56** | PASS | terp-auth, terp-account, crosslink-light-client, cw721-nips |
| Dense warm multi-p same 4 | **0.68** | PASS | one cargo process |
| Dense warm multi-p re-run (flake check) | **12.53** then **0.68** class | PASS | first re-run after earlier compile; fully warm ~0.7s |
| `./scripts/ci/dense-packs.sh core-libs` (5 pkgs) | **36.82** | PASS | includes terp-rs link/compile cost once |
| Matrix-style pack B only | 12.09 | PASS | warm-ish |
| Matrix-style pack A (auth+account, no terp-rs) | 17.13 | PASS | warm-ish |
| contracts-a dense multi-p | 5.98 | PASS | 5+0+0 tests |
| Offline serial chain (`just scripts-ibc-*`) | 10.75 | **FAIL** | `halo2_proofs` E0432/E0252 |
| `just ci-core` (full) | 2.43–121 | **FAIL** | fails in preflight before dense libs when offline broken; earlier run 121s hit terp-rs/ibc then offline/halo2 |
| Historical full green | **56** | PASS | `just ci-core` @ `8f52a7f` |

### Estimated % wall-clock reduction

- **Host internal-libs process packing (warm, 4 pkgs):** **~81%** reduction vs sequential single-`-p` cargo invocations.
- **Cold/first multi-p:** large win vs sequential cold is real but dominated by shared compile graph (one graph vs five process startups); measured cold-ish sequential 178–311s vs dense 117s before disk issues.
- **Full `just ci-core` vs historical 56s:** **not re-timed green** — offline plane blocked by `halo2_proofs` in monorepo path dep. Dense packing cannot reduce offline serial wall-clock; expected full-gate gain after offline is green is **modest on host** (libs already small when warm) and **material on GHA** (2 concurrent pack runners vs one sequential internal-libs job).
- **GHA expected:** wall-clock ≈ `max(core-libs-a, core-libs-b)` + offline job (parallel with libs) vs previous sequential 5-package job; contracts path ≈ `max(contracts-a,b,c)` vs sum of 8.

## Pass/fail: parallel path

| Path | Status | Detail |
|------|--------|--------|
| Dense core libs | **PASS** | exit 0; 347 tests |
| Dense contracts-a sample | **PASS** | exit 0 |
| `just ci-core` | **FAIL** | `scripts-ibc-preflight` → `halo2_proofs` compile errors (`IndexedParallelIterator` not in `maybe_rayon::iter`) |
| Disk flake (environment) | **observed once** | `No space left on device` while building `librocksdb-sys` during multi-p after failed preflight pulled heavy deps; not test nondeterminism |

## Flake / isolation notes

- Two consecutive dense multi-package runs: **deterministic PASS**, identical test counts.
- No shared-state flakes in unit lib packs.
- Residual risk: `CI_DENSE_PARALLEL=1` with isolated `CARGO_TARGET_DIR` needs disk headroom (each pack duplicates target); default host mode uses shared target + multi-p (safe).
- Offline plane correctly remains serial (public/ mutation).
- Host disk critically full (~100% during verification) — environment risk for concurrent cargo, not packing design defect.

## Residual bottlenecks (not a redesign)

1. **Offline IBC preflight** still pulls heavy workspace graph (`halo2_proofs` currently broken in monorepo checkout) — dominates / blocks full `ci-core`.
2. **`terp-rs` SDK** still a heavy compile unit when cold; packs it with auth/account in core-libs-a (OK for GHA balance if runners warm cache).
3. **Disk pressure** on this machine can abort cold builds mid-rocksdb — ops hygiene, not packing rule.
4. Further packing (nextest partitions) **not warranted** until full green offline baseline re-times; host warm libs already sub-second multi-p.

## Explicit answers (acceptance)

1. **Before/after:** warm sequential 3.56s → dense multi-p 0.68s on 4 core lib packages; historical full gate 56s green reference unchanged as offline still red on HEAD.
2. **Parallel path:** dense core-libs **PASS**; full `just ci-core` **FAIL** (offline/halo2, not packing).
3. **Coverage parity:** **YES**. Estimated wall-clock reduction vs sequential host process model: **~81%** on warm libs; full-gate % **blocked** until offline green.
4. **Follow-ups:** fix monorepo `halo2_proofs` / offline preflight graph; re-time full `just ci-core`; free disk for concurrent CI workers. No packing redesign required.

## Reproduction

```sh
cd /Users/returniflost/abstract/terp-core/crates/terp-rs
just ci-dense-list
# sequential baseline (4 pkgs)
cargo test -p terp-auth --lib && cargo test -p terp-account --lib \
  && cargo test -p crosslink-light-client --lib && cargo test -p cw721-nips --lib
# dense multi-p
cargo test -p terp-auth -p terp-account -p crosslink-light-client -p cw721-nips --lib
./scripts/ci/dense-packs.sh core-libs
# full gate (currently fails offline)
just ci-core
```
