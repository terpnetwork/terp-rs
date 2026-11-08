use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("nullifier already spent (domain={domain}, session={session})")]
    AlreadySpent { domain: String, session: String },

    #[error("invalid authenticator params: {0}")]
    InvalidParams(String),

    #[error("invalid auth payload: {0}")]
    InvalidPayload(String),

    #[error("session_id required but missing")]
    MissingSession,

    #[error("session_id mismatch: params={params:?} payload={payload:?}")]
    SessionMismatch {
        params: Option<String>,
        payload: Option<String>,
    },

    #[error("too many nullifiers: {got} > max {max}")]
    TooManyNullifiers { got: u32, max: u32 },

    #[error("empty nullifier list")]
    EmptyNullifiers,

    #[error("empty nullifier bytes")]
    EmptyNullifierBytes,

    #[error("could not extract nullifiers from message for ConfirmExecution")]
    NullifiersNotInMsg,
}
