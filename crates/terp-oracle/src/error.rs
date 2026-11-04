use cosmwasm_std::{StdError, Uint256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum OracleError {
    #[error("{0}")]
    Std(String),
    #[error("oracle pricing is not configured")]
    NotConfigured,
    #[error("oracle pricing is configured but not enforced; only the default asset is accepted")]
    NotEnforced,
    #[error("asset {asset} is not accepted against {base_denom}")]
    UnknownAsset { asset: String, base_denom: String },
    #[error("asset {asset} cannot be paid with native funds")]
    NotNative { asset: String },
    #[error("invalid oracle pricing config: {0}")]
    InvalidConfig(String),
    #[error("feed {feed} returned no observations")]
    NoObservations { feed: String },
    #[error("feed {feed} returned an unusable price: {reason}")]
    BadPrice { feed: String, reason: String },
    #[error("oracle price is zero")]
    ZeroPrice,
    #[error(
        "feed {feed} is stale: observed at height {observed}, now {current}, max age {max_age} blocks"
    )]
    Stale {
        feed: String,
        observed: u64,
        current: u64,
        max_age: u64,
    },
    #[error(
        "feed {feed} has height {observed} above the current height {current}; staleness needs local heights"
    )]
    FutureObservation {
        feed: String,
        observed: u64,
        current: u64,
    },
    #[error("feed {feed} has no height; it cannot pass a staleness check")]
    UnknownAge { feed: String },
    #[error("only {got} oracle sources answered, need {min}")]
    NotEnoughSources { got: u32, min: u32 },
    #[error("only {got} observations behind the price, need {min}")]
    NotEnoughObservations { got: u32, min: u32 },
    #[error("feed {feed} deviates {deviation} from the combined price, max {max}")]
    Deviation {
        feed: String,
        deviation: String,
        max: String,
    },
    #[error("math overflow")]
    Overflow,
    #[error("payment of {paid}{denom} is below the required {required}{denom}")]
    Underpaid {
        paid: Uint256,
        required: Uint256,
        denom: String,
    },
    #[error("send exactly one coin")]
    WrongFundsCount,
}

impl From<StdError> for OracleError {
    fn from(e: StdError) -> Self {
        OracleError::Std(e.to_string())
    }
}
