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
```

ict-rs: `terp-orch --features ict` + `terp_orch::ict::daemon_builder`.

On-chain code stays under `contracts/`.
