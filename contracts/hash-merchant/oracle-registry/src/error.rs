use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("unauthorized")]
    Unauthorized {},
    #[error("market_id empty")]
    EmptyMarket {},
    #[error("height is past the current block")]
    FutureHeight {},
    #[error("height 0")]
    ZeroHeight {},
    #[error("observation already recorded for this market and height")]
    AlreadyRecorded {},
    #[error("no observations in window")]
    EmptyWindow {},
    #[error("invalid mid")]
    InvalidMid {},
    #[error("ibc2 client is not registered")]
    Ibc2Unregistered {},
    #[error("empty trading pair")]
    EmptyPair {},
    #[error("dex quote already in flight for this market and height")]
    QuoteInFlight {},
    #[error("dex quote batch must contain 1 to 16 quotes")]
    BatchLimit {},
    /// Universal error ack, ICS-20 JSON error, or empty ack. Not a price.
    #[error("no dex quote in acknowledgement")]
    NoQuote {},
}
