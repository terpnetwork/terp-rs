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

    #[error("insufficient funding: sent {sent}{denom}, required {required}{denom}")]
    InsufficientFunding {
        denom: String,
        sent: cosmwasm_std::Uint256,
        required: cosmwasm_std::Uint256,
    },

    #[error("Contract not found")]
    ContractNotFound {},

    #[error("Instantiation failed")]
    InstantiationFailed {},

    #[error("Unknown reply id: {id}")]
    UnknownReplyId { id: u64 },
}

pub type ContractResult<T> = Result<T, ContractError>;
