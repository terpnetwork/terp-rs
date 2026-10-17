//! svg-auction: a minimal escrow market for cw721-svg tokens.
//! Interface is a JSON-compatible subset of `terp-marketplace-utils` (crates/marketplace):
//! list by `send_nft` with `Cw721HookMsg::CreateAuction { denom, reserve_price, is_instant_sale }`;
//! `place_bid`, `settle`, `cancel_auction`; queries `config`, `auction`, `nft_auction`,
//! `auction_by_contract`, `bid_history_by_auction_id`.
//! * instant sale: the first bid paying exactly `reserve_price` buys the token at once.
//! * auction: bids >= reserve and >= last * (1 + min_increment); the previous bidder is refunded
//!   in the same tx; ends at `end_time`; a bid in the last `extension` seconds pushes the end out.
//! * settle (anyone, after end): NFT to winner, proceeds to seller minus protocol fee; no bid -> NFT back.
//! * cancel: seller only and only while there is no bid.
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, Event,
    MessageInfo, Order, Response, StdResult, Timestamp, Uint128, WasmMsg,
};
use cw_storage_plus::{Bound, Item, Map};

#[derive(thiserror::Error, Debug)]
pub enum ContractError {
    #[error(transparent)]
    Std(#[from] cosmwasm_std::StdError),
    #[error("{0}")]
    Msg(String),
    #[error("Unauthorized")]
    Unauthorized,
}
fn err<T>(m: impl Into<String>) -> Result<T, ContractError> { Err(ContractError::Msg(m.into())) }

#[cw_serde]
pub struct InstantiateMsg {
    /// Protocol fee in basis points of the sale price (max 1000 = 10%).
    pub protocol_fee_bps: u16,
    /// Minimum raise over the last bid in basis points.
    pub min_increment_bps: u16,
    pub min_reserve_price: Uint128,
    /// Auction length in seconds.
    pub duration: u64,
    /// Anti-snipe: a bid inside this many seconds before the end extends the end by this much.
    pub extension: u64,
    pub accepted_denom: Vec<String>,
    pub protocol_addr: String,
}
#[cw_serde]
pub struct Cw721ReceiveMsg { pub sender: String, pub token_id: String, pub msg: Binary }
#[cw_serde]
pub enum Cw721HookMsg { CreateAuction { denom: String, reserve_price: Uint128, is_instant_sale: bool } }
#[cw_serde]
pub enum ExecuteMsg {
    ReceiveNft(Cw721ReceiveMsg),
    PlaceBid { auction_id: u64 },
    Settle { auction_id: u64 },
    CancelAuction { auction_id: u64 },
}
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
    #[returns(AuctionResponse)]
    Auction { auction_id: u64 },
    /// Active auction for a token, if any.
    #[returns(Option<AuctionResponse>)]
    NftAuction { nft_contract: String, token_id: String },
    #[returns(Vec<AuctionResponse>)]
    AuctionByContract { nft_contract: String, start_after: Option<u64>, limit: Option<u32> },
    #[returns(Vec<Bid>)]
    BidHistoryByAuctionId { auction_id: u64, limit: Option<u32> },
}
#[cw_serde]
pub struct Config {
    pub protocol_fee_bps: u16, pub min_increment_bps: u16, pub min_reserve_price: Uint128, pub duration: u64,
    pub extension: u64, pub accepted_denom: Vec<String>, pub protocol_addr: Addr,
}
#[cw_serde]
pub enum Status { Open, Sold, Settled, Cancelled }
#[cw_serde]
pub struct AuctionResponse {
    pub auction_id: u64, pub nft_contract: Addr, pub token_id: String, pub seller: Addr, pub denom: String,
    pub reserve_price: Uint128, pub is_instant_sale: bool, pub start_time: Timestamp, pub end_time: Timestamp,
    pub bidder: Option<Addr>, pub amount: Uint128, pub status: Status,
}
#[cw_serde]
pub struct Bid { pub bidder: Addr, pub amount: Uint128, pub time: Timestamp, pub refunded: bool }

const CONFIG: Item<Config> = Item::new("config");
const NEXT_ID: Item<u64> = Item::new("next_id");
const AUCTIONS: Map<u64, AuctionResponse> = Map::new("auctions");
const ACTIVE: Map<(&Addr, &str), u64> = Map::new("active");
const BY_CONTRACT: Map<(&Addr, u64), ()> = Map::new("by_contract");
const BIDS: Map<(u64, u32), Bid> = Map::new("bids");
const NBIDS: Map<u64, u32> = Map::new("nbids");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(deps: DepsMut, _env: Env, _info: MessageInfo, msg: InstantiateMsg) -> Result<Response, ContractError> {
    if msg.protocol_fee_bps > 1000 || msg.min_increment_bps == 0 || msg.duration == 0 || msg.accepted_denom.is_empty() {
        return err("protocol_fee_bps<=1000, min_increment_bps>0, duration>0, accepted_denom non-empty");
    }
    let c = Config { protocol_fee_bps: msg.protocol_fee_bps, min_increment_bps: msg.min_increment_bps, min_reserve_price: msg.min_reserve_price,
        duration: msg.duration, extension: msg.extension, accepted_denom: msg.accepted_denom, protocol_addr: deps.api.addr_validate(&msg.protocol_addr)? };
    CONFIG.save(deps.storage, &c)?;
    NEXT_ID.save(deps.storage, &1)?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

fn send_nft_back(nft: &Addr, to: &Addr, token_id: &str) -> StdResult<CosmosMsg> {
    Ok(WasmMsg::Execute { contract_addr: nft.to_string(), funds: vec![],
        msg: to_json_binary(&serde_json_transfer(to, token_id))? }.into())
}
#[cw_serde]
enum NftExec { TransferNft { recipient: String, token_id: String } }
fn serde_json_transfer(to: &Addr, token_id: &str) -> NftExec { NftExec::TransferNft { recipient: to.to_string(), token_id: token_id.into() } }

fn payout(c: &Config, a: &AuctionResponse, amount: Uint128) -> Vec<CosmosMsg> {
    let fee = amount.multiply_ratio(c.protocol_fee_bps as u128, 10_000u128);
    let mut v: Vec<CosmosMsg> = vec![];
    if !fee.is_zero() { v.push(BankMsg::Send { to_address: c.protocol_addr.to_string(), amount: vec![Coin::new(fee, &a.denom)] }.into()); }
    let rest = amount - fee;
    if !rest.is_zero() { v.push(BankMsg::Send { to_address: a.seller.to_string(), amount: vec![Coin::new(rest, &a.denom)] }.into()); }
    v
}
fn paid(info: &MessageInfo, denom: &str) -> Result<Uint128, ContractError> {
    if info.funds.len() != 1 || info.funds[0].denom != denom { return err(format!("send exactly one coin of {denom}")); }
    Ok(Uint128::try_from(info.funds[0].amount).map_err(|_| ContractError::Msg("amount too large".into()))?)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> Result<Response, ContractError> {
    let c = CONFIG.load(deps.storage)?;
    match msg {
        ExecuteMsg::ReceiveNft(r) => {
            let Cw721HookMsg::CreateAuction { denom, reserve_price, is_instant_sale } = from_json(&r.msg)?;
            if !c.accepted_denom.contains(&denom) { return err(format!("denom {denom} not accepted")); }
            if reserve_price < c.min_reserve_price || reserve_price.is_zero() { return err(format!("reserve_price below minimum {}", c.min_reserve_price)); }
            let nft = info.sender.clone();
            let seller = deps.api.addr_validate(&r.sender)?;
            let id = NEXT_ID.load(deps.storage)?;
            NEXT_ID.save(deps.storage, &(id + 1))?;
            let a = AuctionResponse { auction_id: id, nft_contract: nft.clone(), token_id: r.token_id.clone(), seller: seller.clone(), denom: denom.clone(),
                reserve_price, is_instant_sale, start_time: env.block.time, end_time: env.block.time.plus_seconds(c.duration),
                bidder: None, amount: Uint128::zero(), status: Status::Open };
            AUCTIONS.save(deps.storage, id, &a)?;
            ACTIVE.save(deps.storage, (&nft, &r.token_id), &id)?;
            BY_CONTRACT.save(deps.storage, (&nft, id), &())?;
            Ok(Response::new().add_event(Event::new("auction-create").add_attribute("auction_id", id.to_string())
                .add_attribute("nft_contract", nft).add_attribute("token_id", r.token_id).add_attribute("seller", seller)
                .add_attribute("price", format!("{reserve_price}{denom}")).add_attribute("instant", is_instant_sale.to_string())
                .add_attribute("end_time", a.end_time.seconds().to_string())))
        }
        ExecuteMsg::PlaceBid { auction_id } => {
            let mut a = AUCTIONS.load(deps.storage, auction_id)?;
            if a.status != Status::Open || env.block.time >= a.end_time { return err("auction is not open"); }
            if info.sender == a.seller { return err("seller cannot bid"); }
            let amt = paid(&info, &a.denom)?;
            let n = NBIDS.may_load(deps.storage, auction_id)?.unwrap_or(0);
            let mut msgs: Vec<CosmosMsg> = vec![];
            let mut ev = Event::new("auction-bid").add_attribute("auction_id", auction_id.to_string()).add_attribute("bidder", info.sender.to_string())
                .add_attribute("amount", format!("{amt}{}", a.denom));
            if a.is_instant_sale {
                if amt != a.reserve_price { return err(format!("buy-now price is {}{}", a.reserve_price, a.denom)); }
                msgs.push(send_nft_back(&a.nft_contract, &info.sender, &a.token_id)?);
                msgs.extend(payout(&c, &a, amt));
                a.status = Status::Sold; a.bidder = Some(info.sender.clone()); a.amount = amt;
                ACTIVE.remove(deps.storage, (&a.nft_contract, &a.token_id));
                ev = ev.add_attribute("sold", "true");
            } else {
                let min = if a.bidder.is_some() { a.amount + a.amount.multiply_ratio(c.min_increment_bps as u128, 10_000u128).max(Uint128::one()) } else { a.reserve_price };
                if amt < min { return err(format!("bid must be at least {min}{}", a.denom)); }
                if let Some(prev) = &a.bidder {
                    msgs.push(BankMsg::Send { to_address: prev.to_string(), amount: vec![Coin::new(a.amount, &a.denom)] }.into());
                    BIDS.update(deps.storage, (auction_id, n - 1), |b| -> StdResult<_> { let mut b = b.unwrap(); b.refunded = true; Ok(b) })?;
                }
                if a.end_time.seconds() - env.block.time.seconds() < c.extension { a.end_time = env.block.time.plus_seconds(c.extension); }
                a.bidder = Some(info.sender.clone()); a.amount = amt;
                ev = ev.add_attribute("end_time", a.end_time.seconds().to_string());
            }
            BIDS.save(deps.storage, (auction_id, n), &Bid { bidder: info.sender, amount: amt, time: env.block.time, refunded: false })?;
            NBIDS.save(deps.storage, auction_id, &(n + 1))?;
            AUCTIONS.save(deps.storage, auction_id, &a)?;
            Ok(Response::new().add_messages(msgs).add_event(ev))
        }
        ExecuteMsg::Settle { auction_id } => {
            let mut a = AUCTIONS.load(deps.storage, auction_id)?;
            if a.status != Status::Open { return err("auction is closed"); }
            if env.block.time < a.end_time { return err("auction has not ended"); }
            let mut msgs = vec![];
            match &a.bidder {
                Some(w) => { msgs.push(send_nft_back(&a.nft_contract, w, &a.token_id)?); msgs.extend(payout(&c, &a, a.amount)); }
                None => msgs.push(send_nft_back(&a.nft_contract, &a.seller, &a.token_id)?),
            }
            a.status = Status::Settled;
            ACTIVE.remove(deps.storage, (&a.nft_contract, &a.token_id));
            AUCTIONS.save(deps.storage, auction_id, &a)?;
            Ok(Response::new().add_messages(msgs).add_event(Event::new("auction-settle").add_attribute("auction_id", auction_id.to_string())
                .add_attribute("winner", a.bidder.map(|b| b.to_string()).unwrap_or_else(|| "none".into())).add_attribute("amount", format!("{}{}", a.amount, a.denom))))
        }
        ExecuteMsg::CancelAuction { auction_id } => {
            let mut a = AUCTIONS.load(deps.storage, auction_id)?;
            if info.sender != a.seller { return Err(ContractError::Unauthorized); }
            if a.status != Status::Open || a.bidder.is_some() { return err("cannot cancel: closed or has a bid"); }
            a.status = Status::Cancelled;
            ACTIVE.remove(deps.storage, (&a.nft_contract, &a.token_id));
            AUCTIONS.save(deps.storage, auction_id, &a)?;
            Ok(Response::new().add_message(send_nft_back(&a.nft_contract, &a.seller, &a.token_id)?)
                .add_event(Event::new("auction-cancel").add_attribute("auction_id", auction_id.to_string())))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Auction { auction_id } => to_json_binary(&AUCTIONS.load(deps.storage, auction_id)?),
        QueryMsg::NftAuction { nft_contract, token_id } => {
            let nft = deps.api.addr_validate(&nft_contract)?;
            let a = match ACTIVE.may_load(deps.storage, (&nft, &token_id))? { Some(id) => Some(AUCTIONS.load(deps.storage, id)?), None => None };
            to_json_binary(&a)
        }
        QueryMsg::AuctionByContract { nft_contract, start_after, limit } => {
            let nft = deps.api.addr_validate(&nft_contract)?;
            let ids: Vec<u64> = BY_CONTRACT.prefix(&nft).keys(deps.storage, start_after.map(Bound::exclusive), None, Order::Ascending)
                .take(limit.unwrap_or(30).min(100) as usize).collect::<StdResult<_>>()?;
            to_json_binary(&ids.into_iter().map(|i| AUCTIONS.load(deps.storage, i)).collect::<StdResult<Vec<_>>>()?)
        }
        QueryMsg::BidHistoryByAuctionId { auction_id, limit } => to_json_binary(&BIDS.prefix(auction_id)
            .range(deps.storage, None, None, Order::Descending).take(limit.unwrap_or(30).min(100) as usize)
            .map(|r| r.map(|(_, b)| b)).collect::<StdResult<Vec<_>>>()?),
    }
}

#[cfg(test)]
mod tests;
