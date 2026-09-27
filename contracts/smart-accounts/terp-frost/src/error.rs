use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("group_vk is not a secp256k1 point: {group_vk}")]
    GroupVkNotPoint { group_vk: String },

    #[error("frost signature length {got}, need {need}")]
    BadSignatureLength { got: usize, need: usize },

    #[error("frost signature R is not a secp256k1 point")]
    SignatureRNotPoint,

    #[error("frost signature z is not a canonical scalar")]
    SignatureZNotScalar,

    #[error("frost signature rejected for group_vk {group_vk} message_len {message_len}")]
    SignatureRejected {
        group_vk: String,
        message_len: usize,
    },

    #[error("empty sign doc")]
    EmptySignDoc,

    #[error("authenticator params are not the current group point for group_vk {group_vk}")]
    AuthenticatorParamsMismatch { group_vk: String },
}
