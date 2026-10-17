use cosmwasm_std::{Coin, StdError, Uint128};
use cw_ownable::OwnershipError;
use cw_utils::{ParseReplyError, PaymentError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error(transparent)]
    Ownable(#[from] OwnershipError),

    #[error("{0}")]
    PaymentError(#[from] PaymentError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("LicenseFeeRequired:  {fee}")]
    LicenseFeeRequired { fee: Coin },

    #[error("{0}")]
    ParseReplyError(#[from] ParseReplyError),

    #[error("Got a submessage reply with unknown id: {id}")]
    UnknownReplyId { id: u64 },

    #[error("reentered factory during payroll instantiation")]
    Reentrancy,

    #[error("vesting contract vests ({expected}) tokens, funded with ({sent})")]
    WrongFundAmount { sent: Uint128, expected: Uint128 },
}
