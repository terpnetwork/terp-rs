//! Raffle harness. Create pulls the NFT. Draw uses the instantiate seed, not drand.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{
    Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult, Timestamp,
    Uint128, to_json_binary,
};
use cw_storage_plus::{Item, Map};

use crate::pay;

#[cw_serde]
pub struct InstantiateMsg {
    pub fee_bps: u16,
    pub fee_addr: String,
    pub draw_seed: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    Create {
        collection: String,
        token_id: String,
        ticket_price: Uint128,
        duration_secs: u64,
    },
    BuyTicket {
        raffle_id: u64,
    },
    Draw {
        raffle_id: u64,
    },
    Claim {
        raffle_id: u64,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
}

#[cw_serde]
pub struct Config {
    pub fee_bps: u16,
    pub fee_addr: Addr,
    pub draw_seed: String,
}

#[cw_serde]
struct Raffle {
    owner: Addr,
    collection: String,
    token_id: String,
    ticket_price: Uint128,
    end: Timestamp,
    tickets: Vec<Addr>,
    winner: Option<Addr>,
    claimed: bool,
}

const CONFIG: Item<Config> = Item::new("config");
const NEXT_ID: Item<u64> = Item::new("next_id");
const RAFFLES: Map<u64, Raffle> = Map::new("raffles");

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    if msg.fee_bps > 10_000 {
        return Err(StdError::msg("fee_bps exceeds 10000"));
    }
    CONFIG.save(
        deps.storage,
        &Config {
            fee_bps: msg.fee_bps,
            fee_addr: deps.api.addr_validate(&msg.fee_addr)?,
            draw_seed: msg.draw_seed,
        },
    )?;
    NEXT_ID.save(deps.storage, &1u64)?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::Create {
            collection,
            token_id,
            ticket_price,
            duration_secs,
        } => create(
            deps,
            env,
            info,
            collection,
            token_id,
            ticket_price,
            duration_secs,
        ),
        ExecuteMsg::BuyTicket { raffle_id } => buy_ticket(deps, env, info, raffle_id),
        ExecuteMsg::Draw { raffle_id } => draw(deps, env, raffle_id),
        ExecuteMsg::Claim { raffle_id } => claim(deps, info, raffle_id),
    }
}

pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
    }
}

fn create(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: String,
    token_id: String,
    ticket_price: Uint128,
    duration_secs: u64,
) -> StdResult<Response> {
    if ticket_price.is_zero() || duration_secs == 0 {
        return Err(StdError::msg("price and duration must be set"));
    }
    let owner = pay::owner_of(deps.as_ref(), &collection, &token_id)?;
    if owner != info.sender {
        return Err(StdError::msg("not the owner"));
    }
    let id = NEXT_ID.load(deps.storage)?;
    NEXT_ID.save(deps.storage, &(id + 1))?;
    let transfer = pay::transfer(&collection, &token_id, &env.contract.address)?;
    RAFFLES.save(
        deps.storage,
        id,
        &Raffle {
            owner: info.sender,
            collection,
            token_id,
            ticket_price,
            end: env.block.time.plus_seconds(duration_secs),
            tickets: Vec::new(),
            winner: None,
            claimed: false,
        },
    )?;
    Ok(Response::new()
        .add_message(transfer)
        .add_attribute("action", "create")
        .add_attribute("raffle_id", id.to_string()))
}

fn buy_ticket(deps: DepsMut, env: Env, info: MessageInfo, raffle_id: u64) -> StdResult<Response> {
    let mut raffle = RAFFLES.load(deps.storage, raffle_id)?;
    if env.block.time >= raffle.end {
        return Err(StdError::msg("raffle closed"));
    }
    if raffle.winner.is_some() {
        return Err(StdError::msg("already drawn"));
    }
    pay::must_pay_exact(&info, raffle.ticket_price)?;
    raffle.tickets.push(info.sender);
    RAFFLES.save(deps.storage, raffle_id, &raffle)?;
    Ok(Response::new().add_attribute("action", "buy_ticket"))
}

fn draw(deps: DepsMut, env: Env, raffle_id: u64) -> StdResult<Response> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut raffle = RAFFLES.load(deps.storage, raffle_id)?;
    if env.block.time < raffle.end {
        return Err(StdError::msg("raffle still open"));
    }
    if raffle.tickets.is_empty() {
        return Err(StdError::msg("no tickets"));
    }
    if raffle.winner.is_some() {
        return Err(StdError::msg("already drawn"));
    }
    let index = winner_index(&cfg.draw_seed, raffle_id, raffle.tickets.len());
    raffle.winner = Some(raffle.tickets[index].clone());
    RAFFLES.save(deps.storage, raffle_id, &raffle)?;
    Ok(Response::new().add_attribute("action", "draw"))
}

fn claim(deps: DepsMut, info: MessageInfo, raffle_id: u64) -> StdResult<Response> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut raffle = RAFFLES.load(deps.storage, raffle_id)?;
    let winner = raffle
        .winner
        .clone()
        .ok_or_else(|| StdError::msg("not drawn"))?;
    if info.sender != winner {
        return Err(StdError::msg("not the winner"));
    }
    if raffle.claimed {
        return Err(StdError::msg("claimed"));
    }
    raffle.claimed = true;
    let pot = raffle
        .ticket_price
        .checked_mul(Uint128::from(raffle.tickets.len() as u128))?;
    let fee = pot.multiply_ratio(u128::from(cfg.fee_bps), 10_000u128);
    let owner_amt = pot.checked_sub(fee)?;
    let mut msgs = vec![pay::transfer(
        &raffle.collection,
        &raffle.token_id,
        &winner,
    )?];
    if let Some(msg) = pay::send(&raffle.owner, owner_amt) {
        msgs.push(msg);
    }
    if let Some(msg) = pay::send(&cfg.fee_addr, fee) {
        msgs.push(msg);
    }
    RAFFLES.save(deps.storage, raffle_id, &raffle)?;
    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "claim"))
}

fn winner_index(seed: &str, raffle_id: u64, n: usize) -> usize {
    let mut acc = raffle_id;
    for byte in seed.bytes() {
        acc = acc.wrapping_mul(131).wrapping_add(u64::from(byte));
    }
    (acc as usize) % n
}

#[cw_orch::interface(
    InstantiateMsg,
    ExecuteMsg,
    QueryMsg,
    cosmwasm_std::Empty,
    id = "nftfi_bench_raffle"
)]
pub struct BenchRaffle;

impl<Chain: cw_orch::prelude::CwEnv> cw_orch::prelude::Uploadable for BenchRaffle<Chain> {
    fn wasm(_chain: &cw_orch::prelude::ChainInfoOwned) -> cw_orch::prelude::WasmPath {
        panic!("nftfi-bench raffle harness has no wasm");
    }

    fn wrapper() -> Box<dyn cw_orch::prelude::MockContract<cosmwasm_std::Empty>> {
        Box::new(cw_orch::prelude::ContractWrapper::new_with_empty(
            execute,
            instantiate,
            query,
        ))
    }
}
