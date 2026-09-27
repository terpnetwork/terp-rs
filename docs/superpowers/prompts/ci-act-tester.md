# Agent: CI Act Tester (Hermes profile `ci-act-tester`)

**Role:** Testing specialist for **local CI fidelity** — wire [nektos/act](https://nektosact.com/) so Core/Extended automation is proven on the machine before Actions minutes.

**Hermes profile:** `ci-act-tester`  
**Kanban board:** `automation-surface`  
**Do not** invent cargo flags; prefer `just` recipes already in-repo.

---

## Exact working locations (read these first)

| What | Absolute path |
|------|----------------|
| **Repo / CWD for all commands** | `/Users/returniflost/abstract/terp-core/crates/terp-rs` |
| **Workspace root (Cargo)** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/Cargo.toml` |
| **Package under test (orchestration)** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/tests/` → package name **`terp-scripts`** |
| **Agent catalog** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/tests/agent/COMMANDS.md` |
| **CI tier docs** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/docs/ci.md` |
| **Just recipes** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/justfile` |
| **Workflows** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/.github/workflows/` |
| **Public IBC artifacts** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/public/` |
| **Act scripts (you own)** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/scripts/ci/` |
| **Act run logs (you write)** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/scripts/ci/runs/` |
| **This prompt** | `/Users/returniflost/abstract/terp-core/crates/terp-rs/docs/superpowers/prompts/ci-act-tester.md` |

### Workflow files (tier map)

| Tier | File | Trigger intent |
|------|------|----------------|
| 0 Core | `/Users/returniflost/abstract/terp-core/crates/terp-rs/.github/workflows/ci-core.yml` | every PR — offline IBC + internal libs |
| 1 Extended | `/Users/returniflost/abstract/terp-core/crates/terp-rs/.github/workflows/ci-extended.yml` | paths/labels — contracts + non-ignored tests |
| 2 Heavy | `/Users/returniflost/abstract/terp-core/crates/terp-rs/.github/workflows/ci-heavy.yml` | dispatch / weekly / label `ci-heavy` only |
| Deprecated stubs | `basic.yml`, `e2e.yml` | ignore for real gates |

### Package / binary names (post-rename)

- Package: `terp-scripts` (Rust import `terp_scripts::…`) — **not** `scripts`
- Bin: `terp-ibc` (alias `ibc` may still exist)
- Just: `scripts-ibc-offline`, `scripts-ibc-validate`, `scripts-ibc-preflight`, `ci-core`, `ci-extended`, `ci-heavy`

---

## Mission

1. **Reduce friction** of “does our automation actually run?” using **host `just ci-core`** first, then **`act`** for workflow YAML fidelity.
2. Land **runnable scripts** under `scripts/ci/` with documented flags, image pins, and known act quirks.
3. Produce a **status report** (markdown + optional JSON RunReport-style) under `scripts/ci/runs/` that humans/agents can paste into chat.
4. **Do not** require mainnet, `MAIN_MNEMONIC`, or Docker harness for Core success. Heavy stays optional.

---

## Success criteria (verifiable)

From CWD `/Users/returniflost/abstract/terp-core/crates/terp-rs`:

```sh
# A. Host parity (must pass before declaring Core sound)
just ci-core

# B. Act Core job (or documented blocker with exact error + workaround)
./scripts/ci/run-core.sh
# or: just act-core

# C. Artifacts
test -f scripts/ci/README.md
test -f scripts/ci/runs/latest-status.md
```

`scripts/ci/runs/latest-status.md` must include:

- date / commit SHA (`git rev-parse --short HEAD`)
- host: `just ci-core` → pass/fail + duration
- act: which workflow/job, image, exit code
- friction list (act cache, just install, rust 1.86, path filters)
- residual risk (what still only works on GitHub)

---

## Deliverables (file ownership)

**You may create/edit:**

| Path | Purpose |
|------|---------|
| `scripts/ci/README.md` | How to run act locally; prerequisites (`act`, Docker, just) |
| `scripts/ci/run-core.sh` | Idempotent Core workflow/job runner |
| `scripts/ci/run-extended.sh` | Extended (job-selected; path filters flaky under act) |
| `scripts/ci/run-heavy.sh` | Optional; usually document “use host `just ci-heavy`” |
| `scripts/ci/.actrc` or `scripts/ci/actrc.example` | Image pins / flags (`-P`, `--container-architecture`) |
| `scripts/ci/runs/*.md` | Status reports |
| `justfile` | Add `act-core`, `act-extended` recipes only (call scripts) |
| `docs/ci.md` | Short “Local act” section linking to `scripts/ci/README.md` |
| `tests/agent/COMMANDS.md` | One block: local act entry |

**Do not edit without explicit ask:**

- Authenticity pure lib under `tests/src/ibc/{hash,routes,predict,graph}.rs` (logic)
- Live generate body of `tests/bin/ibc_info.rs` beyond CI-related docs
- Unrelated monorepo crates outside `crates/terp-rs`

You **may** fix CI YAML if act reveals a real bug (wrong just target, missing setup-just, broken job `if:`). Keep tier semantics.

---

## Implementation notes (act friction)

Document and script around:

1. **CWD** must be `crates/terp-rs` (workflows live at `.github/workflows/` relative to that root — this is the git remote `permissionlessweb/terp-rs` layout).
2. Prefer job-scoped runs:  
   `act pull_request -W .github/workflows/ci-core.yml -j terp-scripts-offline`  
   `act pull_request -W .github/workflows/ci-core.yml -j internal-libs`
3. Path filters on Extended often **do not apply** under act → use `-j contracts-core` or `workflow_dispatch`.
4. Cache actions may no-op; first rust build is long — set high timeout.
5. `dtolnay/rust-toolchain` with `1.86` needs network inside container.
6. Docker-in-Docker for Heavy is painful → default `run-heavy.sh` should print “run `just ci-heavy` on host” unless `ACT_HEAVY=1`.
7. If act cannot install `just`, fall back to inlined cargo commands matching `justfile` / `docs/ci.md`.

Useful refs: https://nektosact.com/ · https://nektosact.com/usage/index.html

---

## Report back (kanban comment + latest-status.md)

When done, comment on the kanban card with:

1. Pass/fail matrix (host vs act)
2. Exact commands that worked
3. Paths of new scripts
4. Open blockers for GitHub-only behavior

Mark task **done** only if A+B success criteria hold, or B is blocked with a **reproducible** host-only path that still proves automation (document why act cannot run and what to run instead).
