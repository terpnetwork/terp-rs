use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Coin, Decimal, Uint128};

use crate::state::{Auction, Config, HaltManager};

#[cw_serde]
pub struct InstantiateMsg {
    pub fee_manager: String,
    /// Fraction of the winning bid paid as the protocol fee (for example 0.02 = 2%).
    pub trading_fee_percent: Decimal,
    /// Each later bid must be at least this fraction above the previous bid.
    pub min_bid_increment_percent: Decimal,
    pub min_duration: u64,
    pub max_duration: u64,
    pub extend_duration: u64,
    /// Exact coin charged when creating an auction. Zero amount means no fee.
    pub create_auction_fee: Coin,
    pub max_auctions_to_settle_per_block: u64,
    pub halt_duration_threshold: u64,
    pub halt_buffer_duration: u64,
    pub halt_postpone_duration: u64,
    /// Share of the protocol fee that is burned. The rest is sent to `fee_manager`.
    pub burn_bps: u64,
    pub min_reserve_prices: Vec<Coin>,
}

/// svg-auction sends this inside `SendNft`. `duration` is optional; `is_instant_sale` defaults to false.
#[cw_serde]
pub struct Cw721ReceiveMsg {
    pub sender: String,
    pub token_id: String,
    pub msg: Binary,
}

#[cw_serde]
pub enum HookMsg {
    CreateAuction {
        denom: String,
        reserve_price: Uint128,
        #[serde(default)]
        duration: Option<u64>,
        #[serde(default)]
        is_instant_sale: bool,
    },
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    CreateAuction {
        collection: String,
        token_id: String,
        reserve_price: Coin,
        duration: u64,
        seller_funds_recipient: Option<String>,
        #[serde(default)]
        instant_sale: bool,
    },
    UpdateReservePrice {
        collection: String,
        token_id: String,
        reserve_price: Coin,
    },
    CancelAuction {
        collection: String,
        token_id: String,
    },
    PlaceBid {
        collection: String,
        token_id: String,
    },
    SettleAuction {
        collection: String,
        token_id: String,
    },
    ReceiveNft(Cw721ReceiveMsg),
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
    #[returns(HaltManager)]
    HaltManager {},
    #[returns(Option<Auction>)]
    Auction {
        collection: String,
        token_id: String,
    },
}

#[cw_serde]
pub enum SudoMsg {
    BeginBlock {},
    EndBlock {},
    SetMinReservePrices { min_reserve_prices: Vec<Coin> },
    UnsetMinReservePrices { denoms: Vec<String> },
}
