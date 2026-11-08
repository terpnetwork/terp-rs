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

    #[error("Unauthorized")]
    Unauthorized,

    #[error("InvalidInput: {0}")]
    InvalidInput(String),

    #[error("InvalidConfig: {0}")]
    InvalidConfig(String),

    #[error("duration {got} is outside {min}..={max}")]
    InvalidDuration { min: u64, max: u64, got: u64 },

    #[error("auction already exists")]
    AuctionAlreadyExists,

    #[error("auction already started")]
    AuctionStarted,

    #[error("auction has ended")]
    AuctionEnded,

    #[error("auction has not ended")]
    AuctionNotEnded,

    #[error("seller cannot bid")]
    SellerShouldNotBid,

    #[error("bid below minimum")]
    BidTooLow,

    #[error("instant sale bid must equal the reserve price")]
    InstantSaleBidMustMatchReserve,

    #[error("wrong create-auction fee")]
    WrongFee,
}
