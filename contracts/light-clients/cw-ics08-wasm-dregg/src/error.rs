use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("client is frozen at height {0}")]
    Frozen(u64),

    #[error("settlement_vk_hash must be non-zero 32-byte hex")]
    InvalidVkHash,

    #[error("num_turns must be > 0")]
    ZeroTurns,

    #[error("continuity broken: update genesis_root must equal current proven_lanes")]
    ContinuityBroken,

    #[error("lane value not in BabyBear field")]
    InvalidLane,

    #[error("proof verification failed: {0}")]
    VerifyFailed(String),

    #[error("membership proofs not supported for dregg cell heaps in B2 (residual)")]
    MembershipNotSupportedYet,

    #[error("misbehaviour not yet enforced (LC-MISB residual)")]
    MisbehaviourResidual,

    #[error("unauthorized")]
    Unauthorized,

    #[error("invalid claim_ceiling / trust_level for this client build")]
    PolicyRejected,
}
