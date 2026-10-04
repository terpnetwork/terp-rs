# terp-orch suites

cw-orch / Mock / ict-rs deploy bags live here — **not** CosmWasm contract packages.

| Crate | Role |
|-------|------|
| **`terp-orch`** | Unified suite: Path A circuits, private-bridge, optional auth/ict |
| Headstash Product A claim | `cd crates/headstash && just demo-claim-clearnet` — fresh Halo2 prove + `proof_instance_verify` + clearnet `uterp` (not DEX). Metrics: `artifacts/headstash_claim_metrics.json`. |
| `terp-authenticator-suite` | Halo2-heavy authenticator Mock sudo (re-export via `terp-orch --features auth`) |

```text
cargo test -p terp-orch --test path_a_demos
cargo test -p terp-orch --test bridge_lab
cargo test -p terp-orch --features stack --test artifact_stack
```

`stack` composes the authenticator suite, the verify demo, private-dex, the account contracts, and the light clients, then uploads `crates/terp-rs/artifacts/*.wasm`. `cw-ics08-wasm-zap1` stays off the Rust dependency graph (crates.io cosmwasm-std 1.5); the live test still uploads its wasm.

Account billboards and cw-infuser live in this workspace. Their cw-orch bags are opt-in:

```text
cargo test -p terp-orch --features billboards
cargo test -p terp-orch --features infuser
# deploy binary also links shitstrap: --features shitstrap
cargo test -p cw-infuser-suite
cargo test -p terp-account-scripts
```

Live chain (local image, not the running validator):

```text
TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 \
  cargo test -p terp-orch --features stack,ict --test artifact_stack live -- --ignored --nocapture
```

ict-rs: `terp-orch --features ict` + `terp_orch::ict::daemon_builder`.

On-chain code stays under `contracts/`.
