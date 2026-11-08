# Local CI exercise (host + act)

**CWD:** `/Users/returniflost/abstract/terp-core/crates/terp-rs`

## Optimization model (time)

| Priority | Command | Docker? | What it proves | Time |
|----------|---------|---------|----------------|------|
| **1 — always** | `just act-host-core` / `just ci-core` | no | Real tests (terp-scripts offline + libs) | minutes, cargo-cached |
| **2 — wiring** | `just act-wire` | light | Workflow YAML parseable (`act -l`) | seconds |
| **3 — optional** | `just act-core` | heavy | Full GHA job graph under act | long; fragile on Docker Desktop |
| **cleanup** | `just act-teardown` | n/a | Kill act containers + volumes | seconds |

**Do not** use full act as the merge gate on macOS if Docker Desktop hits `RWLayer nil`. Host green is authoritative.

## Commands

```sh
cd /Users/returniflost/abstract/terp-core/crates/terp-rs

# Recommended daily loop
just act-teardown          # clear lagging containers/volumes
just act-host-core         # timed just ci-core → scripts/ci/runs/latest-host-core.md
just act-wire              # list jobs in ci-*.yml; always tears down

# Optional full act (after Docker restart if needed)
ACT_PULL=0 ACT_JOB_TIMEOUT=900 just act-core
just act-teardown          # belt-and-suspenders

# Extended host
just ci-extended
```

## Teardown (always)

Full act runners install `trap EXIT INT TERM → act_teardown`. Also run explicitly:

```sh
./scripts/ci/teardown.sh
# or
just act-teardown
```

Removes containers matching act labels/names and volumes matching `act|nektos`, then prunes old `scripts/ci/runs/*_tmp` (keeps last `ACT_RUNS_KEEP=5`).

## Env knobs

| Env | Default | Meaning |
|-----|---------|---------|
| `ACT_PULL` | `0` | Set `1` to re-pull runner image |
| `ACT_JOB_TIMEOUT` | `900` | Seconds per act job before kill + teardown |
| `ACT_IMAGE` | `catthehacker/ubuntu:act-latest` | Runner image |
| `ACT_ARCH` | auto arm64 on Apple Silicon | e.g. `linux/amd64` |
| `CARGO_BUILD_JOBS` | `2` | Cap parallel rustc inside act |
| `ACT_RUNS_KEEP` | `5` | How many run log dirs to keep |
| `ACT_WIRE_DRY` | `0` | `1` = also run `core-gate` job under act |

## Layout

| Path | Role |
|------|------|
| `lib.sh` | paths, teardown, act_run_job |
| `teardown.sh` | standalone cleanup |
| `run-host-core.sh` | timed host Tier 0 |
| `run-wire.sh` | fast act list |
| `run-core.sh` | full act Core jobs |
| `run-extended.sh` | full act Extended jobs |
| `runs/latest-status.md` | combined status |
| `runs/latest-host-core.md` | host gate report |

## GHA vs monorepo act

- **GitHub** (`permissionlessweb/terp-rs`): checkout root = terp-rs; workflow steps use `.`
- **Local act**: bind monorepo (`terp-core`) so `../cosmwasm` etc. resolve; scripts set `GITHUB_WORKSPACE` to the crate root
