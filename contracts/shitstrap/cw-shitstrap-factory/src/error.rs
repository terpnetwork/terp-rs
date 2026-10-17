use cosmwasm_std::{StdError, Uint128};
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

    #[error("{0}")]
    ParseReplyError(#[from] ParseReplyError),

    #[error("Parsing Error: {err}")]
    ReplyParseError { err: String },

    #[error("Got a submessage reply with unknown id: {id}")]
    UnknownReplyId { id: u64 },

    #[error("reentered factory during payroll instantiation")]
    Reentrancy,

    #[error("vesting contract vests ({expected}) tokens, funded with ({sent})")]
    WrongFundAmount { sent: Uint128, expected: Uint128 },

    #[error("IBC ack is not a success")]
    AckNotSuccess {},

    #[error("No transfer data in IBC destination callback")]
    NoTransferData {},

    #[error("Receiver mismatch: expected {expected}, got {got}")]
    ReceiverMismatch { expected: String, got: String },
}

impl PartialEq for ContractError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}
