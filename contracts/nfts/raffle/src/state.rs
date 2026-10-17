use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Coin, Decimal, Env, StdResult, Storage, Timestamp, Uint256};
use cw_storage_plus::{Item, Map};
use nftfi_randomness::RandomnessSource;
use nftfi_utils::{AssetInfo, Locks};

use crate::error::ContractError;

pub const CONFIG: Item<Config> = Item::new("config");
pub const RAFFLES: Map<u64, RaffleInfo> = Map::new("raffle");
/// Ticket index -> buyer. Indexes are `0..ticket_count`.
pub const TICKETS: Map<(u64, u32), Addr> = Map::new("tickets");
pub const USER_TICKETS: Map<(Addr, u64), u32> = Map::new("user_tickets");

#[cw_serde]
pub struct Config {
    pub name: String,
    pub owner: Addr,
    pub fee_addr: Addr,
    pub last_raffle_id: Option<u64>,
    pub minimum_raffle_duration: u64,
    pub max_tickets_per_raffle: Option<u32>,
    pub raffle_fee: Decimal,
    pub locks: Locks,
    pub creation_coins: Vec<Coin>,
    pub randomness: RandomnessSource,
}

#[cw_serde]
pub struct RaffleOptions {
    pub start: Timestamp,
    pub duration: u64,
    pub comment: Option<String>,
    pub max_ticket_number: Option<u32>,
    pub max_ticket_per_address: Option<u32>,
    pub min_ticket_number: Option<u32>,
    pub one_winner_per_asset: bool,
}

#[cw_serde]
pub struct DrawnRandomness {
    /// 32-byte seed.
    pub seed: cosmwasm_std::Binary,
    pub round: u64,
    pub provider: Addr,
}

#[cw_serde]
pub struct RaffleInfo {
    pub owner: Addr,
    pub assets: Vec<AssetInfo>,
    pub ticket_price: Coin,
    pub ticket_count: u32,
    pub options: RaffleOptions,
    pub randomness: Option<DrawnRandomness>,
    pub winners: Vec<Addr>,
    pub is_cancelled: bool,
}

#[cw_serde]
pub enum RaffleState {
    Created,
    Started,
    /// Sale ended, a draw is required before claim.
    Closed,
    /// Ready to pay winners, or already short of the minimum and ready to settle.
    Finished,
    Claimed,
    Cancelled,
}

pub fn load_raffle(storage: &dyn Storage, raffle_id: u64) -> Result<RaffleInfo, ContractError> {
    RAFFLES
        .may_load(storage, raffle_id)?
        .ok_or(ContractError::NotFound)
}

pub fn raffle_state(env: &Env, info: &RaffleInfo) -> RaffleState {
    if info.is_cancelled {
        return RaffleState::Cancelled;
    }
    let end = info.options.start.plus_seconds(info.options.duration);
    if env.block.time < info.options.start {
        return RaffleState::Created;
    }
    if env.block.time < end {
        return RaffleState::Started;
    }
    let minimum = info.options.min_ticket_number.unwrap_or(0);
    let needs_draw = info.ticket_count > 0 && info.ticket_count >= minimum;
    if needs_draw && info.randomness.is_none() {
        return RaffleState::Closed;
    }
    if info.winners.is_empty() {
        RaffleState::Finished
    } else {
        RaffleState::Claimed
    }
}

pub fn seed_bytes(drawn: &DrawnRandomness) -> StdResult<[u8; 32]> {
    let raw = drawn.seed.as_slice();
    if raw.len() != 32 {
        return Err(cosmwasm_std::StdError::msg("stored seed is not 32 bytes"));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(raw);
    Ok(seed)
}

/// Native coin amount still held for this raffle's prizes (not ticket sales).
pub fn prize_coins(info: &RaffleInfo) -> Vec<Coin> {
    info.assets.iter().filter_map(|a| a.as_coin().cloned()).collect()
}

pub fn ticket_pot(info: &RaffleInfo) -> Result<Uint256, ContractError> {
    info.ticket_price
        .amount
        .checked_mul(Uint256::from(info.ticket_count as u128))
        .map_err(|e| ContractError::Std(cosmwasm_std::StdError::msg(e.to_string())))
}
