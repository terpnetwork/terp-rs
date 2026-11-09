use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("unauthorized")]
    Unauthorized {},

    #[error("voting session not registered: {session_id}")]
    UnknownSession { session_id: String },

    #[error("invalid VSCK proof: {reason}")]
    InvalidProof { reason: String },

    #[error("nullifier already used in session")]
    NullifierReplay {},

    #[error("role not allowed for this message: {role}")]
    RoleDenied { role: String },

    #[error("missing authenticator params")]
    MissingParams {},

    /// Envelope / role-circuit policy passed, but vote-sdk Halo2 verify is not
    /// linked. Default build is fail-closed — stub proofs must not authenticate.
    #[error("vote-sdk circuit verify not linked: {circuit}")]
    CircuitNotLinked { circuit: String },
}
