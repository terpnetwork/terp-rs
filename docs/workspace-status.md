# Workspace status

Snapshot of whether `crates/terp-rs` builds and tests, and what stands between
it and a green `just ci-core`. Recorded rather than fixed — every item below is
a finding, not a plan.

Re-check the path graph at any time:

```sh
scripts/ci/check-manifest-paths.sh .   # 0 = every path dep resolves
just --list                             # recipes exist and parse
```

## Headline

**The root cargo workspace does not load.** `cargo metadata`, `check`, `build`,
`test` and `run` all fail identically, for every member, including packages
that have nothing to do with the missing crate:

```
failed to load manifest for workspace member .../tests
Caused by: failed to read `.../crates/headstash/circuit/Cargo.toml`
No such file or directory
```

This is a manifest-load failure, not a compile error. Nothing in the root
workspace can be built or tested until it is resolved, so no statement about
whether the Rust compiles is available yet. Treat every crate under
`crates/`, `contracts/`, `tests/` and `proto/` as **unverified**, not as
passing or failing.

## Cause: a rename in the sibling `crates/headstash` repo

`crates/headstash` moved its circuit crates:

| Was | Is now |
|-----|--------|
| `circuit/` | `circuits/halo2/` |
| `claim-flock/` | `circuits/claim-flock/` |

`crates/headstash/Cargo.toml` still lists `"circuit"` and `"claim-flock"` as
workspace members, so **that repo's own workspace is broken too**:

```
$ cargo metadata --no-deps   # in crates/headstash
failed to load manifest for workspace member .../headstash/circuit
```

The fix is not in this repository. `crates/headstash` needs its members list
updated to `circuits/claim-flock` and `circuits/halo2`; once that lands, the
two path deps below resolve and the root workspace loads again.

## Broken path dependencies

`scripts/ci/check-manifest-paths.sh` finds 18 unresolvable `path =` entries out
of 171 checked. Grouped by what each one blocks:

### 1. Blocks the root workspace (2)

| Manifest | Points at |
|---|---|
| `tests/suite/terp-orch/Cargo.toml:48` | `../../../../headstash/circuit` |
| `crates/wasm-bindgen/snap-n-pull-rs/Cargo.toml:49` | `../../../headstash/circuit` |

Both are the optional `zk-headstash` dep. `tests/suite/terp-orch` reaches it
through the `headstash` feature, and `tests/` reaches `terp-orch` through
`tests/Cargo.toml`, so an unresolvable optional dep still fails the load.

### 2. Blocks `scripts/ibc-ops/cw-orch-ibc` only (15)

Every monorepo-relative path in this nested project is off by one level. It
lives at `scripts/ibc-ops/cw-orch-ibc/`, so reaching the terp-core siblings
needs four `../`, not three:

| Declared | Should be |
|---|---|
| `../../../crates/cosmwasm/packages/std` | `../../../../cosmwasm/packages/std` |
| `../../../crates/cw-minus/packages/cw-utils` | `../../../../cw-minus/packages/cw-utils` |
| `../../../crates/cw-plus/packages/cw4` | `../../../../cw-plus/packages/cw4` |
| `../../../crates/dao-contracts/...` | `../../../../dao-contracts/...` |
| `../../../crates/cw-multi-test-fork` | `../../../../cw-multi-test-fork` |

`cosmwasm-std` appears twice (`[dependencies]` and `[dev-dependencies]`). This
is its own `[workspace]`, so it does not affect the root workspace — it just
cannot be built.

### 3. Blocks `tools/gen-tools/gen-api` only (1)

`tools/gen-tools/gen-api/Cargo.toml:12` declares
`gen-tools = { path = "../../tools/gen-tools" }`, which resolves to
`tools/tools/gen-tools`. From that directory the correct path is
`../..`. This is a stray `tools/` segment, not a depth problem.

## What does build

Nested projects own their own `[workspace]`, so they are unaffected by the
root failure. Verified on this snapshot:

| Project | Command | Result |
|---|---|---|
| `tools/onboard-authz` | `cargo check --all-targets` | clean, warnings only |
| `contracts/hash-merchant/marketplace-mirror` | `cargo check --all-targets` | clean, no warnings |

Nested projects not yet verified, because they are large or need nightly:

| Project | Note |
|---|---|
| `tools/hash-market` | largest; excluded from the root workspace on purpose |
| `crates/wasm-bindgen/zkjwt-wasm` | needs nightly (`generic_const_exprs`) |
| `crates/wasm-bindgen/norick-wasm` | stable |
| `contracts/hash-merchant/trusted-builder-sbt` | own workspace |

## Protos are ahead of the generated Rust

`proto/terp` and `proto/osmosis` are vendored from terp-core and now match it
byte for byte (`just check-protos` is green). `crates/sdk/src/gen/` does **not**
match them. Running `just proto-gen` today would change the SDK's public types.

Deltas, `proto/` versus `crates/sdk/src/gen/`:

- `terp.hashmerchant.v1` gains `OracleSourceKind`, `CustodyAuthenticator`,
  `OracleSource`, `OracleAttestation`; `RegisteredChain.oracle_sources` (6),
  `RootRound.attestations` (7), `HashPairTicket.attestations` (8), and
  `Params.contract_gas_limit` (6). `go_package` moves v5 to v6.
- `terp.drip.v1.Query` gains a pagination import.
- `terp.smartaccount.v1beta1` `tx.proto` gains a `gogoproto` import.

`just proto-gen` cannot run: `proto` is a root-workspace member, and the root
workspace does not load. The protos themselves are verified independently —
`protoc` compiles every vendored file with only unused-import warnings — so the
inputs are sound and only the generator is blocked.

`proto/src/main.rs` also post-processes generated files by stripping derives
from a hardcoded list of type names. That list predates the oracle types above,
so regenerating without reviewing it is likely to produce redundant or missing
derives rather than a clean diff.

## Scripts and recipes

`scripts/act/` was renamed to `scripts/ci/`. The rename is now complete: the
`RUNS_DIR` in `scripts/ci/lib.sh`, the `combo` recipes, the two
`bash scripts/ci/ci-combo.sh` jobs in `.github/workflows/combo.yml`, the
`.gitignore` run-log rules, and the `scripts/act/` references in
`docs/ci/`, `docs/superpowers/prompts/ci-act-tester.md` and
`skills/tests.md` all point at the new path, and `ci-combo.sh` / `act-run.sh`
now resolve their repo root correctly from two levels down.

One stale reference is deliberate: `scripts/ci/runs/baseline-wallclock-*.md`
still names `scripts/act/runs/`. Those are dated measurement records of a run
that happened before the rename; rewriting them would falsify the record.

All 36 recipes referenced by `docs/`, `skills/` and the CI workflows resolve,
and `just` parses the merged import list.

## Known-inconsistent documentation

Found by reading, not by a gate. None of these break a build.

- `crates/frost_escrow/src/lib.rs` is a compile-only stub. `FrostLabCommittee::dkg`
  returns 32 zero bytes, and `validate_matches_burn` / `verify_spend_encoded`
  return `Ok(())` unconditionally — the second with a literal
  `// SILENT ERROR` comment. This is pre-existing at `v0.0.2-dev`; the only
  change on this branch is that comment being added. The crate is not a
  workspace member, so nothing in the root graph links it. Worth deciding
  whether a fails-open verifier should exist at all.
- `contracts/smart-accounts/terp-dao/Cargo.toml` uses
  `version = { workspace = true }` but `terp-dao` is not in the root `members`
  list, so it cannot build on its own. `terp-wavs` and `terp-zktls` share this.
- `contracts/smart-accounts/terp-zktls/src/lib.rs` still uses a bare
  `#[entry_point]` on one export while its siblings are
  `#[cfg_attr(not(feature = "library"), entry_point)]`. Part of an incomplete
  sweep.
- `crates/crosslink/light-client/README.md` still documents a `sigverify`
  feature that `Cargo.toml` no longer declares.
- `contracts/smart-accounts/terp-eth/Readme.md` documents an
  `saa::EthPersonalSign` design; the crate now implements `eth_personal_hash()`
  and `recover_eth_address()` inline, with no `saa` dependency.
- `contracts/revenue/README.md` names the private-dex package `private-dex`, but
  `contracts/revenue/private-dex/Cargo.toml` calls it `cw-private-dex`, and that
  crate's own README says `cd crates/headstash` for its build commands. All
  three disagree.
- Three docs still point at the deleted TypeScript snap
  (`tools/snap-n-pull-js/`): `tools/hash-market/docs/headstash.md`,
  `tools/hash-market/docs/architecture.md`, `tools/hash-market/docs/index.mdx`.
  The Rust replacement is `crates/wasm-bindgen/snap-n-pull-rs/`.
- ~15 `docs/private-bridge/` files still name the retired seam crates
  (`private_dex_seams`, `compose_seams`, `bridge_auth_seams`, `seam_note_out`).
  `crates/terp-seams/tests/no_retired_seam_path.rs` only walks `Cargo.toml`
  files, so stale prose passes it.
- Four files still link to `docs/private-bridge/agents/ROUND*.md`, which moved
  to `agents/demo/`: `E2E-HARNESS-PLAN.md`, `DEMO-CASHAPP-ZEC-CORRIDOR.md`,
  `tests/e2e/private-bridge/README.md`, and
  `agents/connect-private-swap-design-prep-2026-07-22/PROMPT-NOTE-SWAP-SPEND.md`.
- The root `README.md` workspace tree still lists `docs/superpowers/`, which no
  longer exists, and omits `contracts/` and `tests/suite/`.
- `tests/README.md` architecture tree omits `tests/e2e/` and `tests/suite/`.
  The authoritative doc for the latter is `tests/suite/README.md`.
- `tools/onboard-authz/MOVED.md` says the crate "now lives in
  `crates/cw-infuser/tools/onboard-authz`". That path does not exist here, and
  the crate is still referenced by `tools/hash-market/Cargo.toml`,
  `scripts/ci/ci-combo.sh` and the root justfile. Read the file as a pointer
  stub, not as a completed move.

## Untracked binary

`contracts/light-clients/cw-ics08-wasm-crosslink/artifacts/cw_ics08_wasm_crosslink.wasm`
is tracked from before this branch, but `.gitignore` now ignores
`contracts/**/artifacts/` on the grounds that `crates/terp-rs/artifacts/` is the
canonical store (`contracts/light-clients/README.md` says the same). Gitignore
does not apply to already-tracked files, so the old copy is still committed. It
should be `git rm --cached` in a follow-up. The canonical `artifacts/` at the
repo root, and `crates/wasm-bindgen/zkjwt-wasm/pkg/`, are tracked on purpose.
