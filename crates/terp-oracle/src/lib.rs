//! terp-oracle: read oracle feeds from inside NFT minting contracts.
//!
//! * [`oracle_query`]: attribute macro that appends oracle query variants to a
//!   contract `QueryMsg` (like `cw_ownable_query`).
//! * [`OracleQuerier`]: typed helpers for the terp oracle contracts
//!   (registry spot / history / TWAP, aggregator median, TWAP probe) plus a raw
//!   passthrough that deserializes into any `T`.
//! * [`math`]: aggregation (median, mean, weighted, min, max) and
//!   base-to-alt conversion in `Decimal256` / `Uint512`, rounded up.
//! * [`OraclePricing`]: optional config. Lets a contract accept an alternate
//!   asset priced against its default mint asset, with staleness, source-count,
//!   deviation and slippage guards.
//!
//! See README.md for usage.

pub mod error;
pub mod math;
pub mod msg;
pub mod pricing;
pub mod querier;
pub mod types;

pub use error::OracleError;
pub use msg::{OracleQueryMsg, handle_oracle_query};
pub use pricing::{Settlement, price_of, quote, settle_payment};
pub use querier::OracleQuerier;
pub use terp_oracle_derive::oracle_query;
pub use types::*;

#[cfg(test)]
mod tests;
