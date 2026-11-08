# Replacing preview `svg-auction` with revenue contracts

Preview `contracts/infuser/svg-auction` (main tree, uncommitted) is a single-collection English auction with an instant-sale flag. It is not a drop-in ABI match for these revenue contracts. A later swap can keep the same cw721 `SendNft` listing path and route the two sale modes to two contracts.

## What stays the same

Both sides escrow with cw721 `TransferNft` / `SendNft` and read `OwnerOf`. `SendNft` delivers:

```json
{"receive_nft":{"sender":"<owner>","token_id":"<id>","msg":"<hook>"}}
```

The hook JSON from preview is:

```json
{"create_auction":{"denom":"uterp","reserve_price":"1000","is_instant_sale":true}}
```

- `revenue-marketplace` accepts that hook when `is_instant_sale` is true and creates a fixed ask at `reserve_price`. The NFT is already escrowed by `SendNft`, so the marketplace does not transfer it again. `is_instant_sale: false` returns `TimedAuctionUseReserve`.
- `revenue-reserve-auction` accepts the same hook for both flags. Missing `duration` uses `min_duration` (`#[serde(default)]`). `is_instant_sale: true` requires the first bid to equal the reserve and settles in that transaction. `false` runs a timed reserve auction.

cw721-svg metadata does not include `royalty_info`. Royalties apply only when the collection extension JSON has `royalty_info`. A query failure leaves the NFT tradable and takes no royalty.

## Message map

| Preview `svg-auction` | Revenue replacement |
| --- | --- |
| `ReceiveNft` + `CreateAuction { is_instant_sale: true }` | Marketplace `ReceiveNft` (fixed ask) or reserve-auction `ReceiveNft` (instant settle on the first exact bid) |
| `ReceiveNft` + `CreateAuction { is_instant_sale: false }` | Reserve-auction `ReceiveNft`. Marketplace rejects it |
| `PlaceBid { auction_id }` | Reserve-auction `PlaceBid { collection, token_id }`. Marketplace bids are `SetBid` / `AcceptBid` and do not use a clock |
| `Settle { auction_id }` | Reserve-auction `SettleAuction { collection, token_id }`, or sudo `EndBlock` |
| `CancelAuction { auction_id }` | Reserve-auction `CancelAuction` before the first bid. Marketplace `RemoveAsk` while the ask is open |
| Instant buy at reserve | Marketplace `AcceptAsk` / `BuySpecificNft` after the instant hook. Reserve-auction instant sale settles inside `PlaceBid` |
| Proceeds: protocol bps to `protocol_addr`, no royalty | Protocol fee split by `burn_bps` (`BankMsg::Burn`) and `fee_manager`, plus cw721 royalty capped on the marketplace by `max_royalty_fee_bps` |

## Config map

| Preview | Marketplace | Reserve auction |
| --- | --- | --- |
| `protocol_fee_bps` (max 1000) | `protocol_fee_bps` | `trading_fee_percent` (`Decimal`, 0.02 = 200 bps) |
| `protocol_addr` | `fee_manager` plus `burn_bps` | same |
| `min_increment_bps` | not used (fixed price) | `min_bid_increment_percent` |
| `min_reserve_price` | price must be non-zero and fit in `u128` | `min_reserve_prices` per denom |
| `duration`, `extension` | not used | `min_duration` / `max_duration`, `extend_duration` |
| `accepted_denom` | `default_denom` or per-collection denom | denoms present in `min_reserve_prices` |
| numeric `auction_id` | ask id is sha256(collection \|\| token_id) | key is `(collection, token_id)` |

Preview bids refund the previous bidder in the same transaction and extend the end when a bid lands inside `extension`. Reserve-auction does the same with `extend_duration`. Marketplace bids escrow until accept, cancel, or an automatic match. They do not expire.

Query JSON uses struct variants (`{"ask":{"id":"..."}}`, `{"auction":{"collection":"...","token_id":"..."}}`). Preview queries are keyed by `auction_id`.
