//! Fixed-price NFT marketplace: asks, token bids, and collection bids.
//!
//! Ported from public-awesome `stargaze-marketplace-v2`. Sale proceeds pay
//! a capped cw721 royalty, optional finder rewards taken out of the protocol
//! fee, and a fair-burn split of the remaining protocol fee.
mod error;
pub mod execute;
mod helpers;
pub mod instantiate;
pub mod msg;
mod orders;
pub mod query;
pub mod state;

pub use crate::error::ContractError;
pub use crate::execute::execute;
pub use crate::instantiate::instantiate;
pub use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
pub use crate::query::query;

#[cfg(feature = "interface")]
mod interface;
#[cfg(feature = "interface")]
pub use interface::RevenueMarketplace;

pub const CONTRACT_NAME: &str = "revenue-marketplace";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod multitest;
