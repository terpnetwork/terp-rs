# Origin

- Repo: https://github.com/public-awesome/marketplace
- Commit: `ae9ffacbf2c61b8f7f6635030bad4cbbea7cfd3c`
- Path: `contracts/reserve-auction`
- License: Business Source License 1.1, Change Date 2025-05-01, Change License GPL-2.0-or-later. The change date has passed. This crate is GPL-2.0-or-later and keeps the upstream LICENSE.

## What changed

- CosmWasm v3 / cw-storage-plus from the terp-rs workspace. `Coin.amount` is `Uint256`. No `sg-*` crates.
- `fair_burn` address is replaced by `fee_manager` plus `burn_bps`. The protocol fee is split with `BankMsg::Burn` and `BankMsg::Send`. There is no fair-burn contract.
- `trading_fee_percent` and `min_bid_increment_percent` stay `Decimal` percents, matching upstream.
- Royalty is read from the cw721 collection extension `royalty_info`. The Stargaze royalty registry is not queried and is not initialized.
- `prev_block_time` is set to the instantiate block time so the first `BeginBlock` does not treat genesis as a halt.
- `instant_sale` (serde default `false`) settles a first bid that equals the reserve in the same transaction. `ReceiveNft` / `CreateAuction` accepts the preview svg-auction hook (`denom`, `reserve_price`, `is_instant_sale`) plus an optional `duration` (default `min_duration`).
- Query variants are structs (`{"auction":{"collection":"...","token_id":"..."}}`), not tuples.
- IndexedMap has no lifetime parameter on this cw-storage-plus. The end-time index uses `u64::MAX` until the first bid.
