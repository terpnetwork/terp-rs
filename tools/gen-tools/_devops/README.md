# Dependency Management Tooling

Tooling for managing forked crate dependencies across many Rust workspaces
in the Terp monorepo (`terp-core/crates`).

**Stable operator surface:** see [`STABLE.md`](./STABLE.md) and `_scripts/dep.py`.

---

## Quick Reference

```bash
# From crates/terp-rs/tools/gen-tools
just deps-config         # monorepo root + policy
just deps-validate       # matrix integrity (CI gate)
just deps-heal           # fix absolute/wrong package paths in matrix
just deps-status         # workspace dep modes
just deps-local          # switch → path deps
just deps-git            # switch → git remotes
just deps-stable         # switch → crates.io
just deps-dev            # git identity + local path patches
just deps-patch-local dao-contracts   # version lines + path patch
just deps-patch-git dao-contracts     # version lines + git rev patch
just deps-pin-revs neutron-std        # matrix branch -> full rev
just deps-lint-patches dao-contracts  # drop unused [patch] entries
just deps-verify local   # cargo metadata source check
just deps-scrape         # full scan + diagnostics
just deps-overview       # OVERVIEW.md dashboard

# Granular
python3 _scripts/dep.py switch git --crate cosmwasm-std
python3 _scripts/dep.py switch local --workspace cw-plus
python3 _scripts/dep.py switch git --workspace abstract/framework --target patches
python3 _scripts/dep.py switch git --dry-run
```

---

## Architecture

```
_devops/
  dependency-matrix.toml     # Source of truth: crate versions, URLs, paths
  migration-progress.yaml    # Build status per workspace (check/test)
  dep-scrape.json            # Generated: full dep scan data

_scripts/
  dep_common.py              # Shared library: discovery, parsing, TOML I/O
  dep-switch.py              # Switch deps between local/git/stable/zk modes
  dep-scrape.py              # Scan all Cargo.tomls, classify deps, diagnostics
  dep-overview.py            # Generate OVERVIEW.md dashboard

OVERVIEW.md                  # Generated: project tables, health, matrix coverage
```

---

## The Dependency Matrix

`_devops/dependency-matrix.toml` is the single source of truth for every forked
crate. Each entry defines where the crate lives across all resolution modes.

```toml
[crates.cosmwasm-std]
description = "Standard library for CosmWasm contracts"
package = "cosmwasm-std"
stable = "3.0.4"
git = { url = "https://github.com/permissionlessweb/cosmwasm", branch = "main" }
local = "cosmwasm/packages/std"
consumers = ["cw-plus", "abstract/framework", "cw-orchestrator"]
```

### Fields

| Field         | Purpose                                                        |
|---------------|----------------------------------------------------------------|
| `stable`      | crates.io version used in release builds                       |
| `fork_version`| Exact fork version when it differs from `stable` (`=4.0.0-zk`) |
| `git`         | Forked git repo URL + branch. `rev` or `commit` is the full SHA `patch-git` writes |
| `local`       | Path relative to monorepo root for local development           |
| `zk_git`      | ZK variant git source, falls back to `git` if absent           |
| `zk_local`    | ZK variant local path, falls back to `local` if absent         |
| `package`     | Published package name (if different from the matrix key)      |
| `consumers`   | Which workspace directories depend on this crate               |
| `description` | Human-readable purpose of the crate                            |

Set a field to `"N/A"` when a mode is not applicable.

---

## dep-switch.py — The Dependency Switcher

Reads the matrix and rewrites Cargo.toml files across all discovered
workspaces. Handles three levels of dependency declaration:

1. **`[patch.crates-io]`** — workspace root overrides
2. **`[workspace.dependencies]`** — centralized dep specs inherited by members
3. **Direct member deps** — inline deps and `[dependencies.name]` expanded
   table sections in individual member Cargo.tomls

### Modes

| Mode       | Effect                                              |
|------------|-----------------------------------------------------|
| `stable`   | Revert to crates.io version (remove path/git)       |
| `local`    | `path = "../../fork/..."` (relative to each file)   |
| `git`      | `git = "...", branch = "..."` (from matrix)         |
| `zk_local` | ZK fork local paths, falls back to `local`          |
| `zk_git`   | ZK fork git URLs, falls back to `git`               |
| `patch-local` | Version lines stay. `[patch.crates-io]` is the local path |
| `patch-git` | Version lines stay. `[patch.crates-io]` is `git` + a full `rev` |

`patch-git` does not add `[patch.'<git-url>']`. Cargo keeps a crates.io→git replacement and will not chain a path over that same edge. `patch-local` is the unified shape: crate versions in the dependency line, local paths only in `[patch.crates-io]`. It drops a git-url section once the path graph no longer depends on that URL. `patch-git` still leaves an existing git-url section in place. `dev` remains the mode that puts git in the dependency line and a path in `[patch.'<url>']`.

### Targets

| Target     | What gets rewritten                                  |
|------------|------------------------------------------------------|
| `patches`  | Only `[patch.crates-io]` sections                    |
| `ws-deps`  | Only `[workspace.dependencies]` entries              |
| `members`  | Only direct deps in member Cargo.tomls               |
| `all`      | All three (default)                                  |

### Usage

```bash
# Full switch — all targets, all workspaces, all matrix crates
python3 _scripts/dep-switch.py switch local
python3 _scripts/dep-switch.py switch git
python3 _scripts/dep-switch.py switch stable

# Target specific workspace(s)
python3 _scripts/dep-switch.py switch git --workspace abstract/framework
python3 _scripts/dep-switch.py switch local --workspace cw-orchestrator

# Target specific crate(s)
python3 _scripts/dep-switch.py switch git --crate cosmwasm-std
python3 _scripts/dep-switch.py switch local --crate cosmwasm-std --crate cw-storage-plus

# Combined: one crate in one workspace
python3 _scripts/dep-switch.py switch git --workspace abstract/framework --crate cosmwasm-std

# Only edit specific target level
python3 _scripts/dep-switch.py switch local --target patches
python3 _scripts/dep-switch.py switch git --target members

# Preview without writing
python3 _scripts/dep-switch.py switch git --dry-run

# Show current state (patches, ws-deps, and member direct deps)
python3 _scripts/dep-switch.py status
```

### Branch Specifications

Every git dep in the matrix includes an explicit `branch = "..."`. When
switching to git mode, dep-switch writes this branch into every Cargo.toml
entry. This is critical — without an explicit branch, Cargo defaults to the
repo's default branch, which may not be your working branch.

`patch-git` does not write that branch. It writes `rev` from `git.rev` or
`git.commit`, and only when the value is a full 40-hex SHA. A missing or
short rev is skipped with a warning. That pin is what makes a publish
reproducible. The branch field stays in the matrix for `git` / `dev` mode.

The matrix branches must match the actual branches you push to. Use
`dep-switch.py status` to verify, and check the matrix against local branches:

```bash
# Compare matrix branches to local repo branches
for dir in */; do
    [ -d "$dir/.git" ] || continue
    branch=$(git -C "$dir" branch --show-current)
    echo "$dir: $branch"
done
```

### How It Works

1. Loads the matrix from `dependency-matrix.toml`
2. Discovers all workspace roots via `dep_common.discover_projects()`
3. For each workspace, rewrites the selected targets:
   - **Patches**: parses `[patch.crates-io]` line by line, replaces managed
     entries, preserves static/non-matrix entries
   - **Workspace deps**: rewrites matching entries in `[workspace.dependencies]`
   - **Member deps**: walks every member Cargo.toml, rewrites both inline deps
     (`name = { version = "..." }`) and expanded table sections
     (`[dependencies.name]` multi-line format)
4. Computes relative paths from each file's location to the fork directory

---

## dep-scrape.py — Dependency Scraper

Scans all Cargo.tomls across all projects, classifies every dependency, and
cross-references against the matrix.

```bash
python3 _scripts/dep-scrape.py                    # Full scan → JSON + summary
python3 _scripts/dep-scrape.py --project cw-plus  # Single project
python3 _scripts/dep-scrape.py --json-only        # JSON only
python3 _scripts/dep-scrape.py --summary-only     # Human summary only
python3 _scripts/dep-scrape.py --check            # Exit 1 on warnings (CI)
```

Output includes per-project counts of path/git/registry deps, patch entries,
and diagnostics like git deps that should be local or version mismatches.

---

## dep-overview.py — Dashboard Generator

Generates `OVERVIEW.md` at the repo root with:

- **Summary**: total projects, workspace members, dep entries, matrix crates
- **Project tables**: grouped by migration layer, with dep counts and
  cargo check/test status icons
- **Dependency Health**: diagnostic warnings and info
- **Matrix Coverage**: every matrix crate with declared vs actual consumers,
  linked to project Cargo.tomls

```bash
python3 _scripts/dep-overview.py                  # Generate OVERVIEW.md
python3 _scripts/dep-overview.py --output path.md # Custom output path
```

---

## Workflow: Switching Between Modes

The standard development cycle uses three modes. Each represents a commit
point so you can switch between them cleanly.

### 1. Local Development (path deps)

```bash
just deps-local
# All matrix crates resolve to ../fork-dir/ paths
# Edit code across repos, changes reflect immediately
cargo check --workspace
```

Commit in each sub-repo with a message like `local deps stable` to bookmark
the known-good local state.

### 2. Remote Verification (git deps)

```bash
just deps-git
# All matrix crates resolve to git remotes
# Verifies your remotes have the correct code pushed
```

Commit in each sub-repo with `remote git deps` to bookmark the git state.

### 3. Release (crates.io)

```bash
just deps-stable
# All matrix crates resolve from crates.io registry
# This is what downstream users will build against
cargo check --workspace
cargo test --workspace
```

---

## Pushing to Git Remotes

When switching from local to git deps, each workspace specifies dependencies
like:

```toml
cosmwasm-std = { git = "https://github.com/org/cosmwasm", branch = "main" }
```

Cargo resolves `branch = "main"` to the **tip commit of that branch at fetch
time**, and records the exact commit hash in `Cargo.lock`. Since every consumer
points to the same URL + branch, Cargo deduplicates them into a single copy --
no version conflicts.

### Push in dependency order

Repos form a dependency graph. Push upstream repos first so that downstream
repos can resolve their git deps immediately. If you push out of order, a
downstream `cargo check` will fetch a stale upstream commit.

```
Layer 1 (no deps on other forks):
  tendermint-rs, cosmos-rust, ibc-proto-rs

Layer 2 (depends on Layer 1):
  cosmwasm (cw-minus), cw-plus, cw-plus-plus, cw-asset, cw-nfts

Layer 3 (depends on Layer 2):
  cw-multi-test-fork, clone-cw-multi-test,
  osmosis-test-tube, neutron-test-tube, neutron-std, osmosis-rust

Layer 4 (depends on Layers 1-3):
  polytone, wynddex, wynd-lsd, dao-contracts,
  cw-ibc-demo, cw-packages, abstract-cw-plus, xion-account

Layer 5 (depends on everything):
  cw-orchestrator, abstract/framework
```

### Cargo.lock regeneration

After pushing upstream repos and before building downstream, regenerate the
lock file so Cargo fetches the new commits:

```bash
cargo update              # Update all git deps to latest branch tips
cargo update -p crate     # Update a specific git dep only
```

Without this, Cargo reuses the commit hash from the old `Cargo.lock`, which
may point to a pre-push commit or a commit that no longer exists on the remote.

### Verify after push

```bash
# After pushing all repos, switch to git and confirm resolution
just deps-git
just deps-scrape

# The scraper should show 0 non-git matrix deps
# Then check each critical workspace builds:
cd cw-plus && cargo check
cd abstract/framework && cargo check
```

### Common pitfalls

- **Forgot to push a dependency**: Cargo will fetch an old commit from the
  remote that may lack your changes. Symptom: compile errors referencing
  missing types or methods.
- **Branch mismatch**: The matrix says `branch = "main"` but you pushed to a
  feature branch. Update the matrix or push to the correct branch.
- **Stale Cargo.lock**: After pushing, run `cargo update` in downstream repos.
  The lock file pins exact commit hashes -- it won't auto-update.
- **Circular git deps**: If A depends on B and B depends on A via git, both
  must be pushed before either can resolve. Break the cycle by pushing one
  with the dep temporarily removed, then push the other, then re-add.

---

## Commit Workflow for Mode Switches

Each sub-directory in the monorepo is its own independent git repository (not
submodules). Mode switches modify Cargo.toml files across many repos at once.
The recommended workflow:

```bash
# 1. Switch to local, verify, commit
just deps-local
# ... develop, test ...
# Commit in each sub-repo:
#   "local deps stable" or "local deps: bump cosmwasm-std to 3.0.4"

# 2. Switch to git, verify, commit
just deps-git
just deps-scrape          # Confirm 0 non-git matrix deps
# Commit in each sub-repo:
#   "remote git deps"

# 3. Push all repos (in dependency order)
# See "Push in dependency order" above

# 4. Verify remotes resolve
just deps-git
cd abstract/framework && cargo update && cargo check
```

Keep both "local deps" and "git deps" commits so you can switch between modes
with `git checkout`:

```bash
# Need to go back to local development?
cd cw-plus && git checkout HEAD~1    # Back to "local deps stable" commit
# Or better: use named branches
cd cw-plus && git checkout local-dev
```

---

## When `[patch]` Works and When It Doesn't

### Works

- **Any crate published on crates.io.** Cargo uses the patch unconditionally
  as long as the package name matches and the semver requirement is compatible.

- **Transitive deps pulled via git URL:**

  ```toml
  [patch.'https://github.com/upstream/repo']
  some-crate = { path = "../local-fork/some-crate" }
  ```

### Does NOT Work

- **Crates never published on crates.io.** Use `path =` directly in
  `[workspace.dependencies]` instead.

- **Package name mismatches.** The `package` field must match what consumers
  resolve. When a fork renames a crate:

  ```toml
  [patch.crates-io]
  original-name = { path = "../fork/crate", package = "fork-name" }
  ```

- **Patches don't propagate across workspace boundaries.** If workspace A
  patches `cw-utils` and workspace B depends on A's crate, B does NOT inherit
  A's patch. B must declare its own patch. This is why the switcher edits
  patches in every workspace root independently.

### Why Direct Deps Over Patches

Some workspaces in this monorepo use direct path/git deps in
`[workspace.dependencies]` and member Cargo.tomls instead of patches. This is
intentional:

- **Patches are fragile across workspace boundaries.** A patch in workspace A
  does not affect workspace B's resolution, even if B depends on A.
- **Direct deps are explicit.** The Cargo.toml shows exactly what source is
  being used -- no hidden overrides.
- **The switcher handles both.** `dep-switch.py --target all` rewrites patches,
  workspace deps, and member deps in a single pass, so there's no extra
  maintenance burden for using direct deps.

---

## Adding a New Forked Crate

1. **Add to the matrix:**

   ```toml
   # _devops/dependency-matrix.toml
   [crates.new-crate]
   description = "What this crate does"
   stable = "1.0.0"
   git = { url = "https://github.com/yourorg/new-crate", branch = "main" }
   local = "new-crate"
   consumers = ["workspace-a", "workspace-b"]
   ```

2. **Clone the fork** into the monorepo root:

   ```bash
   git clone https://github.com/yourorg/new-crate
   ```

3. **Run the switcher** to wire it up:

   ```bash
   just deps-local                   # Writes path deps everywhere
   just deps-scrape                  # Verify it appears in the scan
   ```

4. **Update the overview:**

   ```bash
   just deps-overview
   ```

---

## Troubleshooting

### "failed to get `crate` as a dependency" after switching to git

The remote doesn't have the expected branch or hasn't been pushed yet. Check:

```bash
git -C fork-dir branch -r          # List remote branches
git -C fork-dir log --oneline -3   # Check latest local commits
git -C fork-dir push origin main   # Push if needed
```

### Version mismatch after switching to stable

The matrix `stable` version may not match what's in `[workspace.dependencies]`.
Run the scraper to find mismatches:

```bash
python3 _scripts/dep-scrape.py --check
```

### Some deps weren't switched

Check if they use a format the switcher handles:

```bash
python3 _scripts/dep-switch.py status   # Shows counts per target level
```

The switcher handles:
- `name = { version = "..." }` (inline)
- `name = { path = "..." }` (inline)
- `name = { git = "...", branch = "..." }` (inline)
- `[dependencies.name]` (expanded table section)
- `[dev-dependencies.name]` (expanded table section)
- `[build-dependencies.name]` (expanded table section)
- `[target.'cfg(...)'.dependencies.name]` (target-specific expanded table)

If a dep uses `workspace = true`, it's managed via `[workspace.dependencies]`
and switched at the `ws-deps` target level -- not at the member level.
