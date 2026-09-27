# cw-private-dex

CosmWasm **private swap settle** contract for Terp.

- **Seam math SSOT:** `crates/terp-seams` (`dex` module: `apply_swap_action`,
  constant-product quote, oracle bounds, pool nullifiers).
- **Research docs:** https://docs.terp.network/research/private-dex
- **Contract wiring reference:** in-tree **`crates/dex`** (astroport-core fork) —
  especially `contracts/pair` (`swap`, `compute_swap`, `Simulation`, pause window).
- **Proof API:** CosmWasm fork `deps.api.proof_instance_verify(zkid, proof, instances)`.

## Design map: transparent pair → private settle

| `crates/dex` pair (astroport XYK) | `cw-private-dex` |
|-----------------------------------|------------------|
| `ExecuteMsg::Swap { offer_asset, max_spread, … }` | `SettleSwap { statement, proof }` |
| Bank / CW20 offer transfer in | Proven note spend (nullifiers + proof) |
| `compute_swap` + commission Decimal | `quote_exact_in` with γ/γ_den (seam SSOT) |
| `assert_max_spread` / belief price | `min_out` + optional oracle mid bounds |
| Ask asset `into_msg` transfer out | Emit `cm_out_*` attributes (private notes) |
| `QueryMsg::Simulation` | `QueryMsg::QuoteExactIn` |
| `PoolPaused` / `pool_unpause_at` | `PoolStatus::Paused` / `SetPoolStatus` |
| Factory creates pair | Owner `CreatePool` (factory-lite v0) |

We **extend designs** from the fork (msg shape, settle pipeline, simulation,
pause) rather than depending on the full `astroport` package graph in this guest.

## Dual-path verify (honest)

| Mode | When | Behavior |
|------|------|----------|
| **mock_verify** | `Config.mock_verify = true` **or** `cfg!(test)` | Accept non-empty proof after host seam checks (lab / ict film) |
| **proof_instance_verify** | `mock_verify = false` + feature `zk-api` + chain exports host import | Real host VK verify against registered `zkid` |
| **fail-closed** | production config without `zk-api` build | Reject settle; no silent pass |

Guest default builds **omit** cosmwasm-std `zk` so wasm deploys on stock wasmd
(same floor as `cw-headstash` BridgeMintNote).

## What this is / is not

| Is | Is not |
|----|--------|
| On-chain **settle** of public swap statements | Halo2 swap circuit (still pure seams / future circuit) |
| Public **virtual** pool reserves + pool-spend nullifiers | Full transparent LP / incentives / router (use `crates/dex`) |
| Oracle mids as **bounds only** | Oracle-driven balance mint |
| Astroport-shaped pair lifecycle for private notes | Drop-in replacement for XYK bank swaps |

## Build / test

```bash
cd crates/headstash

# unit tests (mock_verify dual-path; no host import)
cargo test -p cw-private-dex

# guest wasm without proof_instance_verify import
cargo build -p cw-private-dex --target wasm32-unknown-unknown --release --lib

# host-export guest (only for zk wasmvm chains)
cargo build -p cw-private-dex --target wasm32-unknown-unknown --release --lib --features zk-api
```

## Spec alignment

- https://docs.terp.network/research/private-dex/spec
- `crates/terp-seams` (`dex`)
- `crates/dex/contracts/pair` + `packages/astroport/src/pair.rs`
- `crates/cosmwasm/ZK_PROOF_VERIFICATION_ARCHITECTURE.md`
