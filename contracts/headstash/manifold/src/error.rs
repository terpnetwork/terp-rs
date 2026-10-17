use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Ownable(#[from] cw_ownable::OwnershipError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Invalid funding token")]
    InvalidFundingToken {},

    #[error("Contract not found")]
    ContractNotFound {},

    #[error("Instantiation failed")]
    InstantiationFailed {},

    #[error("Unknown reply id: {id}")]
    UnknownReplyId { id: u64 },
}

pub type ContractResult<T> = Result<T, ContractError>;
