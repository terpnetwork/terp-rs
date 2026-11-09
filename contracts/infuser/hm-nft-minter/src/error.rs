use cosmwasm_std::StdError;
use cw_ownable::OwnershipError;
use hm_loyalty_whitelist::WhitelistError;
use hm_nft_oracle::OracleError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Ownership(#[from] OwnershipError),

    #[error("{0}")]
    Whitelist(#[from] WhitelistError),

    #[error("{0}")]
    Oracle(#[from] OracleError),

    #[error("unauthorized")]
    Unauthorized {},

    #[error("contract is paused")]
    Paused {},

    #[error("sold out: max_tokens={max_tokens}")]
    SoldOut { max_tokens: u32 },

    #[error("cw721 collection not linked")]
    CollectionNotLinked {},

    #[error("cw721 collection already linked")]
    CollectionAlreadyLinked {},

    #[error("invalid max_tokens (must be > 0)")]
    InvalidMaxTokens {},

    #[error("invalid token_uri policy")]
    InvalidTokenUri {},
}

impl PartialEq for ContractError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Whitelist(a), Self::Whitelist(b)) => a == b,
            (Self::Oracle(a), Self::Oracle(b)) => a == b,
            (Self::Unauthorized {}, Self::Unauthorized {}) => true,
            (Self::Paused {}, Self::Paused {}) => true,
            (Self::SoldOut { max_tokens: a }, Self::SoldOut { max_tokens: b }) => a == b,
            (Self::CollectionNotLinked {}, Self::CollectionNotLinked {}) => true,
            (Self::CollectionAlreadyLinked {}, Self::CollectionAlreadyLinked {}) => true,
            (Self::InvalidMaxTokens {}, Self::InvalidMaxTokens {}) => true,
            (Self::InvalidTokenUri {}, Self::InvalidTokenUri {}) => true,
            // Std / Ownership: compare Display only (upstream types are not PartialEq).
            (Self::Std(a), Self::Std(b)) => a.to_string() == b.to_string(),
            (Self::Ownership(a), Self::Ownership(b)) => a.to_string() == b.to_string(),
            _ => false,
        }
    }
}
