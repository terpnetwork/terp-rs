# Origin

- Repo: https://github.com/AtlasDA0/app-contracts
- Commit: `4b921920092004f4a79724e3c0db3f7993b64be7`
- Paths: `contracts/raffles/` (`contract.rs`, `execute.rs`, `msg.rs`, `state.rs`, `query.rs`, `utils.rs`, `randomness/`, `interface.rs`, `examples/schema.rs`)

## What changed

- CosmWasm 1.4 / Stargaze messages replaced with the workspace CosmWasm v3 fork (`Addr`, `Uint256` coins, `cw-storage-plus`, `cw721` from `../cw-nfts`).
- `sg721` transfers removed. Prizes move with `Cw721ExecuteMsg::TransferNft`. The sender must own the token and approve this contract (or set it as operator) before create. `cw721-svg` accepts that message.
- Nois proxy and Drand verification removed. See `contracts/nfts/RANDOMNESS.md`.
- Fee discounts, DAO voting, staking gates, cw20 ticket prices, and merkle whitelists are not ported. They depended on `dao-interface`, `cw20`, and Stargaze queries.
- A sale that ends with zero tickets, or fewer than `min_ticket_number`, can be claimed without a draw. Prizes return to the owner and buyers are refunded.
- Schema binary is `src/bin/schema.rs`. cw-orch interface is `NftfiRaffle` behind the `interface` feature.
