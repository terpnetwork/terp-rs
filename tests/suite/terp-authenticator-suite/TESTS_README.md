# Authenticator suite tests — location guide

**Package:** `terp-authenticator-suite`  
**Absolute root:** `/Users/returniflost/abstract/terp-core/crates/terp-rs`  
**This crate:** `tests/suite/terp-authenticator-suite/`

## Run tests

```bash
cd /Users/returniflost/abstract/terp-core/crates/terp-rs
# Default: structural zk-jwt + eth personal_sign goldens + claim e2e
cargo test -p terp-authenticator-suite

# zk-host production path smoke (HostZkJwtVerifier / proof_instance_verify wiring)
cargo test -p terp-authenticator-suite --features zk-host --test zk_host
cargo test -p terp-zkjwt --features zk-host,library
```

## Test binaries (this directory’s `tests/`)

| File | Focus |
|------|--------|
| `exhaustive_matrix.rs` | Full AuthSudoMsg matrix + recovery 2/3 + ed25519 wrong msg/key + vsck roles/replay |
| `zk_jwt_inclusion.rs` | inclusion_set_root + **require_registered_claim e2e** |
| `core_authenticators.rs` | core lifecycle + ed25519/eth host goldens |
| `zk_scaffold_sudo.rs` | zk smoke + coexistence |
| `zk_host.rs` | host path + **real snarkjs jwt-auth Authenticate e2e** (`--features zk-host`) |
| `zk_jwt_circom_codec.rs` | L1 circom claim codec golden pyramid (gateway-style soundness) |

### Suggested CI commands

```bash
cargo test -p terp-authenticator-suite
cargo test -p terp-authenticator-suite --features zk-host --test zk_host
cargo test -p terp-zkjwt --lib circom
```

## Suite source

| Path | Role |
|------|------|
| `src/suite.rs` | Deploy all authenticators |
| `src/fixtures.rs` | Dummy AuthSudoMsg request builders |
| `src/traits.rs` | Mock `wasm_sudo` helpers |
| `src/interfaces/*` | cw-orch interfaces |

## Related contracts

- zk-jwt: `../terp-zkjwt/` (`JWT_AUTH_FLOW.md`, inclusion root)
- Team focus: `../../../reviews/ZKJWT-HEADSCALE-FOCUS.md`

## Hermes QA task breadcrumb

Task `t_a507f133` on board `authenticator-suite-review`.  
See also `../../../reviews/HERMES-QA-TASK-LOCATION.txt`.
