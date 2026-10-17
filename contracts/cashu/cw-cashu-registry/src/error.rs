use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(String),

    #[error("unauthorized")]
    Unauthorized {},

    #[error("invalid mint_id '{id}': must match [A-Za-z0-9._-]{{1,200}} and not contain '..'")]
    InvalidMintId { id: String },

    #[error("url cannot be empty")]
    EmptyUrl {},

    #[error("url exceeds max length {max}")]
    UrlTooLong { max: usize },

    #[error("mint_id '{mint_id}' already registered")]
    MintIdExists { mint_id: String },

    #[error("normalized url already registered as mint_id '{mint_id}'")]
    UrlExists { mint_id: String },

    #[error("mint '{mint_id}' not found")]
    MintNotFound { mint_id: String },

    #[error("metadata exceeds max_metadata_bytes ({got} > {max})")]
    MetadataTooLarge { got: usize, max: u32 },

    #[error("invalid content_sha256: expected 64 lowercase hex chars")]
    InvalidContentSha256 {},

    #[error("IsRegistered requires exactly one of mint_id or url")]
    InvalidIsRegisteredQuery {},
}

impl From<StdError> for ContractError {
    fn from(err: StdError) -> Self {
        ContractError::Std(err.to_string())
    }
}
