# terp-oracle

Read oracle feeds from inside NFT minting contracts, and optionally accept an
alternate asset priced against the default mint asset.

Two crates:

| crate | what |
|---|---|
| `terp-oracle` | types, querier, math, pricing/settlement, query handler |
| `terp-oracle-derive` | `#[oracle_query]` attribute macro (re-exported as `terp_oracle::oracle_query`) |

The library does not link the oracle contracts. It mirrors their query JSON
(`cw-oracle-registry`, `cw-oracle-aggregator`, `cw-oracle-twap-probe`). Wire-JSON
tests check that the mirror stays in sync.

## 1. Derive the queries into your `QueryMsg`

```rust
#[terp_oracle::oracle_query]   // must sit above #[cw_serde] / QueryResponses
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
}
```

This appends:

| variant | returns | what |
|---|---|---|
| `OraclePricing {}` | `Option<OraclePricing>` | the stored config |
| `OraclePrice { asset, base_denom }` | `PriceResponse` | one policy-checked price, with per-feed points |
| `OracleAggregate { pairs }` | `AggregateResponse` | up to 16 pairs at once (`entries[i]` is a price or an error string) |
| `OracleQuote { base, asset }` | `QuoteResponse` | the alt amount required for `base`, and the minimum after slippage |
| `OracleRaw { oracle, msg }` | `Binary` | raw smart-query passthrough, so any return shape works |

It also generates `QueryMsg::into_oracle_query(self) -> Result<OracleQueryMsg, Self>`.
Dispatch looks like this:

```rust
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let msg = match msg.into_oracle_query() {
        Ok(o) => {
            let pricing = ORACLE_PRICING.may_load(deps.storage)?;
            return terp_oracle::handle_oracle_query(deps, &env, pricing.as_ref(), o);
        }
        Err(msg) => msg,
    };
    match msg {
        QueryMsg::Config {} => /* ... */,
        _ => Err(StdError::msg("unknown query")), // the oracle variants were handled above
    }
}
```

`OracleQueryMsg` is also available as a standalone enum, for example for a nested extension query.

## 2. Configure pricing

```rust
OraclePricing {
    enforce: true,                       // false: queries answer, alt payments are rejected
    alt_assets: vec![AltAsset {
        asset: PriceAsset::native("ibc/ATOM..."),
        base_denom: "uterp".into(),      // the default mint asset
        feeds: vec![Feed {
            oracle: registry_addr,
            kind: FeedKind::Spot { market_id: "ATOM/TERP".into() },
            weight: None,
        }],
        combine: AggregateMethod::Median,
        quote: Quote::AltInBase,         // feed = TERP per 1 ATOM
        base_decimals: 6,
        alt_decimals: 6,
    }],
    policy: PricingPolicy {
        max_staleness_blocks: Some(100),
        min_sources: Some(1),
        min_observations: None,
        max_deviation: Some(Decimal::percent(5)),
        slippage: Some(Decimal::percent(1)),
    },
}
```

Call `pricing.validate(api)` on instantiate and on update. It checks oracle
addresses, empty denoms, alt == base, duplicate pairs, feed count (1 to 8),
alt asset count (at most 16), decimals (at most 24), and zero weights.

### Feed kinds (what a feed can return)

| kind | source query | height for staleness |
|---|---|---|
| `Spot { market_id }` | registry `Latest` | observation height |
| `Twap { market_id, window_blocks }` | registry `Twap` | `to_height` |
| `History { market_id, limit, method }` | registry `History`, combined locally | newest row |
| `AggregatorMedian { market_id, window }` | aggregator `Median` | aggregator registry `Latest` |
| `ProbeTwap { pool_id }` | probe `ArithmeticTwap` | none (staleness is skipped) |

Feeds are combined with `Median`, `Mean` (using `Feed::weight`), `Weighted`
(by observation count), `Min` or `Max`. A history window uses the same
methods, and `Weighted` there is a block-weighted TWAP matching the registry.

### Policy guards

* `max_staleness_blocks`: rejects a feed whose observation is too old. It needs feeds that record local terp heights.
* `min_sources` / `min_observations`: set how many feeds must answer, and how many observations must back the price.
* `max_deviation`: rejects the price when any feed is too far from the combined value.
* `slippage`: accepts a payment of at least `ceil(required * (1 - slippage))`.

## 3. Settle a payment

```rust
let s: Settlement = terp_oracle::settle_payment(
    deps.as_ref(), &env, pricing.as_ref(), &base_price /* Coin */, &info.funds)?;
// s.charged: coin to keep / forward;  s.refund: overpayment to return
// s.alt: true when paid in an alt asset; s.quote: the quote used
```

Rules:

* Exactly one coin must be sent.
* A coin in `base.denom` must match exactly, the same as before. This needs no pricing.
* A coin in any other denom needs `enforce == true` and a matching `AltAsset`.
  - The payment must be at least the slippage minimum.
  - Anything above the full quote goes back in `refund`.
* Base-to-alt conversion is done in `Decimal256`/`Uint512` and **rounds up**,
  so the contract is never short. `alt_to_base` rounds down.

Errors are `OracleError` values: `NotConfigured`, `NotEnforced`,
`UnknownAsset`, `NotNative`, `WrongFundsCount`, `Underpaid`, `Stale`,
`FutureObservation`, `UnknownAge`, `NoObservations`, `BadPrice`, `ZeroPrice`,
`NotEnoughSources`, `NotEnoughObservations`, `Deviation`, `Overflow`,
`InvalidConfig`, and `Std`.

## Lower-level pieces

* `OracleQuerier`: typed `latest`, `history`, `twap`, `median`, `probe` and `point(&Feed)`, plus `raw::<T>()` / `raw_binary()` for any contract and return type.
* `math`: `parse_price`, `combine`, `combine_history`, `base_to_alt`, `alt_to_base`, `apply_slippage`, `deviation`, `invert`.
* `pricing`: `price_of`, `quote`, and pure `evaluate`, `quote_from_price` and `settle_with_quote` for testing without a chain.

## Wired contracts

| contract | config | alt payment |
|---|---|---|
| `nftfi-raffle` | `InstantiateMsg.oracle_pricing`, `ExecuteMsg::UpdateOraclePricing` (owner) | `BuyTicket` accepts an alt coin. Alt pots are tracked per denom (`QueryMsg::AltPot`), split by fee at claim, and refunded in the alt coin on cancel |
| `cw721-svg` | `SvgExecuteMsgExt::SetOraclePricing` (creator-owner or instantiating factory) | a paid mint in an alt coin goes to the withdraw address, with the overpayment refunded. Free mints ignore pricing |
| `cw-svg-minter` (factory) | `InstantiateMsg.oracle_pricing`, `ExecuteMsg::UpdateOraclePricing` (owner) | the default is pushed to each **new** collection right after instantiate2 |

When pricing is absent or `enforce == false`, payments behave exactly as before.

## Limits

* cw20 alt assets work for quotes only. Paying with them would need a cw20 `Receive` hook in each contract.
* The registry stores block heights, not timestamps. Observations relayed with foreign-chain heights fail the staleness check (`FutureObservation`), so use `max_staleness_blocks: None` for those feeds.
* Changing the factory default does not update existing collections.

## Tests

```
cargo test -p terp-oracle -p terp-oracle-derive
```

These cover the math (rounding, decimals, overflow), every aggregate method,
policy guards, settlement edge cases, config validation, wire JSON against the
real oracle contracts, and the macro output. Each wired contract also has a
`tests/oracle.rs` multi-test that runs against a live `cw-oracle-registry`.
