use cosmwasm_std::StdError;
use thiserror::Error;

use crate::state::{LoanState, OfferState};

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
    #[error("listing fee was not paid")]
    InvalidFee,
    #[error("unexpected funds")]
    ExtraFunds,
    #[error("loan needs at least one cw721")]
    NoAssets,
    #[error("duplicate collateral")]
    DuplicateAssets,
    #[error("collateral must be a cw721 token")]
    NotNft,
    #[error("sender is not the cw721 owner")]
    NotOwner,
    #[error("cw721 is not approved for this contract")]
    NotApproved,
    #[error("loan not found")]
    NotFound,
    #[error("offer not found")]
    OfferNotFound,
    #[error("loan state {state:?} does not allow this")]
    WrongLoanState { state: LoanState },
    #[error("offer state {state:?} does not allow this")]
    WrongOfferState { state: OfferState },
    #[error("listing has no terms to accept")]
    NoTerms,
    #[error("sent funds do not match the loan terms")]
    FundsMismatch,
    #[error("send exactly one coin")]
    MultipleCoins,
    #[error("loan is already defaulted")]
    AlreadyDefaulted,
    #[error("loan terms need a positive principal and duration")]
    BadTerms,
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
