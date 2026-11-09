# CI inventory: jobs, suites, sequential bottlenecks

**Task:** `t_3c311b49` (board `terp-core-ci`)  
**Scope:** Observation only — no workflow/config changes.  
**Primary subject:** `crates/terp-rs` tiered CI (board default).  
**Secondary:** Go chain `terp-core` worktree workflows (for parallelization parent epic).  
**Date:** 2026-08-07  
**Sibling:** `t_9212e352` baseline wall-clock; `t_f37b435d` dense parallel packing design.

---

## 1. Layout and entrypoints

| Layer | Path (from monorepo `terp-core/`) | Role |
|-------|-------------------------------------|------|
| terp-rs workflows | `crates/terp-rs/.github/workflows/ci-*.yml` | Authoritative Rust PR gates (standalone repo root on GHA) |
| Local just parity | `crates/terp-rs/justfile` (`ci-core`, `ci-extended`, `ci-heavy`) | Same commands CI runs |
| Local act | `crates/terp-rs/scripts/ci/` | YAML fidelity + optional full act; host is merge-authoritative |
| Docs | `crates/terp-rs/docs/ci.md`, `scripts/ci/README.md` | Tier model |
| Go chain CI | `.github/workflows/*` at monorepo root (this worktree) | Docker image + interchaintest matrix |

`Cargo.toml` path deps (`../cosmwasm`, …) mean **local act must bind monorepo root**; GHA checkout of `permissionlessweb/terp-rs` is crate-root-only.

---

## 2. terp-rs workflow inventory

### 2.1 Active tiers

| Tier | Workflow file | Triggers | Jobs (graph) | Parallelism today |
|------|---------------|----------|--------------|-------------------|
| **0 Core** | `ci-core.yml` | PR, push `main`/`master`/`feat/**`, dispatch | `terp-scripts-offline` ∥ `internal-libs` → `core-gate` | **2 jobs parallel**, then gate. *Inside* each job: **sequential** steps/packages |
| **1 Extended** | `ci-extended.yml` | Path filters (contracts/crates/tests/…), label, dispatch | `should-run` → `contracts-core` ∥ `scripts-integration`; optional `contracts-heavy` **needs** `contracts-core` | Job fan-out after decide; **sequential** `for p in … cargo test` inside contract jobs |
| **2 Heavy** | `ci-heavy.yml` | Dispatch, weekly Mon 06:00 UTC, label `ci-heavy` (same-repo only) | `authorize` → `workspace-check` ∥ `docker-harness` | Two heavy jobs parallel after auth |

Concurrency: each workflow uses `cancel-in-progress: true` per ref.

### 2.2 Deprecated stubs

| Workflow | Behavior |
|----------|----------|
| `basic.yml` | Always succeeds with warning → use CI Core |
| `e2e.yml` | Fail-closed redirect to CI Heavy / `just scripts-ibc-harness` |
| `release.yml` | Separate release path (not part of test packing) |

### 2.3 Job → test suite map (terp-rs)

| Job | Commands / suites | Docker | Timeout |
|-----|-------------------|--------|---------|
| **terp-scripts-offline** | `just scripts-ibc-preflight offline` → `scripts-ibc-offline` (`cargo test -p terp-scripts --lib --test ibc_unit --test ibc_golden` + `terp-ibc rebuild-from-public`) → `scripts-ibc-validate` + jq assert on RunReport | No | 45m |
| **internal-libs** | Sequential: `cargo test -p {terp-auth,terp-account,crosslink-light-client,cw721-nips,terp-rs} --lib` | No | 45m |
| **core-gate** | Echo only (`needs` both) | No | default |
| **contracts-core** | Sequential lib tests: `terp-ed25519`, `terp-passkey`, `terp-vsck`, `terp-eth`, `terp-bls`, `terp-recovery`, `terp-authenticator-suite`, `cw-ics08-wasm-crosslink`; then sequential wasm `cargo check` subset | No | 60m |
| **contracts-heavy** | Sequential: `terp-zkjwt` (label/dispatch only) | No | 90m |
| **scripts-integration** | `cargo test -p terp-scripts --lib --tests` (**non-ignored only**) | No | 45m |
| **workspace-check** | `cargo check --workspace --tests` | No | 90m |
| **docker-harness** | `just scripts-ibc-harness` → `cargo test -p terp-scripts --test ibc_multihop_harness -- --ignored` | **Yes** | 120m |

### 2.4 justfile host recipes (parity)

```text
ci-core:     preflight → offline (lib+ibc_unit+ibc_golden+rebuild) → validate
             then sequential cargo test --lib for 5 packages
ci-extended: sequential for 8 contract pkgs; then cargo test -p terp-scripts --lib --tests
ci-heavy:    cargo check --workspace --tests; scripts-ibc-harness
```

Act track: `act-host-core` (authoritative), `act-wire` (list only), `act-core`/`act-extended` (optional, Docker Desktop fragile). `CARGO_BUILD_JOBS` default **2** under act.

---

## 3. Test suite inventory (terp-scripts + libs)

### 3.1 Integration test binaries (`tests/tests/`)

| File | ~LOC | `#\[test\]` count (rough) | Default CI | Notes |
|------|------|---------------------------|------------|-------|
| `ibc_unit.rs` | 163 | 8 | Core offline | Pure/offline IBC pipeline |
| `ibc_golden.rs` | 131 | 4 | Core offline | Golden tables under `public/` |
| `ibc_multihop_harness.rs` | 907 | 4 | Heavy only (`#[ignore]`) | Docker + Terp image; multi-hop authenticity |
| `crosslink_light_client.rs` | 1177 | 2 | Not in core; one `#[ignore]` Docker | Dense integration surface |
| `nostr_orch_suite.rs` | 371 | 7 | Mostly `#[ignore]` Docker/relay | Sidecar/orchestration |
| `no_rick.rs` | 0 | 0 | — | Empty placeholder |

Lib crate unit tests (core job): ~12 in `terp-auth`, fewer in others (counts approximate via attribute grep).

### 3.2 Explicit `#[ignore]` (must stay out of default PR path)

| Location | Reason |
|----------|--------|
| `tests/tests/ibc_multihop_harness.rs` | Docker + Terp image; live multi-hop |
| `tests/tests/crosslink_light_client.rs` | Docker Terp chain |
| `tests/tests/nostr_orch_suite.rs` (×2) | Docker / local chain / relay |
| `crates/wasm-bindgen/snap-n-pull-rs/...` | Live endpoints |

### 3.3 Parallelism knobs present / absent

| Knob | Status |
|------|--------|
| GHA job fan-out (core offline ∥ libs; extended contracts ∥ scripts) | **In use** |
| Go interchaintest `strategy.matrix.test` | **In use** (terp-core) |
| `cargo-nextest` / partitions / profiles | **Not configured** (no `nextest.toml`) |
| Single `cargo test -p a -p b -p c` multi-package | **Not used** — loops of single `-p` |
| Matrix shards for Rust packages | **Not used** |
| `CARGO_BUILD_JOBS` | Cap **2** under act only |
| Dense pack / load-balanced shards | **Not implemented** (parent epic) |

---

## 4. Sequential bottlenecks (ordered by impact)

### B1 — Host `just ci-core` / GHA core jobs: sequential `cargo test -p` packages

**Where:** `justfile` L275–279; `ci-core.yml` internal-libs step; mirror in extended contract loops.

**Why it hurts:** Each `cargo test -p X` pays process + resolution overhead; rustc work can overlap more with one multi-package invocation or nextest. Cold compile still dominates first run; sequential package loops serialize link/test phases unnecessarily after shared deps are warm.

**Safe to parallelize?** **Yes at package level** for pure `--lib` crates with no shared mutable FS (auth, account, light-client, nips, sdk). Prefer: one job matrix per package **or** `cargo nextest run -p …` / `cargo test -p a -p b … --lib`.

### B2 — Extended contracts: serial for-loop (8 packages + wasm checks)

**Where:** `ci-extended.yml` contracts-core; `just ci-extended`.

**Why it hurts:** Longest non-Docker sequential chain on path-gated PRs. Wasm checks are another serial loop.

**Safe to parallelize?** **Yes** via GHA matrix (`package: [terp-ed25519, …]`) with `fail-fast: false`, or nextest. **Serial only if** shared `target/` thrash under act/low-RAM — matrix jobs get own runners on GHA (better).

### B3 — Within-job sequential steps on terp-scripts offline

**Where:** preflight → offline tests → rebuild-from-public → validate.

**Why it hurts:** Cannot fully parallelize: rebuild mutates derived public tables that validate asserts; preflight is a capability gate.

**Must stay serial:** **preflight → (tests ∥ rebuild only if rebuild doesn't race golden writers) → validate**. Unit+golden may run as one cargo invocation already; do **not** parallelize rebuild with golden write paths.

### B4 — `contracts-heavy` needs `contracts-core`

**Where:** `ci-extended.yml` `needs: [should-run, contracts-core]`.

**Why it hurts:** ZK packages wait for full core contracts job even if independent.

**Safe to parallelize?** **Likely yes** — drop `needs: contracts-core` unless intentional “cheap first” cost control. Tradeoff: more concurrent minutes when heavy label set.

### B5 — Docker multihop harness (wall-clock king when enabled)

**Where:** Heavy tier; `ibc_multihop_harness.rs`.

**Why it hurts:** Image pull, chain boot, multi-hop; timeout 120m. Single job, no matrix of scenarios today.

**Safe to parallelize?** **Scenario shards only if** each case can own isolated Docker network/containers (no shared chain state). Default: **serial per harness process**; optional matrix per test name with high cost. Keep **fail-closed** (no `continue-on-error`).

### B6 — Go `interchaintest-E2E.yml` (monorepo root)

**Jobs:** `build-docker` → matrix `e2e-tests` (`e2e-basic`, `e2e-pfm`, `e2e-ibc`, `e2e-polytone`; more commented).

**Already parallel** across matrix shards (`fail-fast: false`). Bottleneck is **serial Docker image build** before all tests + **retry loops** (up to 2 retries) that stretch flaky suites.

**Must stay serial:** image build before consume; per-suite may need exclusive chain resources **within** a matrix cell.

### B7 — Full `cargo check --workspace --tests` (heavy)

Compile-all is one giant graph — parallelized by cargo internally; splitting is low ROI vs caching. Not a “test packing” target first.

### B8 — act full runs

Not a GHA bottleneck; local Docker Desktop RWLayer issues. Cap rustc jobs at 2. **Do not** use full act as parallelization benchmark; use host timings (`scripts/ci/runs/latest-host-core.md`: **56s** PASS for `just ci-core` on 2026-07-21, warm-ish cache).

---

## 5. Safe-to-parallelize vs must-stay-serial

### Safe to parallelize (recommended first targets)

| Suite / unit | Why safe | Suggested mechanism |
|--------------|----------|---------------------|
| Core internal lib packages (5) | Unit/lib, no Docker, no shared FS protocol | GHA matrix job **or** single multi-`-p` / nextest |
| Extended authenticator contracts (8) | Same | Matrix per package |
| Optional zk contracts (3) | Same; optional tier | Matrix; drop hard need on contracts-core if desired |
| Go e2e matrix cells | Already matrixed | Keep; avoid serializing more make targets into one cell |
| Non-ignored terp-scripts unit/golden tests | Pure tests; cargo parallelizes threads | nextest threads; avoid FS races on golden **writes** |

### Must stay serial / gated

| Suite | Why |
|-------|-----|
| Offline preflight → rebuild-from-public → validate | Data dependency on `public/` derived tables |
| Anything writing golden/public IBC data while golden tests run | Shared artifact mutation |
| `#[ignore]` Docker harnesses sharing one daemon/image without isolation | Global Docker / chain ports |
| Live mainnet `terp-ibc generate` / mnemonic paths | Secrets + external state (not in CI) |
| Fork PRs for ci-heavy Docker | Trust boundary (already refused) |
| Draft PR skip for extended | Cost control, not correctness |

### Grey / measure first

| Item | Note |
|------|------|
| Parallel Docker multihop scenarios | Needs isolation design; high flake risk |
| `cargo test -p terp-scripts --lib --tests` including crosslink/nostr files | Non-ignored only; ignored stay out; watch for accidental un-ignore |
| Combining core offline + libs into one mega-job | **Worse** wall-clock (loses current 2-way job parallel) |

---

## 6. Timing signals (how to measure; sparse historical)

| Signal | Value | Source |
|--------|-------|--------|
| Host `just ci-core` | **56s**, PASS | `scripts/ci/runs/latest-host-core.md` (2026-07-21, commit `8f52a7f`) |
| GHA Core / Extended listing | Recent PR runs ~7–10m wall (aggregate; job detail API 404 without full access) | `gh run list` on `permissionlessweb/terp-rs` |
| Heavy | Budget 90–120m timeouts | Workflow YAML |

**Measurement recipe for `t_9212e352`:**

```sh
cd /Users/returniflost/abstract/terp-core/crates/terp-rs
just act-teardown
/usr/bin/time -p just ci-core          # baseline offline gate
/usr/bin/time -p just ci-extended      # path-gated sequential contracts
# Optional package-level:
for p in terp-auth terp-account crosslink-light-client cw721-nips terp-rs; do
  /usr/bin/time -p cargo test -p "$p" --lib
done
```

Record machine, cargo cache warm/cold, Rust 1.86.

---

## 7. Files to change next (implementation task — not this task)

| Goal | Touch |
|------|-------|
| Fan-out internal libs / contracts | `crates/terp-rs/.github/workflows/ci-core.yml`, `ci-extended.yml` |
| Host parity with matrix (or multi-package cargo) | `crates/terp-rs/justfile` `ci-core` / `ci-extended` |
| nextest partitions / profiles | new `.config/nextest.toml`; wire into just + workflows |
| Drop unnecessary `needs: contracts-core` for heavy contracts | `ci-extended.yml` |
| Multihop scenario matrix | `ci-heavy.yml` + harness test filters |
| Docs | `crates/terp-rs/docs/ci.md` |
| Go image pipeline | monorepo `.github/workflows/interchaintest-E2E.yml` (already matrixed) |
| Act env if matrix under act | `scripts/ci/lib.sh` (`CARGO_BUILD_JOBS`) |

Related tasks: full monorepo workflow scan → `t_2eef0062`; packing design → `t_f37b435d`; baseline → `t_9212e352`.

---

## 8. Estimated savings opportunities (qualitative)

| Change | Expected wall-clock effect | Risk |
|--------|----------------------------|------|
| Matrix 5 core lib packages (GHA) | Core job ~max(package) instead of sum; **medium** if packages uneven | Low flake |
| Matrix 8 contract packages | Extended contracts job **high** savings on cold-ish caches | Low–med (wasm target install once per shard cost) |
| Multi-package single cargo invocation on host | **Low–medium** vs sequential processes | Low |
| nextest + higher test threads | **Low** for small unit counts; bigger later | Low |
| Parallel multihop Docker | **High** when heavy runs; **high** flake if shared Docker | High |
| Keep 2-way core job split | Already good — do not collapse | — |

Rough host core is already ~1 minute warm; **biggest PR wins are Extended serial contracts + Heavy Docker**, not further micro-optimizing warm `ci-core`.

---

## 9. Go monorepo workflows (worktree snapshot)

| Workflow | Role |
|----------|------|
| `interchaintest-E2E.yml` | Build docker artifact → matrix e2e make targets |
| `build.yml`, `build_docker.yml`, `push-docker-image.yml` | Image / build |
| `golangci-lint.yml`, `codeql*.yml`, `dependency-review.yml` | Static analysis |
| `release.yml` | Release |
| `tests.yml.archive` | Archived unit-test workflow |

Dense packing parent epic should treat **Rust extended loops** and **Go e2e matrix balance** as separate packing domains.

---

## 10. Non-goals (this task)

- No CI YAML or justfile edits.
- No nextest introduction here.
- No claim of GHA minute savings without `t_9212e352` baselines.
