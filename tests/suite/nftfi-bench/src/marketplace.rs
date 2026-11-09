//! Listing harness. An ask does not escrow the NFT. Buy and accept transfer
//! it after the owner has approved this contract. Fee  and royalty round down.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{
    Addr, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult,
    Uint128, to_json_binary,
};
use cw_storage_plus::{Item, Map};

use crate::pay::{self, token_key};

#[cw_serde]
pub struct InstantiateMsg {
    pub fee_bps: u16,
    pub royalty_bps: u16,
    pub fee_addr: String,
    pub royalty_addr: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    List {
        collection: String,
        token_id: String,
        price: Uint128,
    },
    Bid {
        collection: String,
        token_id: String,
        price: Uint128,
    },
    Buy {
        collection: String,
        token_id: String,
    },
    Accept {
        collection: String,
        token_id: String,
    },
    Cancel {
        collection: String,
        token_id: String,
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
    pub royalty_bps: u16,
    pub fee_addr: Addr,
    pub royalty_addr: Addr,
}

#[cw_serde]
struct Ask {
    seller: Addr,
    price: Uint128,
}

#[cw_serde]
struct Bid {
    bidder: Addr,
    price: Uint128,
}

const CONFIG: Item<Config> = Item::new("config");
const ASKS: Map<&str, Ask> = Map::new("asks");
const BIDS: Map<&str, Bid> = Map::new("bids");

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    if u32::from(msg.fee_bps) + u32::from(msg.royalty_bps) > 10_000 {
        return Err(StdError::msg("fee_bps + royalty_bps exceeds 10000"));
    }
    CONFIG.save(
        deps.storage,
        &Config {
            fee_bps: msg.fee_bps,
            royalty_bps: msg.royalty_bps,
            fee_addr: deps.api.addr_validate(&msg.fee_addr)?,
            royalty_addr: deps.api.addr_validate(&msg.royalty_addr)?,
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::List {
            collection,
            token_id,
            price,
        } => list(deps, info, collection, token_id, price),
        ExecuteMsg::Bid {
            collection,
            token_id,
            price,
        } => bid(deps, info, collection, token_id, price),
        ExecuteMsg::Buy {
            collection,
            token_id,
        } => buy(deps, info, collection, token_id),
        ExecuteMsg::Accept {
            collection,
            token_id,
        } => accept(deps, info, collection, token_id),
        ExecuteMsg::Cancel {
            collection,
            token_id,
        } => cancel(deps, env, info, collection, token_id),
    }
}

pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
    }
}

fn list(
    deps: DepsMut,
    info: MessageInfo,
    collection: String,
    token_id: String,
    price: Uint128,
) -> StdResult<Response> {
    if price.is_zero() {
        return Err(StdError::msg("price is zero"));
    }
    let owner = pay::owner_of(deps.as_ref(), &collection, &token_id)?;
    if owner != info.sender {
        return Err(StdError::msg("not the owner"));
    }
    let key = token_key(&collection, &token_id);
    if ASKS.may_load(deps.storage, &key)?.is_some() {
        return Err(StdError::msg("ask exists"));
    }
    ASKS.save(
        deps.storage,
        &key,
        &Ask {
            seller: info.sender,
            price,
        },
    )?;
    Ok(Response::new().add_attribute("action", "list"))
}

fn bid(
    deps: DepsMut,
    info: MessageInfo,
    collection: String,
    token_id: String,
    price: Uint128,
) -> StdResult<Response> {
    pay::must_pay_exact(&info, price)?;
    let key = token_key(&collection, &token_id);
    if BIDS.may_load(deps.storage, &key)?.is_some() {
        return Err(StdError::msg("bid exists"));
    }
    BIDS.save(
        deps.storage,
        &key,
        &Bid {
            bidder: info.sender,
            price,
        },
    )?;
    Ok(Response::new().add_attribute("action", "bid"))
}

fn buy(
    deps: DepsMut,
    info: MessageInfo,
    collection: String,
    token_id: String,
) -> StdResult<Response> {
    let cfg = CONFIG.load(deps.storage)?;
    let key = token_key(&collection, &token_id);
    let ask = ASKS.load(deps.storage, &key)?;
    pay::must_pay_exact(&info, ask.price)?;
    let owner = pay::owner_of(deps.as_ref(), &collection, &token_id)?;
    if owner != ask.seller {
        return Err(StdError::msg("seller no longer holds the nft"));
    }
    let mut msgs = vec![pay::transfer(&collection, &token_id, &info.sender)?];
    msgs.extend(pay::payout(
        &ask.seller,
        &cfg.fee_addr,
        &cfg.royalty_addr,
        ask.price,
        cfg.fee_bps,
        cfg.royalty_bps,
    )?);
    msgs.extend(refund_bid(deps.storage, &key)?);
    ASKS.remove(deps.storage, &key);
    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "buy"))
}

fn accept(
    deps: DepsMut,
    info: MessageInfo,
    collection: String,
    token_id: String,
) -> StdResult<Response> {
    let cfg = CONFIG.load(deps.storage)?;
    let key = token_key(&collection, &token_id);
    let ask = ASKS.load(deps.storage, &key)?;
    if ask.seller != info.sender {
        return Err(StdError::msg("not the seller"));
    }
    let bid = BIDS.load(deps.storage, &key)?;
    let mut msgs = vec![pay::transfer(&collection, &token_id, &bid.bidder)?];
    msgs.extend(pay::payout(
        &ask.seller,
        &cfg.fee_addr,
        &cfg.royalty_addr,
        bid.price,
        cfg.fee_bps,
        cfg.royalty_bps,
    )?);
    ASKS.remove(deps.storage, &key);
    BIDS.remove(deps.storage, &key);
    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "accept"))
}

fn cancel(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    collection: String,
    token_id: String,
) -> StdResult<Response> {
    let key = token_key(&collection, &token_id);
    let ask = ASKS.load(deps.storage, &key)?;
    if ask.seller != info.sender {
        return Err(StdError::msg("not the seller"));
    }
    let msgs = refund_bid(deps.storage, &key)?;
    ASKS.remove(deps.storage, &key);
    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "cancel"))
}

fn refund_bid(storage: &mut dyn cosmwasm_std::Storage, key: &str) -> StdResult<Vec<CosmosMsg>> {
    let Some(bid) = BIDS.may_load(storage, key)? else {
        return Ok(Vec::new());
    };
    BIDS.remove(storage, key);
    Ok(pay::send(&bid.bidder, bid.price).into_iter().collect())
}

#[cw_orch::interface(
    InstantiateMsg,
    ExecuteMsg,
    QueryMsg,
    cosmwasm_std::Empty,
    id = "nftfi_bench_marketplace"
)]
pub struct BenchMarketplace;

impl<Chain: cw_orch::prelude::CwEnv> cw_orch::prelude::Uploadable for BenchMarketplace<Chain> {
    fn wasm(_chain: &cw_orch::prelude::ChainInfoOwned) -> cw_orch::prelude::WasmPath {
        panic!("nftfi-bench marketplace harness has no wasm");
    }

    fn wrapper() -> Box<dyn cw_orch::prelude::MockContract<cosmwasm_std::Empty>> {
        Box::new(cw_orch::prelude::ContractWrapper::new_with_empty(
            execute,
            instantiate,
            query,
        ))
    }
}
