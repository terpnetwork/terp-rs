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
    #[error("height 0")]
    ZeroHeight {},
    #[error("observation already recorded for this market and height")]
    AlreadyRecorded {},
    #[error("no observations in window")]
    EmptyWindow {},
    #[error("invalid mid")]
    InvalidMid {},
}
