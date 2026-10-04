use cosmwasm_std::Addr;
use cw_controllers::Hooks;
use cw_storage_plus::Item;
use terp_account::manifold::{Config, SudoParams};

use cosmwasm_std::{StdResult, Storage};
use cw_storage_macro::index_list;
use cw_storage_plus::{IndexedMap, Map, MultiIndex, UniqueIndex};
use terp_account::TokenId;
use terp_account::{Ask, AskKey, Bid, BidKey, PendingBid};

pub const SUDO_PARAMS: Item<SudoParams> = Item::new("sp");

pub const ACCOUNT_COLLECTION: Item<Addr> = Item::new("ac");

/// Controls if minting is paused or not by admin
pub const PAUSED: Item<bool> = Item::new("paused");

pub const CONFIG: Item<Config> = Item::new("config");

// bps fee can not exceed 100%
pub const MAX_FEE_BPS: u64 = 10000;

pub const MAX_REMOVE_BID_LIMIT: u64 = 30;
pub const COOLDOWN_BID: Map<&TokenId, PendingBid> = Map::new("cdb");
pub const OVERFLOW_BIDS_REMOVE: Map<TokenId, Vec<BidKey>> = Map::new("obr");

pub const ASK_HOOKS: Hooks = Hooks::new("ah");
pub const BID_HOOKS: Hooks = Hooks::new("bh");
pub const SALE_HOOKS: Hooks = Hooks::new("sh");

pub const VERSION_CONTROL: Item<Addr> = Item::new("vc");

pub const ASK_COUNT: Item<u32> = Item::new("ask-count");
pub const IS_SETUP: Item<bool> = Item::new("is");

pub fn ask_count(storage: &dyn Storage) -> StdResult<u32> {
    Ok(ASK_COUNT.may_load(storage)?.unwrap_or_default())
}

pub fn increment_asks(storage: &mut dyn Storage) -> StdResult<u32> {
    let val = ask_count(storage)? + 1;
    ASK_COUNT.save(storage, &val)?;
    Ok(val)
}

pub fn decrement_asks(storage: &mut dyn Storage) -> StdResult<u32> {
    let val = ask_count(storage)? - 1;
    ASK_COUNT.save(storage, &val)?;
    Ok(val)
}

/// Convenience ask key constructor
pub fn ask_key(token_id: &str) -> AskKey {
    token_id.to_string()
}

/// Defines indices for accessing Asks
#[index_list(Ask)]
pub struct AskIndicies<'a> {
    /// Unique incrementing id for each ask
    /// This allows pagination when `token_id`s are strings
    pub id: UniqueIndex<'a, u32, Ask, AskKey>,
    /// Index by seller
    pub seller: MultiIndex<'a, Addr, Ask, AskKey>,
}

pub fn asks<'a>() -> IndexedMap<AskKey, Ask, AskIndicies<'a>> {
    let indexes = AskIndicies {
        id: UniqueIndex::new(|d| d.id, "ask__id"),
        seller: MultiIndex::new(
            |_pk: &[u8], d: &Ask| d.seller.clone(),
            "asks",
            "asks__seller",
        ),
    };
    IndexedMap::new("asks", indexes)
}

/// Convenience bid key constructor
pub fn bid_key(token_id: &str, bidder: &Addr) -> BidKey {
    (token_id.to_string(), bidder.clone())
}

/// Defines indices for accessing bids
#[index_list(Bid)]
pub struct BidIndicies<'a> {
    pub bidder: MultiIndex<'a, Addr, Bid, BidKey>,
    pub price: MultiIndex<'a, (String, u128), Bid, BidKey>,
    pub created_time: MultiIndex<'a, (String, u64), Bid, BidKey>,
}

pub fn bids<'a>() -> IndexedMap<BidKey, Bid, BidIndicies<'a>> {
    let indexes = BidIndicies {
        bidder: MultiIndex::new(|_pk: &[u8], b: &Bid| b.bidder.clone(), "b2", "b2__b"),
        price: MultiIndex::new(
            |_pk: &[u8], b: &Bid| (b.token_id.clone(), b.amount.u128()),
            "b2", // Change this to match the primary key namespace
            "b2__price",
        ),
        created_time: MultiIndex::new(
            |_pk: &[u8], b: &Bid| (b.token_id.clone(), b.created_time.seconds()),
            "b2", // Change this to match the primary key namespace
            "b2__time",
        ),
    };
    IndexedMap::new("b2", indexes)
}
