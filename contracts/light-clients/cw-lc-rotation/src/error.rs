use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error(transparent)]
    Registry(#[from] cw_lc_registry::ContractError),
    #[error("unauthorized")]
    Unauthorized {},
    #[error("invalid policy: {0}")]
    BadPolicy(String),
    #[error("committee too small: {have} eligible members, need {need}")]
    TooFewMembers { have: u32, need: u32 },
    #[error("operator {0} alone reaches the threshold")]
    Concentrated(String),
    #[error("an epoch is already pending")]
    PendingInProgress {},
    #[error("an epoch is already active")]
    AlreadyActive {},
    #[error("epoch {0} is not pending")]
    NotPending(u64),
    #[error("epoch {0} expired before activation")]
    Expired(u64),
    #[error("sender is not a member of epoch {0}")]
    NotMember(u64),
    #[error("no active epoch")]
    NoActiveEpoch {},
    #[error("aggregator not set")]
    NoAggregator {},
    #[error("aggregator already set")]
    AggregatorSet {},
    #[error("aggregator reports no stall")]
    NotStalled {},
    #[error("malformed committee key")]
    BadKey {},
    #[error("committee key parameters t={t} n={n} do not match epoch (t={want_t} n={want_n})")]
    KeyMismatch {
        t: u16,
        n: u16,
        want_t: u16,
        want_n: u16,
    },
    #[error("acked shares {have} below threshold {need}")]
    BelowThreshold { have: u32, need: u32 },
    #[error("signature does not verify under the committee key")]
    BadSignature {},
    #[error("hand-off signature from the active epoch is required")]
    HandoffRequired {},
}
