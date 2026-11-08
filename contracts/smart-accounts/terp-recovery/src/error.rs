use cosmwasm_std::{StdError, VerificationError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Verification(#[from] VerificationError),

    #[error("unauthorized")]
    Unauthorized {},

    #[error("missing recovery config")]
    MissingConfig {},

    #[error("recovery threshold not met ({got}/{need})")]
    ThresholdNotMet { got: u32, need: u32 },

    #[error("unsupported or mismatched hash algorithm: {reason}")]
    HashAlg { reason: String },

    #[error("invalid rehash_rounds (must be 1..=64, got {got})")]
    InvalidRounds { got: u32 },

    #[error("invalid guardian: {reason}")]
    InvalidGuardian { reason: String },

    #[error("break-glass challenge verification failed for guardian {guardian}: {reason}")]
    ApprovalFailed { guardian: String, reason: String },

    #[error("duplicate guardian approval: {guardian}")]
    DuplicateApproval { guardian: String },

    #[error("invalid recovery proof: {reason}")]
    InvalidProof { reason: String },

    #[error("recovery preimage proof verify is not linked")]
    ProofNotLinked {},
}
