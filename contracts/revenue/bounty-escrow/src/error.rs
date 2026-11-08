use cosmwasm_std::{OverflowError, StdError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Overflow(#[from] OverflowError),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("proposer {0} is not on the Terp staker whitelist")]
    NotEligible(String),

    #[error("deposit mismatch: expected exactly {expected}, got [{got}]")]
    FundsMismatch { expected: String, got: String },

    #[error("this message does not accept funds")]
    NonPayable,

    #[error("denom {0} is not an allowed reward denom")]
    DenomNotAllowed(String),

    #[error("reward {amount}{denom} outside allowed range [{min}, {max}]")]
    AmountOutOfRange {
        denom: String,
        amount: String,
        min: String,
        max: String,
    },

    #[error("action `{action}` not allowed from status `{from}`")]
    InvalidTransition { from: String, action: String },

    #[error("bounty has expired")]
    Expired,

    #[error("bounty has not expired yet")]
    NotExpired,

    #[error("bounty_ref {0} already used")]
    DuplicateRef(String),

    #[error("submission_hash must be 64 lowercase hex chars (sha256)")]
    InvalidHash,

    #[error("invalid IPFS CID: {0}")]
    InvalidCid(String),

    #[error("submitter {who} has {staked}{denom} bonded, below min_submit_stake {min}{denom}")]
    BelowSubmitStake {
        who: String,
        staked: String,
        min: String,
        denom: String,
    },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("partial payouts are disabled")]
    PartialNotAllowed,

    #[error("payout amount {amount} exceeds remaining escrow {remaining}")]
    AmountExceedsRemaining { amount: String, remaining: String },

    #[error("not implemented: {0}")]
    Unimplemented(String),

    #[error("unknown reply id {0}")]
    InvalidReplyId(u64),

    #[error("refusing to instantiate on mainnet chain id {0} (dev-chain-guard feature)")]
    MainnetGuard(String),
}

/// cosmwasm-std 3+ dropped `PartialEq` on `StdError`; compare variant and rendered message.
impl PartialEq for ContractError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.to_string() == other.to_string()
    }
}
