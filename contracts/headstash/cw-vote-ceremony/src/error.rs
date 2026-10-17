use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("unauthorized")]
    Unauthorized,

    #[error("ceremony already exists: {session_id}")]
    CeremonyExists { session_id: String },

    #[error("ceremony not found: {session_id}")]
    CeremonyNotFound { session_id: String },

    #[error("ceremony not active: {session_id} status={status}")]
    CeremonyNotActive { session_id: String, status: String },

    #[error("domain must be non-empty")]
    EmptyDomain,

    #[error("session_id must be non-empty")]
    EmptySession,

    #[error("nullifier already spent (domain={domain}, session={session})")]
    AlreadySpent { domain: String, session: String },

    #[error("empty nullifier bytes")]
    EmptyNullifierBytes,

    #[error("empty nullifier list")]
    EmptyNullifiers,

    #[error("too many nullifiers: {got} > max {max}")]
    TooManyNullifiers { got: u32, max: u32 },

    #[error("already registered: {addr}")]
    AlreadyRegistered { addr: String },

    #[error("registration closed for session {session_id}")]
    RegistrationClosed { session_id: String },

    #[error("registration gate denied: {reason}")]
    GateDenied { reason: String },

    #[error("leaf_commit required for clerk_attested gate")]
    LeafCommitRequired,

    #[error("leaf already attested")]
    LeafAlreadyAttested,

    #[error("pre-propose registration gate is not wired")]
    PreProposeUnwired,

    #[error("leaf already consumed by {addr}")]
    LeafConsumed { addr: String },
}


