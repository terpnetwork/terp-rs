use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Coin, Decimal, Env, Storage, Timestamp, Uint256};
use cw_storage_plus::{Item, Map};
use nftfi_utils::{AssetInfo, Locks};

use crate::error::ContractError;

pub const CONFIG: Item<Config> = Item::new("config");
pub const COLLATERAL: Map<(Addr, u64), CollateralInfo> = Map::new("collateral");
pub const BORROWER: Map<Addr, BorrowerInfo> = Map::new("borrower");
pub const OFFERS: Map<u64, OfferInfo> = Map::new("offers");

#[cw_serde]
pub struct Config {
    pub name: String,
    pub owner: Addr,
    pub treasury_addr: Addr,
    /// Share of interest paid to the treasury when the borrower repays.
    pub fee_rate: Decimal,
    pub listing_fee_coins: Vec<Coin>,
    pub global_offer_index: u64,
    pub locks: Locks,
}

#[cw_serde]
#[derive(Default)]
pub struct BorrowerInfo {
    pub last_collateral_id: u64,
    pub listed: bool,
}

#[cw_serde]
pub struct LoanTerms {
    pub principle: Coin,
    pub interest: Uint256,
    pub duration_in_blocks: u64,
}

impl LoanTerms {
    pub fn check(&self) -> Result<(), ContractError> {
        if self.principle.amount.is_zero()
            || self.principle.denom.is_empty()
            || self.duration_in_blocks == 0
        {
            return Err(ContractError::BadTerms);
        }
        Ok(())
    }
}

#[cw_serde]
pub enum LoanState {
    Published,
    Started,
    Defaulted,
    Ended,
    Inactive,
}

#[cw_serde]
pub enum OfferState {
    Published,
    Accepted,
    Refused,
    Cancelled,
}

#[cw_serde]
pub struct CollateralInfo {
    pub terms: Option<LoanTerms>,
    pub associated_assets: Vec<AssetInfo>,
    pub list_date: Timestamp,
    pub state: LoanState,
    pub offer_amount: u64,
    pub active_offer: Option<u64>,
    pub start_block: Option<u64>,
    pub comment: Option<String>,
}

#[cw_serde]
pub struct OfferInfo {
    pub lender: Addr,
    pub borrower: Addr,
    pub loan_id: u64,
    pub offer_id: u64,
    pub terms: LoanTerms,
    pub state: OfferState,
    pub list_date: Timestamp,
    pub deposited_funds: Option<Coin>,
    pub comment: Option<String>,
}

pub fn load_collateral(
    storage: &dyn Storage,
    borrower: &Addr,
    loan_id: u64,
) -> Result<CollateralInfo, ContractError> {
    COLLATERAL
        .may_load(storage, (borrower.clone(), loan_id))?
        .ok_or(ContractError::NotFound)
}

pub fn load_offer(storage: &dyn Storage, offer_id: u64) -> Result<OfferInfo, ContractError> {
    OFFERS
        .may_load(storage, offer_id)?
        .ok_or(ContractError::OfferNotFound)
}

pub fn active_offer(
    storage: &dyn Storage,
    collateral: &CollateralInfo,
) -> Result<OfferInfo, ContractError> {
    let id = collateral.active_offer.ok_or(ContractError::OfferNotFound)?;
    load_offer(storage, id)
}

/// Defaulted once the start block plus the duration is strictly behind the current height.
pub fn is_defaulted(env: &Env, collateral: &CollateralInfo, offer: &OfferInfo) -> bool {
    match collateral.state {
        LoanState::Defaulted => true,
        LoanState::Started => {
            let start = collateral.start_block.unwrap_or(env.block.height);
            start + offer.terms.duration_in_blocks < env.block.height
        }
        _ => false,
    }
}
