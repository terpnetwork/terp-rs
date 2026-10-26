//! Typed queries against the terp oracle contracts, plus a raw passthrough.
//!
//! Wire types are mirrored here (same JSON as `cw-oracle-registry`,
//! `cw-oracle-aggregator`, `cw-oracle-twap-probe`) so a minter does not link
//! the oracle contracts. `tests/integration.rs` (wire_types_match_real_contracts) checks they stay in sync.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary, Decimal, QuerierWrapper, StdResult, Uint128, WasmQuery};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::OracleError;
use crate::math::{combine_history, parse_price, to_256};
use crate::types::{Feed, FeedKind, PricePoint};

/// Registry `QueryMsg` subset.
#[cw_serde]
pub enum RegistryQuery {
    Latest { market_id: String },
    History { market_id: String, limit: Option<u32> },
    Twap {
        market_id: String,
        from_height: u64,
        to_height: u64,
    },
}

/// Aggregator `QueryMsg`.
#[cw_serde]
pub enum AggregatorQuery {
    Registry {},
    Median { market_id: String, window: u32 },
}

/// TWAP probe `QueryMsg`.
#[cw_serde]
pub enum ProbeQuery {
    ArithmeticTwap { pool_id: u64 },
}

#[cw_serde]
pub struct History {
    pub observations: Vec<RawObservation>,
}

/// Observation with `source` as an untyped enum tag (`"relayer"`, `"polytone"`, `"ve_quorum"`).
#[cw_serde]
pub struct RawObservation {
    pub market_id: String,
    pub source: String,
    pub source_chain: String,
    pub pool_id: Option<u64>,
    pub height: u64,
    pub mid: Decimal,
    pub recorder: Addr,
}

#[cw_serde]
pub struct Twap {
    pub market_id: String,
    pub from_height: u64,
    pub to_height: u64,
    pub n: u32,
    pub twap: Decimal,
    pub volume_hint: Uint128,
}

#[cw_serde]
pub struct View {
    pub market_id: String,
    pub method: String,
    pub n: u32,
    pub value: Decimal,
}

#[cw_serde]
pub struct ProbeTwap {
    pub arithmetic_twap: String,
    pub pool_id: u64,
}

/// Borrowing wrapper over a `QuerierWrapper`.
pub struct OracleQuerier<'a> {
    pub querier: &'a QuerierWrapper<'a>,
}

impl<'a> OracleQuerier<'a> {
    pub fn new(querier: &'a QuerierWrapper<'a>) -> Self {
        Self { querier }
    }

    /// Smart query any contract and decode into any `T`.
    pub fn raw<T: DeserializeOwned>(&self, contract: &str, msg: &impl Serialize) -> StdResult<T> {
        self.querier.query_wasm_smart(contract, msg)
    }

    /// Smart query with a pre-encoded message; returns the raw response bytes.
    pub fn raw_binary(&self, contract: &str, msg: Binary) -> StdResult<Binary> {
        let req: cosmwasm_std::QueryRequest<cosmwasm_std::Empty> =
            cosmwasm_std::QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: contract.to_string(),
                msg,
            });
        let bin = cosmwasm_std::to_json_vec(&req)?;
        match self.querier.raw_query(&bin) {
            cosmwasm_std::SystemResult::Err(e) => {
                Err(cosmwasm_std::StdError::msg(format!("querier system error: {e}")))
            }
            cosmwasm_std::SystemResult::Ok(cosmwasm_std::ContractResult::Err(e)) => {
                Err(cosmwasm_std::StdError::msg(format!("querier contract error: {e}")))
            }
            cosmwasm_std::SystemResult::Ok(cosmwasm_std::ContractResult::Ok(v)) => Ok(v),
        }
    }

    pub fn latest(&self, registry: &str, market_id: &str) -> StdResult<RawObservation> {
        self.raw(
            registry,
            &RegistryQuery::Latest {
                market_id: market_id.to_string(),
            },
        )
    }

    pub fn history(&self, registry: &str, market_id: &str, limit: u32) -> StdResult<History> {
        self.raw(
            registry,
            &RegistryQuery::History {
                market_id: market_id.to_string(),
                limit: Some(limit),
            },
        )
    }

    pub fn twap(&self, registry: &str, market_id: &str, from: u64, to: u64) -> StdResult<Twap> {
        self.raw(
            registry,
            &RegistryQuery::Twap {
                market_id: market_id.to_string(),
                from_height: from,
                to_height: to,
            },
        )
    }

    pub fn aggregator_registry(&self, aggregator: &str) -> StdResult<Addr> {
        self.raw(aggregator, &AggregatorQuery::Registry {})
    }

    pub fn median(&self, aggregator: &str, market_id: &str, window: u32) -> StdResult<View> {
        self.raw(
            aggregator,
            &AggregatorQuery::Median {
                market_id: market_id.to_string(),
                window,
            },
        )
    }

    pub fn probe(&self, probe: &str, pool_id: u64) -> StdResult<ProbeTwap> {
        self.raw(probe, &ProbeQuery::ArithmeticTwap { pool_id })
    }

    /// Read one feed into a [`PricePoint`]. No policy checks here.
    pub fn point(&self, feed: &Feed) -> Result<PricePoint, OracleError> {
        let label = feed.kind.label();
        let o = feed.oracle.as_str();
        let wrap = |e: cosmwasm_std::StdError| OracleError::BadPrice {
            feed: label.clone(),
            reason: e.to_string(),
        };
        let (value, height, observations) = match &feed.kind {
            FeedKind::Spot { market_id } => {
                let l = self.latest(o, market_id).map_err(wrap)?;
                (to_256(l.mid), Some(l.height), 1)
            }
            FeedKind::Twap {
                market_id,
                window_blocks,
            } => {
                let l = self.latest(o, market_id).map_err(wrap)?;
                let from = l.height.saturating_sub(window_blocks.saturating_sub(1)).max(1);
                let t = self.twap(o, market_id, from, l.height).map_err(wrap)?;
                (to_256(t.twap), Some(l.height), t.n)
            }
            FeedKind::History {
                market_id,
                limit,
                method,
            } => {
                let h = self.history(o, market_id, *limit).map_err(wrap)?;
                if h.observations.is_empty() {
                    return Err(OracleError::NoObservations { feed: label });
                }
                let rows: Vec<_> = h
                    .observations
                    .iter()
                    .map(|r| (to_256(r.mid), r.height))
                    .collect();
                let newest = rows.iter().map(|(_, h)| *h).max();
                (
                    combine_history(&rows, *method)?,
                    newest,
                    rows.len() as u32,
                )
            }
            FeedKind::AggregatorMedian { market_id, window } => {
                let v = self.median(o, market_id, *window).map_err(wrap)?;
                let reg = self.aggregator_registry(o).map_err(wrap)?;
                let l = self.latest(reg.as_str(), market_id).map_err(wrap)?;
                (to_256(v.value), Some(l.height), v.n)
            }
            FeedKind::ProbeTwap { pool_id } => {
                let p = self.probe(o, *pool_id).map_err(wrap)?;
                let v = parse_price(&p.arithmetic_twap).map_err(|e| match e {
                    OracleError::BadPrice { reason, .. } => OracleError::BadPrice {
                        feed: label.clone(),
                        reason,
                    },
                    other => other,
                })?;
                (v, None, 1)
            }
        };
        if value.is_zero() {
            return Err(OracleError::BadPrice {
                feed: label,
                reason: "zero".into(),
            });
        }
        Ok(PricePoint {
            feed: label,
            oracle: feed.oracle.clone(),
            value,
            height,
            observations,
            weight: feed.weight.unwrap_or(1),
        })
    }
}
