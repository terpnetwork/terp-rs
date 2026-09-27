# CI tiers (terp-rs)

Automation is tiered so PRs always get an honest offline gate without burning Docker minutes or mainnet secrets.

| Tier | Workflow | When | What |
|------|----------|------|------|
| **0 Core** | `ci-core.yml` | every PR / push to main & feat/* / manual | `terp-scripts` offline IBC (unit+golden+rebuild+validate+preflight) + internal lib crates |
| **1 Extended** | `ci-extended.yml` | path changes under contracts/crates/tests **or** label `ci-extended` **or** manual | authenticator contracts lib tests, optional wasm check, all non-ignored `terp-scripts` tests; draft PRs skipped unless labeled |
| **2 Heavy** | `ci-heavy.yml` | **manual** `workflow_dispatch`, **weekly** schedule, or label **`ci-heavy`** (same-repo only) | `cargo check --workspace`, Docker multihop harness |

## Labels (maintainer / admin)

| Label | Effect |
|-------|--------|
| `ci-extended` | Force extended jobs on draft PRs; enable optional zk contract job |
| `ci-heavy` | Run Docker harness + workspace check (not for forks) |

`workflow_dispatch` on **CI Heavy** is the admin path: choose Docker and/or workspace check in the UI without labeling a PR.

## Local parity (agents)

```sh
# Tier 0
just ci-core

# Tier 1 (no Docker)
just ci-extended

# Tier 2 (Docker)
just ci-heavy
```

Same commands CI runs — no invented cargo flags.

### Local exercise (optimized)

```sh
cd /Users/returniflost/abstract/terp-core/crates/terp-rs

just act-exercise     # teardown → host ci-core (timed) → act-wire → teardown
# or stepwise:
just act-teardown     # kill lagging act containers/volumes
just act-host-core    # authoritative tests
just act-wire         # act -l only (seconds)
just act-core         # optional full act (slow; Docker Desktop fragile)
just act-teardown
```

| Recipe | Role |
|--------|------|
| `act-host-core` | Real Tier 0 tests (no act) |
| `act-wire` | Workflow parse / job list |
| `act-core` | Full act jobs + per-job teardown |
| `act-teardown` | Containers + volumes + prune run logs |
| `act-exercise` | Daily optimized loop |

Details: `scripts/ci/README.md`. Hermes profile: `ci-act-tester`.

## Dense parallel packing

Independent lib/contract packages run as **dense multi-package packs** (host:
single multi-`-p` cargo; GHA: small matrix of packs), not one sparse job per
crate. Offline IBC preflight→rebuild→validate stays **serial**.

| Entry | Role |
|-------|------|
| `just ci-core` / `ci-extended` | Host dense packing (`CI_DENSE_MODE=all`) |
| `just ci-dense-list` | Print pack membership |
| `just ci-core-matrix` / `ci-extended-matrix` | Host simulation of GHA packs |
| `scripts/ci/dense-packs.sh` | Pack definitions + runner |
| `docs/ci/dense-parallel-packing.md` | Design: strategy, shards, serial exceptions |

Env: `CI_DENSE_MODE`, `CI_DENSE_PARALLEL`, `CI_DENSE_PACK` — see design note.

## What is *not* in CI

- Live mainnet `terp-ibc generate` (needs `MAIN_MNEMONIC` + gRPC) — use preflight + human/live mode only.
- Silent golden overwrite.
- Deprecated `Basic` / old launchpad `E2E` workflows (stubs only).

## Branch protection (recommended)

Require status checks:

1. `terp-scripts offline (ibc unit/golden + validate)`  
2. `internal lib crates`  
3. `core gate`

Optional (path-gated): contract jobs from CI Extended.

Do **not** require CI Heavy on every PR.

## Fail philosophy

- Core offline must be **green** or the PR is not mergeable.  
- Heavy failures are real (fail closed) when the tier is intentionally run — no `continue-on-error` on harness.  
- Missing Docker images on heavy = red (fix image tags / secrets next, don’t hide).
