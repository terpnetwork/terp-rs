# CI wall-clock baseline — 20260808T035956Z

| Field | Value |
|-------|--------|
| date_utc | 20260808T035956Z |
| commit | `37ee97d` (`37ee97d4543db5a4204b85b3a49ac11c2e09ab59`) |
| cwd | `/Users/returniflost/abstract/terp-core/crates/terp-rs` |
| host | `Darwin 25.4.0 arm64` |
| rustc | `rustc 1.98.0-nightly (61d7280f3 2026-06-06)` |
| just | `just 1.34.0` |
| cargo | workspace packages at **0.0.2** + monorepo `[patch]` for cosmwasm/cw-minus/cw-plus/cw-asset stack |
| purpose | Post-unblock re-baseline after Cargo.lock 0.0.2 regenerate (task t_dd89f610) |

## Baseline table (measured this run)

| Command | Duration (s) | Result | Label |
|---------|-------------:|--------|-------|
| `just ci-core` (cold after resolve fix) | 248 | **PASS** | Tier 0 composite (first green compile) |
| `just ci-core` (warm) | 29 | **PASS** | Tier 0 composite (authoritative wall-clock) |
| `just scripts-ibc-preflight offline` | 2 | **PASS** | IBC preflight |
| `just scripts-ibc-offline` | 21 | **PASS** | IBC offline tests |
| `just scripts-ibc-validate` | 1 | **PASS** | IBC validate |
| `cargo test -p terp-auth --lib` | 1 | **PASS** | lib unit |
| `cargo test -p terp-account --lib` | 1 | **PASS** | lib unit |
| `cargo test -p crosslink-light-client --lib` | 92 | **PASS** | lib unit (cold-ish zebra) |
| `cargo test -p cw721-nips --lib` | 1 | **PASS** | lib unit |
| `cargo test -p terp-rs --lib` | 2 | **PASS** | lib unit |
| `just ci-extended` | 18 | **FAIL** (exit 101) | Tier 1 — compile fixed (`NetworkInfoOwned` import); residual: `delegations` bin tests missing fixture files |
| `just act-wire` | 1 | **FAIL** | Docker Desktop starting; `docker info` hangs / not ready |

**Authoritative Tier 0 wall-clock:** **29s** warm host `just ci-core` **PASS** @ `37ee97d`.

**Cold first green:** 248s (includes heavy native rebuild: librocksdb-sys / ict-rs / zebra stack).

## Comparison to prior baseline

| Ref | just ci-core | Result |
|-----|-------------:|--------|
| Historical green @ `8f52a7f` | 56s | PASS |
| FAIL-fast lock stale @ `37ee97d` (pre-unblock) | 6s | FAIL (resolve) |
| **This re-baseline (warm)** | **29s** | **PASS** |

Warm Tier 0 is **faster than historical 56s** (dense multi-package core-libs + resolved lock).

## Unblock changes applied (t_dd89f610)

1. **Cargo.lock regenerated** so workspace members match `[workspace.package].version = "0.0.2"` (e.g. `terp-auth`, `terp-scripts`).
2. **Monorepo `[patch]` tables** in root `Cargo.toml` force permissionlessweb git + crates.io CosmWasm stack onto local path forks:
   - `../cosmwasm/packages/*`
   - `../cw-minus`, `../cw-plus`, `../cw-plus-plus`, `../cw-asset`, `../cw-storage-plus`, `../cw-packages`
3. **Kept** path stubs `crates/frost_escrow` + `crates/threshold_committee` (for optional `zk-test-press` / headstash path graph). Not full crypto; untracked local scaffolding.
4. **Disk pressure:** first green attempt failed mid-`librocksdb-sys` with ENOSPC; freed ~27–37 GiB (other crate `target/` dirs + incremental) then re-ran successfully.

## Residual blockers

| ID | Severity | Note |
|----|----------|------|
| R1 | Tier 1 | `just ci-extended` residual: `delegations` bin tests missing fixture files (NetworkInfoOwned import fixed) |
| R2 | Secondary | Docker Desktop not ready for `just act-wire` (`docker info` hang while backend starts). Full `act-core` still secondary / M-series RWLayer risk |
| R3 | Env | Host disk was 100% full during cold build — keep spare space for rocksdb native compile |

## Copy-paste reproduction

```sh
cd /Users/returniflost/abstract/terp-core/crates/terp-rs
just ci-core          # primary Tier 0 gate — expect ~29s warm PASS
just ci-extended      # Tier 1 — currently FAIL until NetworkInfoOwned import
just act-wire         # needs healthy Docker daemon
```

## Skipped

| Path | Why |
|------|-----|
| `just act-core` | Secondary YAML fidelity; Docker not ready + known Desktop arm64 risk |
