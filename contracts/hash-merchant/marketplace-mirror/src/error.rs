use cosmwasm_std::{OverflowError, StdError};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(String),

    #[error("{0}")]
    Overflow(#[from] OverflowError),

    #[error("unauthorized")]
    Unauthorized {},

    #[error("invalid min_buffer_blocks {got}: must be in range [{min}, {max}]")]
    InvalidBuffer {
        got: u64,
        min: u64,
        max: u64,
    },

    #[error("accepted_denom cannot be empty")]
    EmptyDenom {},

    #[error("sku cannot be empty")]
    EmptySku {},

    #[error("qty must be > 0")]
    ZeroQty {},

    #[error("product {sku} already listed")]
    ProductExists { sku: String },

    #[error("product {sku} not found")]
    ProductNotFound { sku: String },

    #[error("insufficient available stock for {sku}: need {need}, available {available}")]
    InsufficientStock {
        sku: String,
        need: u64,
        available: u64,
    },

    #[error("cannot set stock {stock} below reserved {reserved} for {sku}")]
    StockBelowReserved {
        sku: String,
        stock: u64,
        reserved: u64,
    },

    #[error("order {id} not found")]
    OrderNotFound { id: u64 },

    #[error("order {id} status is {status}, expected {expected}")]
    BadOrderStatus {
        id: u64,
        status: String,
        expected: String,
    },

    #[error("buffer not elapsed: height {height} < settle_after {settle_after}")]
    BufferNotElapsed { height: u64, settle_after: u64 },

    #[error("insufficient funds: need {need}{denom}, got {got}")]
    InsufficientFunds {
        need: String,
        got: String,
        denom: String,
    },

    #[error("wrong denom: expected {expected}")]
    WrongDenom { expected: String },

    #[error("protocol_fee_bps {bps} exceeds 10000")]
    InvalidFeeBps { bps: u64 },
}

impl From<StdError> for ContractError {
    fn from(err: StdError) -> Self {
        ContractError::Std(err.to_string())
    }
}
