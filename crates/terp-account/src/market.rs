//! NFT marketplace types for account billboards. Compiled only with `full`.

use cosmwasm_std::{Addr, Coin, Deps, Env, MessageInfo, StdError, Timestamp, Uint128};
use cw721::error::Cw721ContractError;
use cw721::traits::{Contains, Cw721CustomMsg, Cw721State, StateFactory};

pub const DEFAULT_QUERY_LIMIT: u32 = 10;
pub const MAX_QUERY_LIMIT: u32 = 100;
pub const MAX_TEXT_LENGTH: u32 = 512;
pub const NATIVE_DENOM: &str = "uthiol";
pub const SECONDS_PER_YEAR: u64 = 31536000;
pub const TERP_PREFIX: &str = "terp";
pub const CURRENT_COOLDOWN_FEE: u64 = 1_000_000_000u64;
pub const CURRENT_MINIMUM_BID_PRICE: u64 = 1_000_000_000u64;
pub const CURRENT_BASE_PRICE: u64 = 710_000_000u64;
pub const CURRENT_BASE_DELEGATION: u64 = 1_000_000_000u64;
pub const DEPLOYMENT_DAO: &str = "terp13hmdq0slwmff7sej79kfa8mgnx4rl46nj2fvmlgu6u32tz6vfqes8hsya0";

pub type Collection = String;
pub type Bidder = String;
pub type Seller = String;
pub type TokenId = String;
pub type AskKey = TokenId;
pub type BidKey = (TokenId, Addr);
pub type Id = u32;

#[cosmwasm_schema::cw_serde]
pub struct Ask {
    pub token_id: TokenId,
    pub id: u32,
    pub seller: Addr,
}

#[cosmwasm_schema::cw_serde]
pub struct Bid {
    pub token_id: TokenId,
    pub bidder: Addr,
    pub amount: Uint128,
    pub created_time: Timestamp,
}

impl Bid {
    pub fn new(token_id: &str, bidder: Addr, amount: Uint128, created_time: Timestamp) -> Self {
        Bid {
            token_id: token_id.to_string(),
            bidder,
            amount,
            created_time,
        }
    }
}

#[cosmwasm_schema::cw_serde]
pub struct PendingBid {
    pub ask: Ask,
    pub new_owner: Addr,
    pub amount: Uint128,
    pub unlock_time: Timestamp,
}

impl PendingBid {
    pub fn new(ask: Ask, new_owner: Addr, amount: Uint128, unlock_time: Timestamp) -> Self {
        PendingBid {
            ask,
            new_owner,
            amount,
            unlock_time,
        }
    }
}

#[cosmwasm_schema::cw_serde]
#[derive(Default)]
pub struct Metadata {
    pub account_ownership: bool,
    pub image_nft: Option<NFT>,
    pub records: Vec<TextRecord>,
}

impl Metadata {
    pub fn into_json_string(&self) -> Result<String, cosmwasm_std::StdError> {
        let json_vec = cosmwasm_std::to_json_vec(self)?;
        String::from_utf8(json_vec).map_err(cosmwasm_std::StdError::from)
    }

    pub fn default_with_account() -> Self {
        Self {
            account_ownership: true,
            image_nft: None,
            records: vec![],
        }
    }
}

impl Cw721State for Metadata {}
impl Cw721CustomMsg for Metadata {}

impl Contains for Metadata {
    fn contains(&self, _other: &Self) -> bool {
        true
    }
}

impl StateFactory<Metadata> for Metadata {
    fn create(
        &self,
        _deps: Deps,
        _env: &Env,
        _info: Option<&MessageInfo>,
        _current: Option<&Metadata>,
    ) -> Result<Metadata, Cw721ContractError> {
        Ok(self.clone())
    }

    fn validate(
        &self,
        _deps: Deps,
        _env: &Env,
        _info: Option<&MessageInfo>,
        _current: Option<&Metadata>,
    ) -> Result<(), Cw721ContractError> {
        Ok(())
    }
}

#[cosmwasm_schema::cw_serde]
pub struct NFT {
    pub collection: cosmwasm_std::Addr,
    pub token_id: TokenId,
}

impl NFT {
    pub fn into_json_string(&self) -> String {
        String::from_utf8(cosmwasm_std::to_json_vec(self).unwrap_or_default()).unwrap_or_default()
    }

    pub fn new(
        deps: cosmwasm_std::Deps,
        collection: String,
        token_id: String,
    ) -> cosmwasm_std::StdResult<Self> {
        Ok(Self {
            collection: deps.api.addr_validate(&collection)?,
            token_id,
        })
    }
}

#[cosmwasm_schema::cw_serde]
pub struct TextRecord {
    pub account: String,
    pub value: String,
    pub verified: Option<bool>,
}

impl TextRecord {
    pub fn new(account: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            value: value.into(),
            verified: None,
        }
    }

    pub fn into_json_string(&self) -> String {
        String::from_utf8(cosmwasm_std::to_json_vec(self).unwrap_or_default()).unwrap_or_default()
    }
}

pub fn charge_fees(res: &mut cosmwasm_std::Response, fee: cosmwasm_std::Uint256) {
    if fee > cosmwasm_std::Uint256::zero() {
        res.messages
            .push(cosmwasm_std::SubMsg::new(cosmwasm_std::BankMsg::Burn {
                amount: vec![Coin::new(fee, NATIVE_DENOM)],
            }));
    }
}

#[cfg(feature = "abstract")]
pub use aa::validate_aa_ownership;

#[cfg(feature = "abstract")]
mod aa {
    use super::*;
    use abstract_std::objects::gov_type::GovernanceDetails;
    use abstract_std::objects::ownership::Ownership;

    pub fn validate_aa_ownership(
        deps: Deps,
        aa_addr: &str,
        token_id: &str,
        contract_addr: &Addr,
        must_be_in_use: bool,
    ) -> Result<(), StdError> {
        let owner: Ownership<String> = deps
            .querier
            .query_wasm_smart(aa_addr, &abstract_std::account::QueryMsg::Ownership {})?;

        match &owner.owner {
            GovernanceDetails::NFT {
                collection_addr,
                token_id: owner_token_id,
            } => {
                let is_correct_nft =
                    owner_token_id == token_id && collection_addr == &contract_addr.to_string();
                match is_correct_nft == must_be_in_use {
                    true => Ok(()),
                    false => match must_be_in_use {
                        true => Err(StdError::msg(
                            "Abstract Account is not using this token as its ownership key.",
                        )),
                        false => Err(StdError::msg("Account is still mapped to EOA")),
                    },
                }
            }
            _ => match must_be_in_use {
                true => Err(StdError::msg("Account is not tokenized")),
                false => Ok(()),
            },
        }
    }
}
