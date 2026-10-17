use cosmwasm_std::StdError;
use thiserror::Error;

use crate::state::RaffleState;

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("unauthorized")]
    Unauthorized,
    #[error("contract is locked")]
    Locked,
    #[error("name must be 3-50 bytes")]
    InvalidName,
    #[error("comment is longer than 20000 bytes")]
    CommentTooLong,
    #[error("fee rate must be below 1")]
    InvalidFeeRate,
    #[error("creation fee was not paid")]
    InvalidFee,
    #[error("unexpected funds")]
    ExtraFunds,
    #[error("raffle needs at least one asset")]
    NoAssets,
    #[error("duplicate raffle asset")]
    DuplicateAssets,
    #[error("ticket price must be a native coin of at least 1")]
    InvalidTicketCost,
    #[error("raffle duration is below the minimum")]
    DurationTooShort,
    #[error("max tickets exceed the contract cap")]
    TicketCap,
    #[error("sender is not the cw721 owner")]
    NotOwner,
    #[error("cw721 is not approved for this contract")]
    NotApproved,
    #[error("raffle not found")]
    NotFound,
    #[error("wrong state {state:?} for this action")]
    WrongState { state: RaffleState },
    #[error("tickets already sold")]
    TicketsSold,
    #[error("ticket payment does not match the price")]
    Payment,
    #[error("ticket count must be at least 1")]
    NoTickets,
    #[error("per-address ticket limit exceeded")]
    UserLimit,
    #[error("raffle ticket limit exceeded")]
    RaffleLimit,
    #[error("this randomness source does not accept that message")]
    WrongRandomnessSource,
    #[error("seed must be 32 bytes")]
    InvalidSeed,
    #[error("randomness is already set")]
    RandomnessSet,
    #[error("reply did not include draw attributes")]
    BadDrawReply,
}

impl From<nftfi_utils::PullError> for ContractError {
    fn from(value: nftfi_utils::PullError) -> Self {
        match value {
            nftfi_utils::PullError::NotOwner => Self::NotOwner,
            nftfi_utils::PullError::NotApproved => Self::NotApproved,
            nftfi_utils::PullError::Query(err) => Self::Std(err),
        }
    }
}
