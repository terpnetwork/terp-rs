use cosmwasm_std::{StdError, VerificationError};
use cw_ownable::OwnershipError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    VerificationError(#[from] VerificationError),

    #[error("{0}")]
    OwnershipError(#[from] OwnershipError),

    #[error("authenticator metadata must be provided")]
    MissingAuthenticatorMetadata {},

    #[error("at most 10 operator keys")]
    TooManyKeys {},

    #[error("invalid BLS config: {reason}")]
    InvalidConfig { reason: String },

    #[error("invalid BLS signature: {reason}")]
    InvalidSignature { reason: String },

    #[error("threshold not met ({got}/{need})")]
    ThresholdNotMet { got: u32, need: u32 },
}
