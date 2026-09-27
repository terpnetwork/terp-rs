# Dependency control plane — stable surface (v1)

Locked-in workflow for local ↔ remote crate swapping across the monorepo.

## Source of truth

| Artifact | Path | Role |
|---|---|---|
| Config | `_devops/dep-config.toml` | `monorepo_root`, policy flags |
| Matrix | `_devops/dependency-matrix.toml` | Desired state per forked crate |
| Mode state | `_devops/.dep-mode` | Last full switch mode |
| Nix repos | `_devops/nix/repos.json` | Generated clone list (from matrix) |
| Flake | `terp-core/crates/flake.nix` | Toolchain + bootstrap apps |
| CLI | `_scripts/dep.py` | **Stable entrypoint** |
| Engine | `_scripts/dep-switch.py` + `dep_common.py` | Implementation |
| Probes | `_scripts/dep_probe.py` | Read-only. `branch-check`, `git-status`, write preflight |

**Monorepo root** defaults to `terp-core/crates` (resolved from config).  
Override with `TERP_DEP_ROOT` / `DEP_MONOREPO_ROOT` if needed.

**Nix complements; it does not replace switch.** Flake clones fork *trees*;
`dep.py switch` still rewrites Cargo.toml. See `_devops/nix/README.md`.

**Rust `gen-tools deps`** is experimental (incomplete parity). Prefer `dep.py` / just recipes below.

## First-time bootstrap (new machine / empty crates/)

```bash
# With Nix (recommended for newcomers)
cd terp-core/crates
nix develop
nix run .#setup              # export + clone all matrix forks + switch local

# Without Nix
export TERP_DEP_ROOT="$PWD"
python3 terp-rs/tools/gen-tools/_scripts/dep.py bootstrap
python3 terp-rs/tools/gen-tools/_scripts/dep.py validate
python3 terp-rs/tools/gen-tools/_scripts/dep.py switch local
```

This avoids hand-syncing 50 git remotes. Day-to-day still uses `switch local|git|dev`.

## Daily commands

```bash
cd crates/terp-rs/tools/gen-tools

# Where am I pointed?
just deps-config          # or: python3 _scripts/dep.py config

# Matrix must be clean before switches
just deps-validate

# Every matrix branch must exist on its remote (read-only, exit 1 otherwise)
just deps-branch-check

# What is in flight, and what blocks a write path (read-only git plumbing)
just deps-git-status

# If validate fails (absolute paths / wrong package dirs):
just deps-heal            # rewrite locals; package-name checked
just deps-heal-dry-run

# Inspect / switch
just deps-status
just deps-local           # path deps for hot rebuilds
just deps-git             # git remotes for push verification
just deps-stable          # crates.io versions
just deps-dev             # git identity + local path patches

# Preview
just deps-switch-dry-run local

# After a switch, prove Cargo agrees (slow — cargo metadata)
just deps-verify local
just deps-switch-verify local
```

## Modes

| Mode | Manifest shape | Use |
|---|---|---|
| `local` / `zk_local` | `path = "..."` in ws-deps/members | Day-to-day editing |
| `git` / `zk_git` | `git = "...", branch = "..."` | Remote verify / push prep |
| `dev` / `zk_dev` | git in ws-deps + `[patch.'url']` → path | Hot swap with remote-shaped deps |
| `stable` | registry versions | Release / downstream fidelity |

ZK is a profile overlay on the same machinery (`zk_*` keys in the matrix).

## Integrity rules (fail-closed)

1. **No absolute paths** in matrix `local` / `zk_local`
2. Path must exist under `monorepo_root`
3. `[package].name` at that path must match matrix `package` (or key)
4. Virtual workspace manifests are not valid package locals
5. `dep.py switch` refuses to run if validate would fail (unless `--force`)
6. **Single identity** for CosmWasm-stack crates (`cosmwasm-std`, `cw-schema`,
   `cw-controllers`, …): `dep.py verify` reports `DUAL-IDENTITY` if the same
   package name resolves from two sources (path + git, or two git branches).


## Branch labels

See **[BRANCHING.md](./BRANCHING.md)**. Default product line:

```text
v3.1.0-zk.0    # CosmWasm 3.1.0-dev line + zk fork sub-version 0
```

Retired: `cw3-base-local-freeze`, `cw3-base-git-freeze`, freeze-as-default `mvp`/`zk-mvp`.

## Dual-identity / version-line drift

Cargo treats these as **different crates** even when the package name matches:

| Source | Example |
|---|---|
| path | `../cosmwasm/packages/std` (e.g. via `cw721-nips`) |
| git branch A | `permissionlessweb/cosmwasm` @ `v3.1.0-zk.0` |
| git branch B | same URL @ stale `mvp` / retired freeze |

Symptoms: `cw-controllers` typed against one CosmWasm tip, workspace against
another → dual `cosmwasm_std` / `cw_schema` compile failures.

**Prevention (monorepo develop shape — matches groot product worktrees):**

- Prefer **path** CosmWasm + path `cw-minus` in consumer workspaces, **or**
- `dep.py switch dev` (git-shaped ws-deps + `[patch.'https://…']` → local path)
- One product branch line monorepo-wide: **`v*-zk.*`** (see BRANCHING.md)
- Fork workspaces: no self-referential `git = own-repo` for internal packages
  (use `path = "packages/…"`)
- Matrix `package` must match real `[package].name` (e.g. `clone-cw-multi-test`
  publishes as `abstract-cw-multi-test`)

After any hybrid switch, regenerate locks so stale multi-branch entries die:

```bash
rm -f Cargo.lock && cargo generate-lockfile
python3 _scripts/dep.py verify --mode dev --workspace dao-contracts
```

**Machine basis:** **groot** product tips (`v*-zk.*`) are source of truth.

## Recommended git workflow

```text
deps validate
deps switch local          # develop
# … commit per-repo as needed …

deps switch git            # or zk_git
deps ensure-branches git   # create missing remote branches
python3 _scripts/dep-graph.py --order   # push order
# push repos in topo order, then:
#   cargo update in consumers
deps verify --mode git
```

## Policy knobs (`dep-config.toml`)

```toml
[policy]
forbid_absolute_paths = true
strict_package_names = true
verify_after_switch = false   # set true to always cargo-metadata after switch
```

## Backup

Pre-heal backup of the matrix (if present):

```text
_devops/dependency-matrix.toml.bak-pre-heal
```

Heal report:

```text
_devops/matrix-heal-report.json
```

## What not to do

- Do not rely on `[patch.crates-io]` alone for cross-workspace fidelity
- Do not run experimental `cargo run -- deps baseline` against a dirty tree without validate
- Do not reintroduce absolute matrix paths
- Do not treat Rust deps CLI as the operator surface until it gains full parity + verify

## Native-build path deps (Go c-shared, etc.)

Some matrix crates compile **native** libraries in `build.rs` (e.g. `osmosis-test-tube` →
`libosmosistesttube` via Go `-buildmode=c-shared`).

When gen-tools switches consumers to **path** deps:

1. Default `cargo check` / unit tests **must not** require those natives.
2. Consumers feature-gate the dep:

```toml
[features]
test-tube = ["dep:osmosis-test-tube"]

[dependencies]
osmosis-test-tube = { workspace = true, optional = true }
```

```rust
#[cfg(all(not(target_arch = "wasm32"), feature = "test-tube"))]
pub mod test_tube;
```

3. Matrix fields:
   - `native_tools = ["go"]` — host tools required for a full native build
   - `consumer_optional_feature = true` — hard non-optional consumers fail validate

4. Full integration: `cargo test -p dao-testing --features test-tube` with Go on `PATH`.

gen-tools **must not** enable `default = ["test-tube"]` when healing/switching.
