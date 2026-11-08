//! The oracle query variants, as a standalone enum, and their handler.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Binary, Coin, Deps, Env, StdError, StdResult, to_json_binary};

use crate::pricing::{price_of, quote};
use crate::types::{
    AggregateEntry, AggregateResponse, AssetPair, MAX_AGGREGATE_PAIRS, OraclePricing, PriceAsset,
};

/// Same variants `#[oracle_query]` adds to a contract `QueryMsg`.
#[cw_serde]
#[derive(QueryResponses)]
pub enum OracleQueryMsg {
    #[returns(Option<OraclePricing>)]
    OraclePricing {},
    #[returns(crate::PriceResponse)]
    OraclePrice {
        asset: PriceAsset,
        base_denom: String,
    },
    #[returns(AggregateResponse)]
    OracleAggregate { pairs: Vec<AssetPair> },
    #[returns(crate::QuoteResponse)]
    OracleQuote { base: Coin, asset: PriceAsset },
    #[returns(Binary)]
    OracleRaw { oracle: String, msg: Binary },
}

/// Answer an [`OracleQueryMsg`] for a contract whose config is `pricing`.
#[inline(never)]
pub fn handle_oracle_query(
    deps: Deps,
    env: &Env,
    pricing: Option<&OraclePricing>,
    msg: OracleQueryMsg,
) -> StdResult<Binary> {
    let need = || pricing.ok_or_else(|| StdError::msg("oracle pricing is not configured"));
    let h = env.block.height;
    match msg {
        OracleQueryMsg::OraclePricing {} => to_json_binary(&pricing),
        OracleQueryMsg::OraclePrice { asset, base_denom } => {
            to_json_binary(&price_of(&deps.querier, h, need()?, &asset, &base_denom)?)
        }
        OracleQueryMsg::OracleAggregate { pairs } => {
            if pairs.len() > MAX_AGGREGATE_PAIRS {
                return Err(StdError::msg(format!(
                    "at most {MAX_AGGREGATE_PAIRS} pairs per query"
                )));
            }
            let p = need()?;
            let entries = pairs
                .into_iter()
                .map(
                    |pair| match price_of(&deps.querier, h, p, &pair.asset, &pair.base_denom) {
                        Ok(price) => AggregateEntry {
                            pair,
                            price: Some(price),
                            error: None,
                        },
                        Err(e) => AggregateEntry {
                            pair,
                            price: None,
                            error: Some(e.to_string()),
                        },
                    },
                )
                .collect();
            to_json_binary(&AggregateResponse { height: h, entries })
        }
        OracleQueryMsg::OracleQuote { base, asset } => {
            to_json_binary(&quote(&deps.querier, h, need()?, &base, &asset)?)
        }
        OracleQueryMsg::OracleRaw { oracle, msg } => {
            let p = need()?;
            if !p.oracles().contains(&oracle) {
                return Err(StdError::msg(
                    "oracle is not referenced by the pricing config",
                ));
            }
            // Wrapped so the reply matches `#[returns(Binary)]`: base64 of the oracle JSON.
            to_json_binary(&crate::OracleQuerier::new(&deps.querier).raw_binary(&oracle, msg)?)
        }
    }
}
