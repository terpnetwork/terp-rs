use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Coin, Decimal};
use nftfi_utils::AssetInfo;

use crate::state::{CollateralInfo, Config, LoanState, LoanTerms, OfferInfo};

#[cw_serde]
pub struct InstantiateMsg {
    pub name: String,
    pub owner: Option<String>,
    pub treasury_addr: String,
    pub fee_rate: Decimal,
    pub listing_fee_coins: Option<Vec<Coin>>,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    #[cw_orch(payable)]
    ListCollaterals {
        tokens: Vec<AssetInfo>,
        terms: Option<LoanTerms>,
        comment: Option<String>,
    },
    WithdrawCollaterals {
        loan_id: u64,
    },
    #[cw_orch(payable)]
    MakeOffer {
        borrower: String,
        loan_id: u64,
        terms: LoanTerms,
        comment: Option<String>,
        on_behalf_of: Option<String>,
    },
    CancelOffer {
        global_offer_id: String,
    },
    RefuseOffer {
        global_offer_id: String,
    },
    WithdrawRefusedOffer {
        global_offer_id: String,
    },
    AcceptOffer {
        global_offer_id: String,
    },
    #[cw_orch(payable)]
    AcceptLoan {
        borrower: String,
        loan_id: u64,
        comment: Option<String>,
    },
    #[cw_orch(payable)]
    Repay {
        loan_id: u64,
    },
    WithdrawDefaulted {
        borrower: String,
        loan_id: u64,
    },
    ToggleLock {
        lock: bool,
    },
    SetOwner {
        owner: String,
    },
    SetTreasury {
        treasury_addr: String,
    },
    SetFeeRate {
        fee_rate: Decimal,
    },
    SetListingFee {
        listing_fee_coins: Vec<Coin>,
    },
}

#[cw_serde]
pub enum SudoMsg {
    ToggleLock { lock: bool },
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
    #[returns(CollateralResponse)]
    Collateral { borrower: String, loan_id: u64 },
    #[returns(OfferResponse)]
    Offer { global_offer_id: String },
}

#[cw_serde]
pub struct CollateralResponse {
    pub borrower: String,
    pub loan_id: u64,
    pub state: LoanState,
    pub collateral: CollateralInfo,
}

#[cw_serde]
pub struct OfferResponse {
    pub global_offer_id: String,
    pub offer: OfferInfo,
}
