# Dense parallel test packing (terp-rs CI)

**Task:** `t_f37b435d` (board `terp-core-ci`)  
**Depends on:** inventory `t_3c311b49`, baseline `t_9212e352`  
**Date:** 2026-08-07  

## Goal

Reduce CI wall-clock by running independent suites concurrently and packing
shards **densely** (high utilization, minimal idle matrix slots) without
dropping coverage or introducing shared-state flakes.

## Strategy

| Layer | Mechanism | Rationale |
|-------|-----------|-----------|
| **Host `just ci-core` / `ci-extended`** | Multi-package `cargo test -p a -p b … --lib` via `scripts/ci/dense-packs.sh` (`CI_DENSE_MODE=all`, default) | One rustc graph + shared `target/`; denser than sequential single-`-p` processes |
| **GHA Core `internal-libs`** | Matrix of **2** packs (`core-libs-a`, `core-libs-b`), `fail-fast: false` | Parallel runners; each cell multi-package (not 5 sparse jobs) |
| **GHA Extended `contracts-core`** | Matrix of **3** packs (`contracts-a`..`c`) + pack-local wasm checks | Longest prior sequential loop; 3 dense cells ≫ 8 one-package jobs |
| **GHA `contracts-heavy`** | Multi-package single job; **no** `needs: contracts-core` | ZK packages independent of core contracts; avoid false serialization |
| **Offline IBC plane** | **Serial** preflight → offline tests → rebuild-from-public → validate | Shared `public/` mutation; inventory B3 |
| **Docker multihop** | Unchanged serial in CI Heavy | High flake if naively matrixed (inventory B5) |

### Packing rule

1. Prefer **multi-package cargo** inside a cell over one package per job.
2. Keep matrix cardinality **small** (2 core packs, 3 contract packs) so cold
   cache/setup is amortized and slots stay busy.
3. Balance by **package class / expected compile weight** (auth+sdk vs
   light-client+nips; crypto vs recovery vs suite+IBC client), not pure
   alphabetical splits.
4. When historical durations exist (`scripts/ci/runs/*`), rebalance so max pack
   ≈ median; until then use class weights from inventory.
5. Same packages still run overall — packing is **partition**, not skip.

### Shard map

```
core-libs-a:  terp-auth, terp-account, terp-rs
core-libs-b:  crosslink-light-client, cw721-nips

contracts-a:  terp-ed25519, terp-passkey, terp-vsck   (+ wasm check)
contracts-b:  terp-eth, terp-bls, terp-recovery
contracts-c:  terp-authenticator-suite, cw-ics08-wasm-crosslink  (+ wasm)

contracts-heavy (optional label/dispatch):
  terp-zkjwt
```

List at any time: `just ci-dense-list` or `./scripts/ci/dense-packs.sh list`.

## Serial exceptions (must not parallelize)

| Suite | Why |
|-------|-----|
| Offline preflight → offline tests → rebuild → validate | `public/` data dependency |
| Golden tests concurrent with rebuild writers | Shared artifact mutation |
| Docker multihop / `#[ignore]` harnesses without isolation | Global Docker/chain ports |
| Live mainnet `terp-ibc generate` | Secrets + external state |

## Env / resource notes

| Variable | Meaning |
|----------|---------|
| `CI_DENSE_MODE=all` | Collapse packs into one multi-package cargo (host default) |
| `CI_DENSE_MODE=matrix` | Run packs one-by-one (or parallel if below set) |
| `CI_DENSE_PARALLEL=1` | Host only: run matrix packs in parallel with **isolated** `CARGO_TARGET_DIR=target/dense-<pack>` (avoids shared-target races; more disk) |
| `CI_DENSE_PACK=<name>` | Run a single pack (GHA matrix cell) |
| `CARGO_BUILD_JOBS` | rustc job cap; act defaults to **2** (`scripts/ci/lib.sh`) |

No new global locks. Do **not** enable `CI_DENSE_PARALLEL=1` against a shared
`target/` without isolated dirs — cargo file locks help but concurrent graphs
still thrash and can flake under low RAM.

## Local commands

```sh
cd /Users/returniflost/abstract/terp-core/crates/terp-rs

# Primary host gates (dense multi-package libs/contracts)
just ci-core
just ci-extended

# Inspect packs
just ci-dense-list

# Simulate GHA matrix packs on host (serial packs, shared target)
just ci-core-matrix
just ci-extended-matrix

# Optional true parallel packs (isolated target dirs)
CI_DENSE_MODE=matrix CI_DENSE_PARALLEL=1 ./scripts/ci/dense-packs.sh core-libs

# Act YAML fidelity (after Docker is healthy)
just act-wire
```

## Files touched

| Path | Change |
|------|--------|
| `scripts/ci/dense-packs.sh` | Pack definitions + runner |
| `.github/workflows/ci-core.yml` | internal-libs dense 2-pack matrix |
| `.github/workflows/ci-extended.yml` | contracts 3-pack matrix; heavy independent |
| `justfile` | `ci-core` / `ci-extended` + matrix helpers |
| `docs/ci/dense-parallel-packing.md` | This design note |
| `docs/ci.md` | Pointer to dense packing |

## Baseline / expected gains

From `t_9212e352`: current HEAD fails cargo resolve (~6s fail-fast). Historical
green `just ci-core` ≈ **56s** @ `8f52a7f`. Re-time after lock fix (`t_de000feb`).

| Change | Expected wall-clock | Risk |
|--------|---------------------|------|
| Host multi-package core libs | Low–medium vs sequential `-p` | Low |
| GHA 2-pack core libs | ~max(pack) vs sum on separate runners | Low |
| GHA 3-pack contracts | **High** on Extended path | Low–med |
| Drop heavy `needs: contracts-core` | Medium when heavy label set | Low |

## Coverage parity

Every package previously tested under Core/Extended still appears in exactly one
dense pack (heavy optional tier unchanged membership). No `#[ignore]` removed;
Docker harness remains CI Heavy only.

## Follow-ups

- `t_de000feb`: verify parity + speedup vs baseline after resolve is green.
- Rebalance packs when per-package durations exist.
- Optional nextest profiles later; not required for this packing design.
