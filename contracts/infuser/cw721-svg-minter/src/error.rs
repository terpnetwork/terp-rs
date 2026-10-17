use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Ownership(#[from] cw_ownable::OwnershipError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Instantiate2 address error: {msg}")]
    Instantiate2AddressError { msg: String },

    #[error("{0}")]
    Svg(#[from] cw_svg::SvgError),
}
