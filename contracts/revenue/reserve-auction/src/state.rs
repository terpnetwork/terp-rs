use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure, Addr, Coin, Decimal, Storage, Timestamp, Uint256};
use cw_storage_plus::{Index, IndexList, IndexedMap, Item, Map, MultiIndex};

use crate::ContractError;

pub const MAX_BASIS_POINTS: u64 = 10_000;

#[cw_serde]
pub struct Config {
    pub fee_manager: Addr,
    pub trading_fee_percent: Decimal,
    pub min_bid_increment_percent: Decimal,
    pub min_duration: u64,
    pub max_duration: u64,
    pub extend_duration: u64,
    pub create_auction_fee: Coin,
    pub max_auctions_to_settle_per_block: u64,
    pub halt_duration_threshold: u64,
    pub halt_buffer_duration: u64,
    pub halt_postpone_duration: u64,
    pub burn_bps: u64,
}

impl Config {
    pub fn save(&self, storage: &mut dyn Storage) -> Result<(), ContractError> {
        ensure!(
            self.min_bid_increment_percent > Decimal::zero()
                && self.min_bid_increment_percent < Decimal::one(),
            ContractError::InvalidConfig(
                "min_bid_increment_percent must be greater than 0 and below 100%".into(),
            )
        );
        ensure!(
            self.trading_fee_percent <= Decimal::one(),
            ContractError::InvalidConfig("trading_fee_percent must be <= 100%".into())
        );
        ensure!(
            self.min_duration > 0 && self.max_duration >= self.min_duration,
            ContractError::InvalidConfig("duration bounds are invalid".into())
        );
        ensure!(
            self.extend_duration > 0,
            ContractError::InvalidConfig("extend_duration must be > 0".into())
        );
        ensure!(
            self.burn_bps <= MAX_BASIS_POINTS,
            ContractError::InvalidConfig("burn_bps must be <= 10000".into())
        );
        CONFIG.save(storage, self)?;
        Ok(())
    }
}

pub const CONFIG: Item<Config> = Item::new("cfg");
pub const MIN_RESERVE_PRICES: Map<String, Uint256> = Map::new("mrp");

#[cw_serde]
pub struct HighBid {
    pub coin: Coin,
    pub bidder: Addr,
}

#[cw_serde]
pub struct Auction {
    pub collection: Addr,
    pub token_id: String,
    pub seller: Addr,
    pub reserve_price: Coin,
    pub duration: u64,
    pub end_time: Option<Timestamp>,
    pub seller_funds_recipient: Option<Addr>,
    pub high_bid: Option<HighBid>,
    pub first_bid_time: Option<Timestamp>,
    /// First bid must equal the reserve and the auction settles in that transaction.
    #[serde(default)]
    pub instant_sale: bool,
}

impl Auction {
    pub fn denom(&self) -> String {
        self.reserve_price.denom.clone()
    }

    pub fn funds_recipient(&self) -> Addr {
        self.seller_funds_recipient
            .clone()
            .unwrap_or_else(|| self.seller.clone())
    }

    pub fn min_bid_amount(&self, increment: Decimal) -> Uint256 {
        match &self.high_bid {
            Some(high) => high.coin.amount.mul_ceil(Decimal::one() + increment),
            None => self.reserve_price.amount,
        }
    }
}

pub type AuctionKey = (Addr, String);

pub struct AuctionIndexes<'a> {
    pub seller: MultiIndex<'a, String, Auction, AuctionKey>,
    pub end_time: MultiIndex<'a, u64, Auction, AuctionKey>,
}

impl IndexList<Auction> for AuctionIndexes<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<Auction>> + '_> {
        let v: Vec<&dyn Index<Auction>> = vec![&self.seller, &self.end_time];
        Box::new(v.into_iter())
    }
}

pub fn auctions<'a>() -> IndexedMap<AuctionKey, Auction, AuctionIndexes<'a>> {
    let indexes = AuctionIndexes {
        seller: MultiIndex::new(
            |_pk, auction: &Auction| auction.seller.to_string(),
            "auctions",
            "auctions__seller",
        ),
        end_time: MultiIndex::new(
            |_pk, auction: &Auction| auction.end_time.map(|t| t.seconds()).unwrap_or(u64::MAX),
            "auctions",
            "auctions__end",
        ),
    };
    IndexedMap::new("auctions", indexes)
}

#[cw_serde]
pub struct HaltWindow {
    pub start_time: u64,
    pub end_time: u64,
}

#[cw_serde]
pub struct HaltManager {
    pub prev_block_time: u64,
    pub halt_windows: Vec<HaltWindow>,
}

pub const HALT_MANAGER: Item<HaltManager> = Item::new("hm");

impl HaltManager {
    pub fn contains(&self, block_time: u64) -> bool {
        self.halt_windows
            .iter()
            .any(|w| block_time > w.start_time && block_time < w.end_time)
    }
}
