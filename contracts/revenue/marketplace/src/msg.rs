use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Coin, Uint128};

use crate::orders::{Ask, Bid, CollectionBid, OrderTerms};
use crate::state::{Config, Denom, OrderId, TokenId};
use crate::ContractError;

#[cw_serde]
pub struct InstantiateMsg {
    pub fee_manager: String,
    /// Optional Stargaze-style royalty registry. cw721 collection royalties are used when this is unset or returns nothing.
    pub royalty_registry: Option<String>,
    pub protocol_fee_bps: u64,
    pub max_royalty_fee_bps: u64,
    /// Finder reward as a fraction of the protocol fee, in basis points.
    pub maker_reward_bps: u64,
    pub taker_reward_bps: u64,
    /// Share of the remaining protocol fee (and of listing fees) that is burned. The rest goes to `fee_manager`.
    pub burn_bps: u64,
    pub default_denom: String,
}

/// Message-level order terms. Addresses are validated before storage.
#[cw_serde]
pub struct OrderDetails {
    pub price: Coin,
    pub recipient: Option<String>,
    pub finder: Option<String>,
}

impl OrderDetails {
    pub fn validate(self, api: &dyn cosmwasm_std::Api) -> Result<OrderTerms, ContractError> {
        Ok(OrderTerms {
            price: self.price,
            recipient: cw_utils::maybe_addr(api, self.recipient)?,
            finder: cw_utils::maybe_addr(api, self.finder)?,
        })
    }
}

#[cw_serde]
pub struct Cw721ReceiveMsg {
    pub sender: String,
    pub token_id: String,
    pub msg: Binary,
}

/// Hook payload inside `ReceiveNft`. `CreateAuction` matches preview `svg-auction`.
#[cw_serde]
pub enum HookMsg {
    CreateAuction {
        denom: String,
        reserve_price: Uint128,
        is_instant_sale: bool,
    },
    SetAsk {
        details: OrderDetails,
    },
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    UpdateConfig {
        config: InstantiateMsg,
    },
    UpdateCollectionDenom {
        collection: String,
        denom: Denom,
    },
    SetListingFee {
        fee: Coin,
    },
    RemoveListingFee {
        denom: Denom,
    },
    SetAsk {
        collection: String,
        token_id: TokenId,
        details: OrderDetails,
    },
    RemoveAsk {
        id: OrderId,
    },
    UpdateAsk {
        id: OrderId,
        details: OrderDetails,
    },
    /// Buy a listed ask. `details.price` is the maximum the buyer will pay. The ask price is charged.
    AcceptAsk {
        id: OrderId,
        details: OrderDetails,
    },
    SetBid {
        collection: String,
        token_id: TokenId,
        details: OrderDetails,
    },
    RemoveBid {
        id: OrderId,
    },
    UpdateBid {
        id: OrderId,
        details: OrderDetails,
    },
    AcceptBid {
        id: OrderId,
        details: OrderDetails,
    },
    SetCollectionBid {
        collection: String,
        details: OrderDetails,
    },
    RemoveCollectionBid {
        id: OrderId,
    },
    UpdateCollectionBid {
        id: OrderId,
        details: OrderDetails,
    },
    AcceptCollectionBid {
        id: OrderId,
        token_id: TokenId,
        details: OrderDetails,
    },
    /// Set an ask and revert if nothing matches.
    SellNft {
        collection: String,
        token_id: TokenId,
        details: OrderDetails,
    },
    /// Bid on a token and revert if no ask matches.
    BuySpecificNft {
        collection: String,
        token_id: TokenId,
        details: OrderDetails,
    },
    /// Collection bid that reverts if no ask matches.
    BuyCollectionNft {
        collection: String,
        details: OrderDetails,
    },
    ReceiveNft(Cw721ReceiveMsg),
}

#[cw_serde]
pub struct PriceOffset {
    pub id: OrderId,
    pub amount: u128,
}

#[cw_serde]
pub enum BoundOffset<T> {
    Inclusive(T),
    Exclusive(T),
}

#[cw_serde]
pub struct QueryOptions<T> {
    pub descending: Option<bool>,
    pub limit: Option<u32>,
    pub min: Option<BoundOffset<T>>,
    pub max: Option<BoundOffset<T>>,
}

impl<T> Default for QueryOptions<T> {
    fn default() -> Self {
        Self {
            descending: None,
            limit: None,
            min: None,
            max: None,
        }
    }
}

impl<T> QueryOptions<T> {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(30).min(100) as usize
    }

    pub fn order(&self) -> cosmwasm_std::Order {
        if self.descending.unwrap_or(false) {
            cosmwasm_std::Order::Descending
        } else {
            cosmwasm_std::Order::Ascending
        }
    }
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
    #[returns(Option<Denom>)]
    CollectionDenom { collection: String },
    #[returns(Option<Ask>)]
    Ask { id: String },
    #[returns(Vec<Ask>)]
    Asks { ids: Vec<String> },
    #[returns(Vec<Ask>)]
    AsksByCollectionDenom {
        collection: String,
        denom: Denom,
        query_options: Option<QueryOptions<PriceOffset>>,
    },
    #[returns(Vec<Ask>)]
    AsksByCreatorCollection {
        creator: String,
        collection: String,
        query_options: Option<QueryOptions<String>>,
    },
    #[returns(Option<Bid>)]
    Bid { id: String },
    #[returns(Vec<Bid>)]
    Bids { ids: Vec<String> },
    #[returns(Vec<Bid>)]
    BidsByTokenPrice {
        collection: String,
        token_id: TokenId,
        denom: Denom,
        query_options: Option<QueryOptions<PriceOffset>>,
    },
    #[returns(Vec<Bid>)]
    BidsByCreatorCollection {
        creator: String,
        collection: String,
        query_options: Option<QueryOptions<String>>,
    },
    #[returns(Option<CollectionBid>)]
    CollectionBid { id: String },
    #[returns(Vec<CollectionBid>)]
    CollectionBids { ids: Vec<String> },
    #[returns(Vec<CollectionBid>)]
    CollectionBidsByPrice {
        collection: String,
        denom: Denom,
        query_options: Option<QueryOptions<PriceOffset>>,
    },
    #[returns(Vec<CollectionBid>)]
    CollectionBidsByCreatorCollection {
        creator: String,
        collection: String,
        query_options: Option<QueryOptions<String>>,
    },
}

