use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Coin, Decimal, Timestamp};
use nftfi_randomness::RandomnessSourceMsg;
use nftfi_utils::AssetInfo;

use crate::state::{Config, RaffleInfo, RaffleState};

#[cw_serde]
pub struct InstantiateMsg {
    pub name: String,
    pub owner: Option<String>,
    pub fee_addr: String,
    pub minimum_raffle_duration: Option<u64>,
    pub max_tickets_per_raffle: Option<u32>,
    /// Share of ticket sales sent to `fee_addr`. Must be below 1.
    pub raffle_fee: Decimal,
    /// Empty means creating a raffle is free.
    pub creation_coins: Option<Vec<Coin>>,
    pub randomness: RandomnessSourceMsg,
}

#[cw_serde]
pub struct RaffleOptionsMsg {
    /// Defaults to the current block time.
    pub start: Option<Timestamp>,
    pub duration: u64,
    pub comment: Option<String>,
    pub max_ticket_number: Option<u32>,
    pub max_ticket_per_address: Option<u32>,
    pub min_ticket_number: Option<u32>,
    /// When set, one distinct ticket wins each asset, capped by the number of tickets.
    pub one_winner_per_asset: bool,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    #[cw_orch(payable)]
    CreateRaffle {
        owner: Option<String>,
        assets: Vec<AssetInfo>,
        raffle_options: RaffleOptionsMsg,
        raffle_ticket_price: AssetInfo,
    },
    CancelRaffle {
        raffle_id: u64,
    },
    #[cw_orch(payable)]
    BuyTicket {
        raffle_id: u64,
        ticket_count: u32,
        on_behalf_of: Option<String>,
    },
    /// `Seed` source. `seed` is 32 bytes.
    Draw {
        raffle_id: u64,
        seed: cosmwasm_std::Binary,
    },
    /// `Callback` source. The verifier reply stores the seed.
    RequestDraw {
        raffle_id: u64,
    },
    ClaimRaffle {
        raffle_id: u64,
    },
    UpdateConfig {
        owner: Option<String>,
        fee_addr: Option<String>,
        minimum_raffle_duration: Option<u64>,
        max_tickets_per_raffle: Option<u32>,
        raffle_fee: Option<Decimal>,
        creation_coins: Option<Vec<Coin>>,
        randomness: Option<RandomnessSourceMsg>,
    },
    ToggleLock {
        lock: bool,
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
    #[returns(RaffleResponse)]
    Raffle { raffle_id: u64 },
    #[returns(TicketResponse)]
    Ticket { raffle_id: u64, index: u32 },
    #[returns(TicketCountResponse)]
    TicketCount { owner: String, raffle_id: u64 },
}

#[cw_serde]
pub struct RaffleResponse {
    pub raffle_id: u64,
    pub state: RaffleState,
    pub info: RaffleInfo,
}

#[cw_serde]
pub struct TicketResponse {
    pub owner: String,
}

#[cw_serde]
pub struct TicketCountResponse {
    pub count: u32,
}
