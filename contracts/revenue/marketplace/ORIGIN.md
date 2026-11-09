# Origin

| | |
|---|---|
| Repo | https://github.com/public-awesome/marketplace |
| Commit | `ae9ffacbf2c61b8f7f6635030bad4cbbea7cfd3c` |
| Paths | `contracts/stargaze-marketplace-v2/`, `packages/sg-marketplace-common/` (sale, nft transfer, royalty fetch) |
| Related | https://github.com/public-awesome/core `6982c764996ea3fa0cbad4b4404a117f55f4c564` `contracts/royalty-registry/`, `contracts/fair-burn/` (concepts only; those contracts are not vendored) |

Cloned read-only at `/tmp/nftfi-src/upstream/marketplace` and `/tmp/nftfi-src/upstream/core`.

## What changed

- CosmWasm v1 types (`Uint128` coin amounts, `to_binary`) retargeted at this workspace's CosmWasm v3 (`Coin.amount` is `Uint256`, `to_json_binary`, `cw-storage-plus` 3 indexes).
- Dropped `sg-marketplace-common`, `sg-index-query`, `cw-address-like`, and `blake2`. Order ids are SHA-256 (workspace `sha2` + `hex`) instead of Blake2s.
- Royalties are read from the cw721 collection extension (`GetCollectionInfoAndExtension.extension.royalty_info`: `payment_address` + `Decimal` share), which is the royalty field on `cw721::CollectionExtension`. If `royalty_registry` is set, a Stargaze `RoyaltyPayment` query is tried first and the cw721 extension is the fallback. The registry is not written to.
- Fair burn is in-contract. `burn_bps` of the protocol fee and of a listing fee is `BankMsg::Burn`. The remainder is sent to `fee_manager`. There is no fair-burn contract and no Stargaze fair-burn pool message.
- A listing fee is optional. If none is stored for the denom, `SetAsk` must be sent with no funds. Upstream required a preconfigured fee.
- `ReceiveNft` accepts the preview `svg-auction` hook `CreateAuction { denom, reserve_price, is_instant_sale }` plus `SetAsk`. `is_instant_sale: false` is rejected here; timed auctions belong on `revenue-reserve-auction`. See `MIGRATION-svg-auction.md`.
- Prices indexed above `u128::MAX` are rejected. JSON order ids differ from upstream because the hash function changed.
- Query variants are structs (`{"asks_by_seller":{"seller":"..."}}`), not the upstream tuple form.
