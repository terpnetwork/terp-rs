# Origin

- Repo: https://github.com/AtlasDA0/app-contracts
- Commit: `4b921920092004f4a79724e3c0db3f7993b64be7`
- Paths: `packages/utils/src/lib.rs`, `packages/utils/src/payment.rs`, `packages/utils/src/state.rs`, `packages/utils/src/types.rs`

## What changed

- Dropped the `sg` feature and `Sg721Token`. Assets are `Coin` or `Cw721`.
- Coin amounts follow this workspace (`Uint256`).
- `Response` / `SubMsg` aliases for Stargaze message wrappers are gone.
- Added `assert_can_pull` and `transfer_nft_msg` against `cw721::msg::Cw721ExecuteMsg` and `Cw721QueryMsg` (the messages `cw721-svg` uses).
- Native coins with the same denom count as overlapping assets.
- `testing` feature builds a small cw721 double for host tests. It speaks the same execute and query JSON as the trait.
