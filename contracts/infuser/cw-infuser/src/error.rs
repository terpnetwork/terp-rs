use std::fmt;

use cosmwasm_std::{CheckedMultiplyRatioError, Coin, Instantiate2AddressError, StdError};
use cw_controllers::AdminError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Admin(#[from] AdminError),

    #[error("{0}")]
    CheckedMultiplyRatioError(#[from] CheckedMultiplyRatioError),

    #[error("Fee payment not accepted. Ensure you are sending the correct amount.")]
    FeeNotAccepted { have: Coin, want: Coin },

    #[error("{0}")]
    Instantiate2AddressError(#[from] Instantiate2AddressError),

    #[error("The Message sender has to be the owner of the NFT to prevent hacks")]
    SenderNotOwner {},

    #[error("Cannot specify the same contract address more than once")]
    DuplicateCollectionInInfusion,

    #[error("Cannot currently set identical fee sub tokens for eligible collections: {token}")]
    DuplicateFeeSubToken { token: String },

    #[error("NftIsNotEligible: {col}")]
    NftIsNotEligible { col: String },

    #[error("Bundle of type {bun_type} povided does not contain any nfts for collection: {col}. wavs_enabled: {wavs}, min_req: {min_req}")]
    BundleCollectionNotEligilbe {
        bun_type: i32,
        col: String,
        wavs: bool,
        min_req: u64,
    },

    #[error("Bundle Not Accepted. Have:{have}. Want: {want}, Contract: {addr}")]
    BundleNotAccepted { have: u64, want: u64, addr: String },

    #[error("Bundle Not Accepted. Burnt Record:{have}. Minimum Required: {need}")]
    WavsBundleNotAccepted { have: u64, need: u64 },

    #[error("Bundle cannot be empty.")]
    EmptyBundle,

    #[error("Bundle type AnyOf must only contain atleast 1 instance of any eligible collection")]
    AnyOfConfigError { err: AnyOfErr },

    #[error("Bundle type AnyOfBlend has an incorrect setup.")]
    AnyOfBlendConfigError,

    #[error("Max metadata array length is 4")]
    MetadataArrayLengthError,

    #[error("Invalid base token URI (must be an IPFS URI)")]
    InvalidBaseTokenURI {},

    #[error("InvalidAnyOfBundle")]
    InvalidAnyOfBundle {},

    #[error("Token id: {token_id} already sold")]
    TokenIdAlreadySold { token_id: u32 },

    #[error("Sold out")]
    SoldOut {},

    #[error("BundleCollectionContractEmpty")]
    BundleCollectionContractEmpty {},

    #[error("FeeSubNotProvided: {col}, want: {want}")]
    FeeSubNotProvided { col: String, want: Coin },

    #[error("payment substitute is enabled for collection {col}, but did not recieve tokens or payment. Have: {have}. Want: {want}")]
    PaymentSubstituteNotProvided { col: String, have: Coin, want: Coin },

    #[error("Too many NFT collections being set for infusion. Have: {have}.  Max: {max}")]
    TooManyCollectionsInInfusion { have: u64, max: u64 },

    #[error("Not enough bundles in nft. Collection: {col} Have: {have}. Min: {min}, Max: {max}")]
    NotEnoughNFTsInBundle {
        col: String,
        have: u64,
        min: u64,
        max: u64,
    },

    #[error("Too many infusions specified. Have: {have}. Min: {min}, Max: {max}")]
    BadBundle { have: u64, min: u64, max: u64 },

    #[error("Too many infusions specified.")]
    MaxInfusionsError,

    #[error("The max_bundles being set is greater than possible. Current hard-coded at 5.")]
    MaxBundleError,

    #[error("You are attempting to create more bundles than possible. Current hard-coded limit set at 2")]
    MaxInfusionErrror,

    #[error("New infusion require a minimum fee of {min} to be created.")]
    InfusionFeeLessThanMinimumRequired { min: Coin },

    #[error("RequirednfusionFeeError: New infusion fee of {fee} required not sent. Retry infusion creation with correct funds.")]
    RequirednfusionFeeError { fee: Coin },

    #[error("contract: {addr} is not an cw721 nft collection.")]
    AddrIsNotNFTCol { addr: String },

    #[error("You cannot set the infusion fee as 0. Omit this value from the create_infsuion message to disable infusion fee requirements.")]
    InfusionFeeCannotbeZero,

    #[error("Cannot set infused collection as sg.")]
    UnauthorizedSg,

    #[error("Unauthorized.")]
    Unauthorized,

    #[error("InfusionIsEnded.")]
    InfusionIsEnded,

    #[error("InfusionDescriptionLengthError")]
    InfusionDescriptionLengthError,

    #[error("untriggered")]
    UnTriggered,

    #[error("MigrationError")]
    MigrationError,

    #[error("you have found a contract feature currently unimplemented! dm me with the words `eretskableret - jroc`.")]
    UnImplemented,
}

#[cosmwasm_schema::cw_serde]
#[derive(Error)]
pub enum AnyOfErr {
    Uneligible,
    Empty,
}

impl fmt::Display for AnyOfErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AnyOfErr::Uneligible => write!(f, "Uneligible error"),
            AnyOfErr::Empty => write!(f, "Empty error"),
            // Add cases for other variants as needed
        }
    }
}
