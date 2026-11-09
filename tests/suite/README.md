# terp-orch suites

cw-orch / Mock / ict-rs deploy bags live here — **not** CosmWasm contract packages.

| Crate | Role |
|-------|------|
| **`terp-orch`** | Unified suite: Path A circuits, private-bridge, optional auth/ict |
| Headstash Product A claim | `cd crates/headstash && just demo-claim-clearnet` — fresh Halo2 prove + `proof_instance_verify` + clearnet `uterp` (not DEX). Metrics: `artifacts/headstash_claim_metrics.json`. |
| `terp-authenticator-suite` | Halo2-heavy authenticator Mock sudo (re-export via `terp-orch --features auth`) |
| `dao-foundation-suite` | Nested subDAOs on `dao-testing`'s `DaoDaoSuite`, plus the native-delegation and quorum bins, plus the ignored terpd instantiate2 test. |

```text
cargo test -p terp-orch --test path_a_demos
cargo test -p terp-orch --test bridge_lab
cargo test -p terp-orch --features stack --test artifact_stack
```

`stack` is `TerpArtifactStack`: authenticators, the verify demo, private-dex, account contracts, light clients, product contracts, the infuser, and the terp721 billboards. It uploads `crates/terp-rs/artifacts/*.wasm`.

Contract end-to-end cases live in `<contract>/e2e` and construct that suite. `cargo test -p cw-ave-e2e` is the ave case. A field check for every other contract is `<contract>-e2e`.

```text
cargo test -p terp-orch --features stack --test manifold_fields
cargo test -p cw-ave-e2e
```

Live chain (local image, not the running validator):

```text
TERP_IMAGE_REPO TERP_IMAGE_VERSION \
  cargo test -p terp-orch --features stack,ict --test artifact_stack live -- --ignored --nocapture
```

ict-rs: `terp-orch --features ict` + `terp_orch::ict::daemon_builder`.

DAO instantiate2 on a real terpd (optimizer wasm in `dao-contracts/artifacts/init2/`, not the cw-orch mock):

```text
TERP_IMAGE_REPO TERP_IMAGE_VERSION ICT_DAO_INIT2=1 \
  cargo test -p dao-foundation-suite --test dao_init2_chain -- --ignored --nocapture
```

On-chain code stays under `contracts/`.
