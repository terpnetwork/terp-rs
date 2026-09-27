---
name: crate-tests
description: >-
  How to build, test, and gate changes in the terp-rs crate workspace. Use
  when running or fixing tests, choosing a test tier, deciding which features or
  env gates a test needs, adding a test, or when a cargo command fails to resolve
  the workspace. Covers the four independent cargo workspaces, the
  `tests/suite/` layout rule, dense packing, and the `#[ignore]` env-gate
  convention. Do not use for terp-core chain or Go tests.
---

# Crate tests (terp-rs)

`crates/terp-rs` is not one cargo workspace. It is a root workspace plus four
nested projects that each own their own `[workspace]`. Picking the wrong one is
the most common way to waste ten minutes on a path-dependency error.

## Pick the project first

| You are touching | Cargo root | How to invoke |
|---|---|---|
| Root workspace members (`crates/*`, most of `contracts/`, `proto/`, `tests/`, `tools/gen-tools`) | repo root | `cargo test -p <pkg>` |
| HashMerchant sidecar | `tools/hash-market` | `cargo test --manifest-path tools/hash-market/Cargo.toml -p <pkg>` |
| Onboard authz / ITL planes | `tools/onboard-authz` | `cargo test --manifest-path tools/onboard-authz/Cargo.toml` |
| zkjwt in-circuit prover | `crates/wasm-bindgen/zkjwt-wasm` | `cargo test --manifest-path crates/wasm-bindgen/zkjwt-wasm/Cargo.toml` |
| norick / snap-n-pull wasm | `crates/wasm-bindgen/{norick-wasm,snap-n-pull-rs}` | `--manifest-path`, each is its own workspace |

`tools/hash-market` is listed in the root `exclude` list. Passing
`cargo test -p hash-market-server` from the repo root will not find it.

`zkjwt-wasm` needs **nightly** — it opens with `#![feature(generic_const_exprs)]`.
`norick-wasm` is stable. MSRV 1.86 throughout; the root workspace is edition 2024.

## Test tiers

Pick the cheapest tier that can actually fail your change. Tiers are ordered by
cost, and the higher ones are all `#[ignore]`d by default so a plain
`cargo test` never needs Docker or a mnemonic.

| Tier | Needs | Command | Use for |
|---|---|---|---|
| 0 offline | nothing | `just test-rs` | the default gate. Pure logic, multi-test, goldens. |
| 1 dense packs | nothing | `just ci-core` | what CI actually runs. Packs many `-p` into one rustc graph. |
| 2 contracts | nothing | `just ci-extended` | contract `--lib` tests + `wasm32-unknown-unknown` checks. |
| 3 heavy | nothing, slow | `just ci-heavy` | full `cargo check --workspace --tests` + zk contract libs. |
| 4 ignored | Docker | `just scripts-ibc-harness` | IBC multihop authenticity against real relays. |
| 5 opt-in | Docker + env | see the env table | anything spawning a chain, a sidecar, or a signer. |

Run tier 0 and tier 1 before proposing a change. Tier 1 is what the merge gate
runs, so a change that passes only tier 0 has not really been gated.

## The `#[ignore]` env-gate convention

Every test that needs something external carries the gate **in its ignore
reason**, so the reason is the documentation:

```rust
#[ignore = "ICT_FROST=1 — docker chain + cw-orch Daemon + frost sidecar"]
```

Read the reason, set the named env var, run with `--ignored`. Common gates:

| Gate | Provides |
|---|---|
| `ICT_SPAWN=1` | ict-rs spawns a real Terp chain |
| `ICT_LCD=<url>` | a live LCD, e.g. `https://testnet-api.terp.network` for `120u-1` |
| `ICT_FROST=1` | docker chain + cw-orch Daemon + frost sidecar |
| `ICT_HASHMERCHANT=1` | docker `local-zk` + a prebuilt `hash-market-server` binary |
| `ZAKURA_CROSSLINK_LAB=1` | `zakurad` for the Crosslink light-client test |
| `ITL_MOCK_CASHAPP=1` | mock Cash App sink, avoids a wasmer host |
| `ITL_FROST_PERSIST_KEY` (>=16 chars) | disk-seals FROST inbox jobs |
| `ONBOARD_BROADCAST=1` | actually submits; **inert without terpd** |

To run one:

```sh
ICT_FROST=1 cargo test -p terp-scripts --test frost_privval_e2e -- --ignored --nocapture
```

Two gates that are **not** env vars:

- `ZAKURA_CROSSLINK_LAB` test is also behind `#![cfg(all(feature = "crosslink-e2e", feature = "zakura-live"))]`. Without both features it compiles to nothing and passes vacuously — a green run proves nothing.
- `terp-ipa-params` has a default test covering k=1..8, plus a second test
  `#[ignore]`d because it writes k=1..=20 (~64 MiB of params).

## Feature flags that gate whole test files

`tests/` defaults to `["docker", "nostr", "hash-market"]`. Some test files
compile to nothing without an extra feature, so a green run can be empty:

| Feature | Unlocks |
|---|---|
| `auth-suite` | `tests/tests/vsck_vote_sdk.rs` (the VSCK fail-closed matrix) |
| `auth-suite-full` | the full authenticator suite via `terp-authenticator-suite` |
| `crosslink-e2e` | `tests/tests/crosslink_light_client.rs` |
| `zakura-live` | `tests/tests/zakura_crosslink_live_lc.rs` |

```sh
# vsck_vote_sdk is #![cfg(feature = "auth-suite")]; without it the bin is empty
cargo test -p terp-scripts --no-default-features --features auth-suite --test vsck_vote_sdk
```

`hashmerchant_planes.rs` is **not** cfg-gated; it needs the in-process Axum
host from the default `hash-market` feature, so a default `cargo test` covers it:

```sh
cargo test -p terp-scripts --test hashmerchant_planes
```

Note `crosslink-e2e` is deliberately off by default because it needs protoc,
and leaving it on would break `--test no_rick`.

## `tests/suite/` holds harnesses, not contracts

`tests/suite/` is for cw-orch / Mock / ict-rs deploy bags. On-chain wasm
packages stay under `contracts/`. Per `tests/suite/terp-orch/src/lib.rs`: do not
add more sibling packages here — add a module to `terp-orch` instead.

| Package | Purpose |
|---|---|
| `terp-orch` | the unified suite; the intended home for new suite code |
| `terp-authenticator-suite` | halo2-heavy Mock-sudo matrix, separate because it pulls `terp-zkjwt` |
| `terp-ipa-params` | pure data gen: Vesta IPA `Params::new(k)` for wasmvm pinning |

The authenticator contracts are consumed with `features = ["library"]` so the
rlib links without duplicate `#[no_mangle]` entry points. A contract that is
newly moved into a suite needs that feature and
`#[cfg_attr(not(feature = "library"), entry_point)]` on its exports.

## Two places a test can live

- **Inline** `#[cfg(test)] mod tests` in `src/`. Preferred for pure logic and
  for negative matrices that need crate-internal access. `terp-vsck` keeps
  `src/sudo_matrix.rs` this way, gated `#[cfg(test)] mod sudo_matrix;`.
- **Integration bin** in `tests/`. Use when the test must see the crate the way
  a consumer does, or when it needs a feature combination.

## Dense packing

CI does not run one process per package. `scripts/ci/dense-packs.sh` packs many
`-p` into a single cargo invocation, because per-package setup and cache
overhead dominate tiny libs. To reproduce a single CI cell:

```sh
just ci-dense-list                # show pack names and membership
just ci-core-matrix core-libs-a   # run one pack
```

When adding a package, add it to a pack in `scripts/ci/dense-packs.sh`. A
package in no pack is silently untested by CI.

## When cargo cannot resolve the workspace

```
failed to load manifest for workspace member .../tests
Caused by: failed to read `.../crates/headstash/circuit/Cargo.toml`
```

`cargo metadata`, `check`, `build`, `test` and `run` **all** fail this way,
including for packages that do not themselves need headstash — it is a manifest
load failure, not a compile error. The cause is a path rename in the
`crates/headstash` sibling repo, not here. See
[docs/workspace-status.md](../../docs/workspace-status.md).

The nested projects still build while the root workspace is down, so use them
to keep making progress:

```sh
cargo check --manifest-path tools/onboard-authz/Cargo.toml --all-targets
cargo check --manifest-path contracts/hash-merchant/marketplace-mirror/Cargo.toml --all-targets
```

## Do not

- Run `cargo test` at the repo root expecting Docker tests to run. They are
  `#[ignore]`d, and that is correct.
- Treat a green `--lib` run as proof a feature combination compiles. That is
  what tier 2 (`ci-extended`) and tier 3 (`ci-heavy`) are for.
- Treat a vacuously-compiled test file as passing. Check the cfg gate first.
- Add a package without adding it to a dense pack.
- Update the IBC goldens silently. They are labelled audited-vs-synthetic;
  re-labelling one changes what the pipeline claims to have observed.
- Put a cw-orch deploy bag in `contracts/`, or a contract in `tests/suite/`.
