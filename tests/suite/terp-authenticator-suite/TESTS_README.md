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
| `auth_gas_bench.rs` | SVG-mint gas + employment. Needs `--features gas-bench` and rustc 1.95. Writes `bench/auth_gas_bench.json` |
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

# Authenticator gas + employment (rustc 1.95; Wasmer). Overwrites bench/auth_gas_bench.json.
rustup run 1.95.0 cargo test -p terp-authenticator-suite --features gas-bench --test auth_gas_bench -- --nocapture
```

`bench/auth_gas_bench.json` (`schema` `terp-auth-gas-bench/v1`) is the file to diff over time. cw-orch Mock does not meter gas. Each row's `auth_gas_*` is a `cosmwasm-vm` sudo delta on `crates/terp-rs/artifacts/terp_*.wasm`. The signed action is a cw721-svg `UpdateExtension` mint. `mint_gas_*` is filled when `cw721_svg.wasm` is present (guest rustc 1.91); otherwise the mint still runs on Mock and those fields stay null.

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
