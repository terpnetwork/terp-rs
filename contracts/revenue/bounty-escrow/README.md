# terp-bounty-escrow (DEV/TEST ONLY, not audited, not deployed)

Custodial bounty escrow for the Terp bounty board:
Pending -> Open -> Assigned -> Submitted -> Accepted -> Paid (plus Refunded / Cancelled / Expired).
The default `dev-chain-guard` feature refuses to instantiate on `morocco-1`.
Design notes and the chain/backend gap analysis: [DESIGN.md](DESIGN.md).
It is also a **DAO DAO proposal module**: bounties are proposals (`info`, `dao`, `list_proposals`, `reverse_proposals`, `proposal_count`, pre-propose `PreProposeInfo`, proposal hooks). See DESIGN.md §9.
Path: `contracts/revenue/bounty-escrow` (was `contracts/bounties/terp-bounty-escrow`).

## Limits enforced on chain
| Field | Limit | Why |
|---|---|---|
| `summary` | 1..=280 bytes UTF-8, no control chars | Stored in every `Bounty` and re-emitted by queries. Measured on a local test chain: about 43 gas per byte (+12k gas from 1 to 280 bytes). Long text lives off chain. |
| `details_hash` / `submission_hash` | 64 lowercase hex (sha256) | Commits to the canonical JSON doc off chain |
| `details_cid` / submission `cid` | IPFS CIDv0 (`Qm…`, base58btc, 46 chars) or CIDv1 base32 (`b…`), ≤ 128 chars, charset and length checked | Points to a `terp-bounty-details/v1` or `terp-bounty-submission/v1` doc (`offchain-schemas/`) |
| `min_submit_stake` | governance param (placeholder 100 TERP); `0` disables | The assignee's bonded stake (staking `AllDelegations` in the bond denom) must be ≥ this value to `submit`. See the `can_submit{address}` query. |
| `min_upload_stake` | governance param (placeholder 10 TERP); `0` disables | Not used by any execute path. The off-chain CID storage service calls the `can_upload{address}` query (same bonded-stake check) before it stores a details or submission document. |

## Layout
- `src/`: contract (`contract.rs`, `msg.rs`, `state.rs`, `cid.rs`, `error.rs`) and `interface.rs` (the cw-orch interface, feature `interface`)
- `src/bin/schema.rs`: picked up by `just gen api` (gen-tools) like the other contracts. Output goes under `api/<lang>/terp-bounty-escrow/`.
- `tests/multitest.rs`: cw-multi-test suite (22 tests); `tests/dao_module.rs`: DAO module suite (9 tests); plus 2 CID unit tests in `src/cid.rs`
- `src/dao.rs`: DAO DAO proposal-module queries, pre-propose reply handling and hooks
- `params/escrow.params.json`: PLACEHOLDER governance params; `params/catalog-rewards.json`: catalog reward amounts used to check the caps
- `offchain-schemas/`: the JSON Schemas (draft 2020-12) for the off-chain docs, with examples
- cw-orch suite: `scripts/bounty-escrow` (`bounty-escrow-scripts`), Mock tests in `tests/suite.rs`

```text
cargo test -p terp-bounty-escrow
cargo test -p bounty-escrow-scripts
cargo run -p terp-bounty-escrow --bin schema     # run in this directory -> schema/
cargo build -p terp-bounty-escrow --lib --release --target wasm32-unknown-unknown
```

The amounts are `Uint256`, because `Coin::amount` is `Uint256` in this workspace's cosmwasm-std. They are JSON strings either way.

## cw-orch interface (feature `interface`)
```toml
terp-bounty-escrow = { workspace = true, features = ["interface"] }   # as in scripts/bounty-escrow
# tests/suite/terp-orch registers it under the `stack` feature as TerpProductSuite.bounty_escrow
```
```rust
use terp_bounty_escrow::interface::TerpBountyEscrow;
use terp_bounty_escrow::msg::{ExecuteMsgFns, QueryMsgFns};
let escrow = TerpBountyEscrow::new(chain.clone());
escrow.upload()?;                    // Mock: native wrapper (with reply); daemon: artifacts wasm
escrow.instantiate(&msg, Some(&admin), &[])?;
let n = escrow.proposal_count()?;    // DAO DAO queries through QueryFns
```
`TerpProductSuite::upload()` and `instantiate_lab()` leave it out on purpose (unaudited, dev-chain-guard).
