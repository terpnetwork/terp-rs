# Headstash inventory

Updated 2026-10-12. The runnable suite is on groot-wan (`groot.local`), at the same paths the laptop checkout does not have.

## Where the code is

Laptop `~/abstract/terp-core` has no `.git`, no `crates/headstash`, and `crates/ict-rs` is docs only. `crates/terp-rs/tests/suite/terp-orch` still points at a missing `../../../../headstash` and its `InstantiateMsg` still has `claim_mock_verify`. That laptop orch is not the live API.

groot-wan:

- `~/abstract/terp-core/crates/headstash` — contracts, circuits, artifacts, justfile
- `~/abstract/terp-core/crates/ict-rs` — Docker chain runtime and examples
- Claim page: `websites/dao-dao-ui/apps/dapp/pages/headstash.tsx` renders `HeadstashClaimPage` from `packages/stateful/components/pages/HeadstashClaim.tsx`
- Product notes: `crates/headstash/docs/circuit/` (`FE-CLAIM-SPEC`, `CLAIM-WORKFLOW`, `HEADSTASH-UI-HANDOFF-2026-10-05`, `PRODUCT-A-CW-ORCH-ICTRS-USAGE`, `AIRDROP-OPTIONS`)

Named Headstash herdr panes from the brief were not in the session. This inventory is from the tree.

## Contracts

| Role | Crate | Wasm on disk | What it actually does |
| --- | --- | --- | --- |
| Claim and escrow | `contracts/cw-headstash` | `artifacts/cw_headstash.wasm` (2026-10-05, 844 KiB). Strings include `proof_instance_verify` and the personal-sign challenge error. | `ProcessHeadstash`. ExistingFungible bank-sends. NewFungible mint messages are commented out. |
| Witness | `contracts/cw-reg-eligibility` | `contracts/cw-reg-eligibility/artifacts/cw_reg_eligibility.wasm` | `VerifyMinHolders` returns `ok: proof == accepted_proof`. Lab byte compare, not a Halo2 witness. |
| Airdrop factory | `contracts/manifold` | no wasm | Not deployed. |
| Circuit blob | `artifacts/headstash_vk.bin` | 17,877,323 bytes | Footer matches Path A: prover 0, curve 0, k 18, param 16,777,292, cs 17,410, vk 1,082,541. |

Older wasm also exists (`contracts/cw-headstash/artifacts/cw_headstash.wasm` Jul 22, `zk_headstash.wasm` Dec 2025). The harness uses the Oct 5 artifact.

## Claim checks, in order

`process_headstash` (`contracts/cw-headstash/src/headstash.rs`):

1. Registered root must equal `i.anchor` (`claim anchor does not match registered root_id`).
2. Poseidon-v1 domain when the contract was instantiated that way.
3. `i.e` must equal `challenge_scalar_be(keccak256(Instance::to_bytes()[..168]))`.
4. Batch nullifier uniqueness, then storage nullifier.
5. Denom bytes must equal `TokenStrategy::proof_representation()` (`blake3(raw)` with byte 31 masked `0x1F`).
6. With the `zk-api` feature, `circuit_id == 0` errors, otherwise `proof_instance_verify`. Without the feature, the contract errors that zk-api is required.
7. Only then: `rr` must equal `i.recp`.

There is no expiry field. Instantiate rejects an empty genesis root and an empty WAVS set. WAVS proof-of-possession uses `api.bls12_381_hash_to_g2` plus a pairing. The comment says the message binds the contract address. The code hashes the public key only.

Instance wire is 200 bytes: anchor, denom field, value `u64` le, nullifier, recipient, commitment, challenge. The challenge covers the first 168 bytes.

## ict-rs

- `examples/headstash.rs` is a single-chain scaffold. `HEADSTASH_CLAIM_MODE=mock` used to return `Ok(())`. That success return is removed. `real` still returns `real_claim_not_wired`. It still sends `claim_mock_verify`, which `InstantiateMsg` no longer has.
- `examples/ibc_transfer.rs` is the two-chain Hermes pattern the new example follows.
- `examples/headstash_local_e2e.rs` is the harness added for this run.
- Host port publish was hardcoded `0.0.0.0` in `ict-rs/src/runtime/docker.rs`. `ICT_PUBLISH_HOST` overrides it. Default remains `0.0.0.0` for other suites.
- Image identity is `TERP_IMAGE_REPO` / `TERP_IMAGE_VERSION`. Tags `local` and `local-zk` panic. Hermes image is `HERMES_IMAGE_REPO` / `HERMES_IMAGE_VERSION`, default `ghcr.io/informalsystems/hermes:1.13.1`.
- Penumbra chain module is a stub. `ICT_MOCK=1` is a mock path in other examples. This harness refuses it.

## Local images already on groot

- `registry.terp.network/terp-core:v6.4.0` (terpd 6.4.0, `proof_instance_*` strings present)
- `ghcr.io/informalsystems/hermes:1.13.1`
- Docker client `/usr/local/bin/docker`, socket `unix:///Users/returniflost/.docker/run/docker.sock`. Non-interactive SSH does not put `/usr/local/bin` on `PATH`.

## Claim page

`HeadstashClaimPage` queries LCD `EpochRoot` for algo `headstash-accepted`. It does not broadcast and does not build `ProcessHeadstash`. Chain select is 120u-1 and morocco-1. `?lab=1` reads `http://127.0.0.1:19820/endpoints.json` and queries that loopback LCD. The harness also serves `crates/headstash/e2e/claim/index.html` on `127.0.0.1:19820`.

## Not on disk

- Manifold wasm
- A compiled hash-market binary (PIR routes exist in source and return 404 unless built with `--features pir`)
- A compiled `zk-headstash` prover binary in the headstash target dir at inventory time
- Laptop Docker

## Mocks that are not a claim

1. `examples/headstash.rs` mock `Ok(())` — removed, now `Err(skip_err("mock_removed"))`.
2. `claim_mock_verify: true` in that old example's instantiate JSON — field is gone from `InstantiateMsg`. The new harness does not send it.
3. Zero genesis root — rejected by the contract. The harness uses 32 bytes of `0x11`.
4. `ICT_MOCK=1` — the new harness exits before starting chains.
5. `cw-reg-eligibility` byte equality — deployed, and a mismatch must return `ok: false`. A match is not recorded as a claim.
6. `BridgeCfg.mock_verify` / `BridgeMintNote` — a different product. Not enabled on this path.
