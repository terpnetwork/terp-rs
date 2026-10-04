# Workspace status

Whether `crates/terp-rs` builds and tests, and what stands between it and a
green `just ci-core`.

Re-check at any time:

```sh
scripts/ci/check-manifest-paths.sh .   # 0 = every path dep resolves
cargo metadata --no-deps               # 0 = the workspace loads
just --list                             # recipes exist and parse
```

## Headline

**The root workspace loads, and one third-party crate blocks a full compile.**

`cargo metadata`, `check`, `build`, `test` and `run` all work. Every path
dependency resolves: 172 of 172.

What does not compile is `frost-rerandomized 2.2.0`, a published crates.io
dependency, and nothing in this repository is the cause:

```
error[E0614]: type `u16` cannot be dereferenced
  --> ~/.cargo/registry/.../frost-rerandomized-2.2.0/src/lib.rs:84:13
   |
84 |             *self.min_signers(),
   |             ^^^^^^^^^^^^^^^^^^^ can't be dereferenced
```

That crate declares `frost-core = "2.2.0"`, and it does resolve `frost-core
2.2.0`, but the call site expects a `&u16` accessor that `frost-core 2.2.0`'s
`#[derive(Getters)]` on `KeyPackage` does not produce. `frost-core 3.0.0`
replaced it with a hand-written `fn min_signers(&self) -> u16` returning by
value, which is the signature the error reports.

Consequence: `cargo check --workspace --tests` fails, so CI tier 2 and tier 3 —
`just ci-extended` and `just ci-heavy` — cannot run. Tiers 0 and 1 work for
every package not in the `frost-ed25519` subgraph.

The `frost-ed25519 2.2` requirement appears in four manifests across two repos:

| Manifest | Requirement |
|---|---|
| `tools/hash-market/Cargo.toml:116` | `2.2`, optional, behind the `frost` feature |
| `tests/Cargo.toml:111` | `2.2` |
| `crates/frost_escrow/Cargo.toml:15` | `2.1`, optional, not a workspace member |
| `crates/kms-shard/Cargo.toml:19` (outside this repo) | `2.2` |

Moving to the 3.x line aligns with `frost-core 3.0.0` and `osst`, which is
already on 3.x. That is a cryptographic dependency major bump across two repos,
so it is a decision, not a fix, and it has not been made here.

Note also that `cargo check -p terp-scripts --lib` fails separately in
`zakura-halo2-proofs` with two `E0599`s. That configuration excludes
dev-dependencies and so resolves a different feature set from
`--workspace --tests`, which does not hit it. Worth understanding before
trusting a single-package `--lib` check as representative.

## What is verified

| Scope | Command | Result |
|---|---|---|
| Manifest graph | `check-manifest-paths.sh` | 172 of 172 resolve |
| Workspace load | `cargo metadata --no-deps` | exit 0 |
| `terp-frost` | `cargo check -p terp-frost --all-targets` | clean, warnings only |
| `terp-frost` tests | `cargo test -p terp-frost` | **13 passed, 0 failed** |
| `tools/onboard-authz` | `cargo check --all-targets` | clean, warnings only |
| `marketplace-mirror` | `cargo check --all-targets` | clean, no warnings |
| Whole workspace | `cargo check --workspace --tests` | 1 error, in `frost-rerandomized` |

`terp-frost`'s 13 include the cases that matter for the Schnorr work: 2-of-3
authorizes while one share does not, a rotation switches the authorizing key
while a pending one does not, an opaque `group_vk` fails auth and still rotates,
`OnAuthRemoved` does not clear `group_vk`, and `simulate` does not write.

Nested projects not yet verified, because they are large or need nightly:

| Project | Note |
|---|---|
| `tools/hash-market` | excluded from the root workspace on purpose; in the `frost` subgraph, so blocked by the above |
| `crates/wasm-bindgen/zkjwt-wasm` | needs nightly (`generic_const_exprs`) |
| `crates/wasm-bindgen/norick-wasm` | stable |
| `contracts/hash-merchant/trusted-builder-sbt` | own workspace |

## The path-dependency fixes

All four classes were directory moves whose relative paths did not follow. The
`terp-rs` side is committed; the `crates/headstash` side is in that repo's
working tree, uncommitted, and must be committed together with the `circuits/`
move it belongs to or headstash will not load from a fresh clone.

| Class | Was | Now |
|---|---|---|
| headstash circuit move | `headstash/circuit` | `headstash/circuits/halo2` |
| `snap-n-pull-rs` depth | `../../../headstash/...` | `../../../../headstash/...` |
| `cw-orch-ibc` depth | `../../../crates/X` | `../../../../X` (15 entries) |
| `gen-api` stray segment | `../../tools/gen-tools` | `../..` |

## The KMS handoff is not committed

`reviews/KMS-PRODUCT-HANDOFF-2026-09-27.md` is the cold-start artifact for the
threshold-KMS effort, and it is **not in git**. `reviews` has been in
`.gitignore` since before this branch, and 0 of the 16 handoffs in that
directory have ever been committed.

This now matters more than it did, because
`crates/cw-orchestrator/docs/src/integrations/kms-signer.md` — which *is*
tracked, in another repo — points at that untracked path. The reference 404s for
anyone who clones.

Left as-is deliberately, per the repo's existing convention. Making it durable
is a one-line change once someone decides between force-adding the single file,
un-ignoring the directory, or relocating it next to its stated source of truth
in `cw-orchestrator/docs/src/integrations/`.

## Protos are ahead of the generated Rust

`proto/terp` and `proto/osmosis` are vendored from terp-core and match it byte
for byte (`just check-protos` is green). `crates/sdk/src/gen/` does **not**
match them. Running `just proto-gen` today would change the SDK's public types.

Deltas, `proto/` versus `crates/sdk/src/gen/`:

- `terp.hashmerchant.v1` gains `OracleSourceKind`, `CustodyAuthenticator`,
  `OracleSource`, `OracleAttestation`; `RegisteredChain.oracle_sources` (6),
  `RootRound.attestations` (7), `HashPairTicket.attestations` (8), and
  `Params.contract_gas_limit` (6). `go_package` moves v5 to v6.
- `terp.drip.v1.Query` gains a pagination import.
- `terp.smartaccount.v1beta1` `tx.proto` gains a `gogoproto` import.

`just proto-gen` can now run, since the workspace loads. The protos themselves
verify independently — `protoc` compiles every vendored file with only
unused-import warnings, and all five terp packages are in the descriptor set —
so the inputs are sound.

`proto/src/main.rs` also post-processes generated files by stripping derives
from a hardcoded list of type names. That list predates the oracle types above,
so regenerating without reviewing it is likely to produce redundant or missing
derives rather than a clean diff.

## Scripts and recipes

The `scripts/act/` to `scripts/ci/` rename is complete: `RUNS_DIR` in
`scripts/ci/lib.sh`, the `combo` recipes, the two
`bash scripts/ci/ci-combo.sh` jobs in `.github/workflows/combo.yml`, the
`.gitignore` run-log rules, and the `scripts/act/` references in `docs/ci/`,
`docs/superpowers/prompts/ci-act-tester.md` and `skills/tests.md` all point at
the new path, and `ci-combo.sh` / `act-run.sh` resolve their repo root correctly
from two levels down.

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
  `// SILENT ERROR` comment. It is not a workspace member. Worth deciding
  whether a fails-open verifier should exist at all.
- `terp-dao`, `terp-bls`, and `terp-zktls` are workspace members.
  `terp-wavs` was renamed to `terp-bls`.
- `contracts/smart-accounts/terp-zktls/src/lib.rs` still uses a bare
  `#[entry_point]` on one export while its siblings are
  `#[cfg_attr(not(feature = "library"), entry_point)]`.
- `crates/crosslink/light-client/README.md` still documents a `sigverify`
  feature that `Cargo.toml` no longer declares.
- `contracts/smart-accounts/terp-eth/Readme.md` documents an
  `saa::EthPersonalSign` design; the crate now implements `eth_personal_hash()`
  and `recover_eth_address()` inline, with no `saa` dependency.
- `contracts/revenue/README.md` names the private-dex package `private-dex`, but
  `contracts/revenue/private-dex/Cargo.toml` calls it `cw-private-dex`, and that
  crate's own README says `cd crates/headstash` for its build commands.
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
- `tests/README.md` architecture tree omits `tests/e2e/` and `tests/suite/`,
  and its feature table says `default = docker, nostr` where `tests/Cargo.toml`
  says `["docker", "nostr", "hash-market"]`. The manifest is right.
- `tools/onboard-authz/MOVED.md` says the crate "now lives in
  `crates/cw-infuser/tools/onboard-authz`". That path does not exist here, and
  the crate is still referenced by `tools/hash-market/Cargo.toml`,
  `scripts/ci/ci-combo.sh` and the root justfile. Read it as a pointer stub.
- `crates/terp-seams/src/dex/note/../README.md` and ~14 other docs still point
  at `docs/plans/spectrum/fixtures/private_dex_seams` for the seam math SSOT;
  the source of truth is now `crates/terp-seams`.

## Untracked binary

`contracts/light-clients/cw-ics08-wasm-crosslink/artifacts/cw_ics08_wasm_crosslink.wasm`
is tracked from before this branch, but `.gitignore` now ignores
`contracts/**/artifacts/` on the grounds that `crates/terp-rs/artifacts/` is the
canonical store (`contracts/light-clients/README.md` says the same). Gitignore
does not apply to already-tracked files, so the old copy is still committed. It
should be `git rm --cached` in a follow-up. The canonical `artifacts/` at the
repo root, and `crates/wasm-bindgen/zkjwt-wasm/pkg/`, are tracked on purpose.

`delegation_messages.json` has the same problem in reverse: it was deleted as a
committed build artifact, `tests/bin/dels.rs` still names it in `RAW_MSG_JSON`,
and there is no ignore rule, so the next run recreates it untracked.
