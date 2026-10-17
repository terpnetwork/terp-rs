use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure, Addr, Storage, Uint256};
use cw_storage_plus::{Index, IndexList, IndexedMap, Item, Map, MultiIndex};

use crate::helpers::collection_token_index;
use crate::orders::{Ask, Bid, CollectionBid};
use crate::ContractError;

pub type OrderId = String;
pub type TokenId = String;
pub type Denom = String;

pub const MAX_BASIS_POINTS: u64 = 10_000;

#[cw_serde]
pub struct Config {
    pub fee_manager: Addr,
    pub royalty_registry: Option<Addr>,
    pub protocol_fee_bps: u64,
    pub max_royalty_fee_bps: u64,
    pub maker_reward_bps: u64,
    pub taker_reward_bps: u64,
    pub burn_bps: u64,
    pub default_denom: Denom,
}

impl Config {
    pub fn save(&self, storage: &mut dyn Storage) -> Result<(), ContractError> {
        ensure!(
            self.protocol_fee_bps <= MAX_BASIS_POINTS,
            ContractError::InvalidInput("protocol_fee_bps must be <= 10000".into())
        );
        ensure!(
            self.max_royalty_fee_bps <= MAX_BASIS_POINTS,
            ContractError::InvalidInput("max_royalty_fee_bps must be <= 10000".into())
        );
        ensure!(
            self.protocol_fee_bps + self.max_royalty_fee_bps <= MAX_BASIS_POINTS,
            ContractError::InvalidInput(
                "protocol fee plus max royalty must be <= 10000 bps".into(),
            )
        );
        ensure!(
            self.maker_reward_bps + self.taker_reward_bps <= MAX_BASIS_POINTS,
            ContractError::InvalidInput(
                "maker and taker reward bps must be <= 10000 combined".into(),
            )
        );
        ensure!(
            self.burn_bps <= MAX_BASIS_POINTS,
            ContractError::InvalidInput("burn_bps must be <= 10000".into())
        );
        CONFIG.save(storage, self)?;
        Ok(())
    }
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const COLLECTION_DENOMS: Map<Addr, Denom> = Map::new("collection_denoms");
pub const LISTING_FEES: Map<Denom, Uint256> = Map::new("listing_fees");
pub const NONCE: Item<u64> = Item::new("nonce");

pub struct AskIndexes<'a> {
    pub collection_denom_price: MultiIndex<'a, (Addr, Denom, u128), Ask, OrderId>,
    pub creator_collection: MultiIndex<'a, (Addr, Addr), Ask, OrderId>,
}

impl IndexList<Ask> for AskIndexes<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<Ask>> + '_> {
        let v: Vec<&dyn Index<Ask>> = vec![&self.collection_denom_price, &self.creator_collection];
        Box::new(v.into_iter())
    }
}

pub fn asks<'a>() -> IndexedMap<OrderId, Ask, AskIndexes<'a>> {
    let indexes = AskIndexes {
        collection_denom_price: MultiIndex::new(
            |_pk, ask: &Ask| {
                (
                    ask.collection.clone(),
                    ask.terms.price.denom.clone(),
                    crate::helpers::amount_u128(ask.terms.price.amount).unwrap_or(u128::MAX),
                )
            },
            "asks",
            "asks__price",
        ),
        creator_collection: MultiIndex::new(
            |_pk, ask: &Ask| (ask.creator.clone(), ask.collection.clone()),
            "asks",
            "asks__creator",
        ),
    };
    IndexedMap::new("asks", indexes)
}

pub struct BidIndexes<'a> {
    pub token_denom_price: MultiIndex<'a, (String, Denom, u128), Bid, OrderId>,
    pub creator_collection: MultiIndex<'a, (Addr, Addr), Bid, OrderId>,
}

impl IndexList<Bid> for BidIndexes<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<Bid>> + '_> {
        let v: Vec<&dyn Index<Bid>> = vec![&self.token_denom_price, &self.creator_collection];
        Box::new(v.into_iter())
    }
}

pub fn bids<'a>() -> IndexedMap<OrderId, Bid, BidIndexes<'a>> {
    let indexes = BidIndexes {
        token_denom_price: MultiIndex::new(
            |_pk, bid: &Bid| {
                (
                    collection_token_index(bid.collection.as_ref(), &bid.token_id),
                    bid.terms.price.denom.clone(),
                    crate::helpers::amount_u128(bid.terms.price.amount).unwrap_or(u128::MAX),
                )
            },
            "bids",
            "bids__price",
        ),
        creator_collection: MultiIndex::new(
            |_pk, bid: &Bid| (bid.creator.clone(), bid.collection.clone()),
            "bids",
            "bids__creator",
        ),
    };
    IndexedMap::new("bids", indexes)
}

pub struct CollectionBidIndexes<'a> {
    pub collection_denom_price: MultiIndex<'a, (Addr, Denom, u128), CollectionBid, OrderId>,
    pub creator_collection: MultiIndex<'a, (Addr, Addr), CollectionBid, OrderId>,
}

impl IndexList<CollectionBid> for CollectionBidIndexes<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<CollectionBid>> + '_> {
        let v: Vec<&dyn Index<CollectionBid>> =
            vec![&self.collection_denom_price, &self.creator_collection];
        Box::new(v.into_iter())
    }
}

pub fn collection_bids<'a>() -> IndexedMap<OrderId, CollectionBid, CollectionBidIndexes<'a>> {
    let indexes = CollectionBidIndexes {
        collection_denom_price: MultiIndex::new(
            |_pk, bid: &CollectionBid| {
                (
                    bid.collection.clone(),
                    bid.terms.price.denom.clone(),
                    crate::helpers::amount_u128(bid.terms.price.amount).unwrap_or(u128::MAX),
                )
            },
            "col_bids",
            "col_bids__price",
        ),
        creator_collection: MultiIndex::new(
            |_pk, bid: &CollectionBid| (bid.creator.clone(), bid.collection.clone()),
            "col_bids",
            "col_bids__creator",
        ),
    };
    IndexedMap::new("col_bids", indexes)
}
