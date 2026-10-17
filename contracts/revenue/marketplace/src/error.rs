use cosmwasm_std::{OverflowError, StdError};
use cw_utils::PaymentError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Overflow(#[from] OverflowError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("InvalidInput: {0}")]
    InvalidInput(String),

    #[error("No match found")]
    NoMatchFound,

    #[error("Insufficient funds")]
    InsufficientFunds,

    #[error("InternalError: {0}")]
    InternalError(String),

    #[error("timed auctions belong on revenue-reserve-auction")]
    TimedAuctionUseReserve,
}
