use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("unauthorized")]
    Unauthorized {},
    #[error("invalid label `{label}`: {reason}")]
    InvalidLabel { label: String, reason: String },
    #[error("invalid client id `{0}`")]
    InvalidClientId(String),
    #[error("client `{0}` already registered")]
    ClientExists(String),
    #[error("client `{0}` not found")]
    ClientNotFound(String),
    #[error("operator already registered")]
    OperatorExists {},
    #[error("operator not registered: {0}")]
    OperatorNotFound(String),
    #[error("operator is not an attestor for this client")]
    NotAttestor {},
    #[error("pubkey must be 32 bytes (ed25519)")]
    BadPubkey {},
    #[error("signature must be 64 bytes (ed25519)")]
    BadSignatureLen {},
    #[error("signature does not verify")]
    BadSignature {},
    #[error("invalid hex field `{0}`")]
    BadHex(String),
    #[error("min_attestations must be 1..=attestors ({attestors})")]
    BadThreshold { attestors: u32 },
    #[error("stale report: observed_at must increase")]
    StaleReport {},
    #[error("report contradicts the hash-market quorum root at height {height}")]
    QuorumConflict { height: u64 },
    #[error("field `{0}` must not be empty")]
    Empty(String),
}
