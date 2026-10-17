//! Open scribing (iteration 4). The seeded theme is permanent. Anyone pays `min_fee` to propose a
//! Marain line for an orb during an epoch (`epoch_blocks`). After the epoch ends, the current owner
//! of the root "shield" orb (`shield_token_id`) selects one proposal during the following epoch;
//! that line becomes the orb's animated message. The epoch's fees for that orb are split between the
//! orb's current owner (`owner_bps`) and its original minter (the rest). Unselected proposals are
//! not refunded (fees are participation) and do not carry over: after the selection window anyone
//! may `settle`, which pays out the same split and leaves the previous message in place.
//! Every proposer is recorded in the participant registry (epoch, orb, proposer) and can claim it once.
use crate::contract::{load_vars, scribe_layout};
use crate::{ContractError, Cw721SvgContract};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, BankMsg, Coin, Deps, DepsMut, Env, Event, MessageInfo, Order, Response, Uint256};
use cw721::traits::Cw721Query;
use cw_storage_plus::{Bound, Item, Map};

#[cw_serde]
pub struct ScribeConfig {
    pub epoch_blocks: u64,
    pub min_fee: Coin,
    /// Share of an epoch pot paid to the orb owner, in basis points; the rest goes to the original minter.
    pub owner_bps: u16,
    /// Spam limit: proposals per orb per epoch.
    pub max_proposals: u32,
    /// Root scribe-orb shield: its current owner selects.
    pub shield_token_id: String,
    pub start_height: u64,
}
impl ScribeConfig {
    pub fn default_config() -> Self {
        ScribeConfig { epoch_blocks: 600, min_fee: Coin::new(1000u128, "uterp"), owner_bps: 7000, max_proposals: 32,
            shield_token_id: "0".into(), start_height: 0 }
    }
    pub fn epoch_at(&self, h: u64) -> u64 { h.saturating_sub(self.start_height) / self.epoch_blocks }
    pub fn epoch_start(&self, e: u64) -> u64 { self.start_height + e * self.epoch_blocks }
}
#[cw_serde]
pub struct Proposal { pub proposer: Addr, pub text: String, pub fee: Coin, pub height: u64 }
#[cw_serde]
pub struct ProposalView { pub index: u32, pub proposer: Addr, pub text: String, pub fee: Coin, pub height: u64 }
#[cw_serde]
pub struct EpochPot { pub amount: Uint256, pub denom: String, pub settled: bool, pub outcome: String }
#[cw_serde]
pub struct MessageRecord { pub text: String, pub proposer: Addr, pub epoch: u64, pub index: u32, pub height: u64, pub selected_by: Addr, pub theme: String }
#[cw_serde]
pub struct Participation { pub proposals: u32, pub claimed: bool }
#[cw_serde]
pub struct ParticipantView { pub epoch: u64, pub token_id: String, pub proposer: Addr, pub proposals: u32, pub claimed: bool }
#[cw_serde]
pub struct EpochInfo { pub height: u64, pub epoch: u64, pub start_height: u64, pub end_height: u64, pub config: ScribeConfig }

pub const CFG: Item<ScribeConfig> = Item::new("svg_scribe_cfg");
pub const PROPOSALS: Map<(&str, u64, u32), Proposal> = Map::new("svg_prop");
pub const PCOUNT: Map<(&str, u64), u32> = Map::new("svg_pcount");
pub const POTS: Map<(&str, u64), EpochPot> = Map::new("svg_pot");
pub const MESSAGES: Map<&str, MessageRecord> = Map::new("svg_message");
pub const SELECTED: Map<(&str, u64), MessageRecord> = Map::new("svg_selected");
pub const PARTICIPANTS: Map<(u64, &str, &Addr), Participation> = Map::new("svg_participant");
pub const ORIGIN: Map<&str, Addr> = Map::new("svg_origin");
pub const SPEED: Map<&str, u16> = Map::new("svg_speed");
pub const MIN_SPEED: u16 = 25;
pub const MAX_SPEED: u16 = 400;

pub fn cfg(deps: Deps) -> ScribeConfig { CFG.may_load(deps.storage).ok().flatten().unwrap_or_else(ScribeConfig::default_config) }

fn owner_of(deps: Deps, token_id: &str) -> Result<Addr, ContractError> {
    Ok(Cw721SvgContract::default().config.nft_info.load(deps.storage, token_id)?.owner)
}
fn text_slot(deps: Deps) -> Result<usize, ContractError> {
    Ok(scribe_layout(&load_vars(deps.storage)?)?.ok_or(ContractError::GenericMsg { m: "this collection has no scribe slot".into() })?.text)
}
fn ensure_admin(deps: Deps, sender: &Addr) -> Result<(), ContractError> {
    match Cw721SvgContract::default().query_creator_ownership(deps.storage)?.owner {
        Some(ow) if &ow == sender => Ok(()),
        _ => Err(ContractError::Unauthorized {}),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn configure(deps: DepsMut, env: Env, info: MessageInfo, epoch_blocks: u64, min_fee: Coin, owner_bps: u16, max_proposals: u32, shield_token_id: String) -> Result<Response, ContractError> {
    ensure_admin(deps.as_ref(), &info.sender)?;
    if epoch_blocks == 0 || owner_bps > 10_000 || max_proposals == 0 || max_proposals > 256 || min_fee.amount.is_zero() {
        return Err(ContractError::GenericMsg { m: "epoch_blocks>0, owner_bps<=10000, 1<=max_proposals<=256, min_fee>0".into() });
    }
    let c = ScribeConfig { epoch_blocks, min_fee, owner_bps, max_proposals, shield_token_id, start_height: env.block.height };
    CFG.save(deps.storage, &c)?;
    Ok(Response::new().add_attribute("action", "configure_scribing").add_event(Event::new("scribe-config")
        .add_attribute("epoch_blocks", c.epoch_blocks.to_string()).add_attribute("min_fee", c.min_fee.to_string())
        .add_attribute("owner_bps", c.owner_bps.to_string()).add_attribute("shield_token_id", c.shield_token_id).add_attribute("start_height", c.start_height.to_string())))
}

pub fn propose(deps: DepsMut, env: Env, info: MessageInfo, token_id: String, text: String) -> Result<Response, ContractError> {
    text_slot(deps.as_ref())?;
    owner_of(deps.as_ref(), &token_id)?;
    cw_svg::marain::validate_text(&text).map_err(|e| ContractError::BadScribeText { reason: e })?;
    let c = cfg(deps.as_ref());
    let paid = cw_utils::must_pay(&info, &c.min_fee.denom).map_err(|e| ContractError::GenericMsg { m: e.to_string() })?;
    if paid < c.min_fee.amount { return Err(ContractError::GenericMsg { m: format!("proposal fee is at least {}", c.min_fee) }); }
    let e = c.epoch_at(env.block.height);
    let n = PCOUNT.may_load(deps.storage, (&token_id, e))?.unwrap_or(0);
    if n >= c.max_proposals { return Err(ContractError::GenericMsg { m: format!("orb {token_id} already has {n} proposals this epoch") }); }
    let fee = Coin::new(paid, c.min_fee.denom.clone());
    PROPOSALS.save(deps.storage, (&token_id, e, n), &Proposal { proposer: info.sender.clone(), text: text.clone(), fee: fee.clone(), height: env.block.height })?;
    PCOUNT.save(deps.storage, (&token_id, e), &(n + 1))?;
    let mut pot = POTS.may_load(deps.storage, (&token_id, e))?.unwrap_or(EpochPot { amount: Uint256::zero(), denom: fee.denom.clone(), settled: false, outcome: "open".into() });
    pot.amount += paid;
    POTS.save(deps.storage, (&token_id, e), &pot)?;
    PARTICIPANTS.update(deps.storage, (e, &token_id, &info.sender), |p| -> Result<_, ContractError> {
        let mut p = p.unwrap_or(Participation { proposals: 0, claimed: false }); p.proposals += 1; Ok(p) })?;
    Ok(Response::new().add_attribute("action", "propose").add_event(Event::new("scribe-proposal")
        .add_attribute("token_id", token_id).add_attribute("epoch", e.to_string()).add_attribute("index", n.to_string())
        .add_attribute("proposer", info.sender.to_string()).add_attribute("fee", fee.to_string()).add_attribute("text", text)))
}

type Payout = (Vec<BankMsg>, Addr, Addr, Uint256, Uint256);
fn payout(deps: Deps, token_id: &str, pot: &EpochPot, bps: u16) -> Result<Payout, ContractError> {
    let owner = owner_of(deps, token_id)?;
    let minter = ORIGIN.may_load(deps.storage, token_id)?.unwrap_or_else(|| owner.clone());
    let to_owner = pot.amount.multiply_ratio(bps as u128, 10_000u128);
    let to_minter = pot.amount - to_owner;
    let mut msgs = vec![];
    if owner == minter {
        if !pot.amount.is_zero() { msgs.push(BankMsg::Send { to_address: owner.to_string(), amount: vec![Coin::new(pot.amount, &pot.denom)] }); }
    } else {
        if !to_owner.is_zero() { msgs.push(BankMsg::Send { to_address: owner.to_string(), amount: vec![Coin::new(to_owner, &pot.denom)] }); }
        if !to_minter.is_zero() { msgs.push(BankMsg::Send { to_address: minter.to_string(), amount: vec![Coin::new(to_minter, &pot.denom)] }); }
    }
    Ok((msgs, owner, minter, to_owner, to_minter))
}

pub fn select(deps: DepsMut, env: Env, info: MessageInfo, token_id: String, epoch: u64, index: u32) -> Result<Response, ContractError> {
    let text_idx = text_slot(deps.as_ref())?;
    let c = cfg(deps.as_ref());
    let now = c.epoch_at(env.block.height);
    if epoch >= now { return Err(ContractError::GenericMsg { m: format!("epoch {epoch} has not ended (current {now})") }); }
    if now > epoch + 1 { return Err(ContractError::GenericMsg { m: format!("selection window for epoch {epoch} closed; use settle") }); }
    if owner_of(deps.as_ref(), &c.shield_token_id)? != info.sender { return Err(ContractError::Unauthorized {}); }
    let mut pot = POTS.load(deps.storage, (&token_id, epoch))?;
    if pot.settled { return Err(ContractError::GenericMsg { m: "epoch already settled".into() }); }
    let p = PROPOSALS.load(deps.storage, (&token_id, epoch, index))?;
    let theme = Cw721SvgContract::default().config.nft_info.load(deps.storage, &token_id)?.extension.params[text_idx].value.clone();
    let rec = MessageRecord { text: p.text.clone(), proposer: p.proposer.clone(), epoch, index, height: env.block.height, selected_by: info.sender.clone(), theme: theme.clone() };
    MESSAGES.save(deps.storage, &token_id, &rec)?;
    SELECTED.save(deps.storage, (&token_id, epoch), &rec)?;
    let (msgs, owner, minter, a, b) = payout(deps.as_ref(), &token_id, &pot, c.owner_bps)?;
    pot.settled = true; pot.outcome = format!("selected:{index}");
    POTS.save(deps.storage, (&token_id, epoch), &pot)?;
    Ok(Response::new().add_attribute("action", "select").add_messages(msgs).add_event(Event::new("scribe")
        .add_attribute("token_id", &token_id).add_attribute("epoch", epoch.to_string()).add_attribute("index", index.to_string())
        .add_attribute("scribe", p.proposer.to_string()).add_attribute("selected_by", info.sender.to_string())
        .add_attribute("theme", theme).add_attribute("text", p.text)
        .add_attribute("paid_owner", format!("{a}{} -> {owner}", pot.denom)).add_attribute("paid_minter", format!("{b}{} -> {minter}", pot.denom))))
}

pub fn settle(deps: DepsMut, env: Env, token_id: String, epoch: u64) -> Result<Response, ContractError> {
    let c = cfg(deps.as_ref());
    let now = c.epoch_at(env.block.height);
    if now <= epoch + 1 { return Err(ContractError::GenericMsg { m: format!("selection window for epoch {epoch} is still open") }); }
    let mut pot = POTS.load(deps.storage, (&token_id, epoch))?;
    if pot.settled { return Err(ContractError::GenericMsg { m: "epoch already settled".into() }); }
    let (msgs, owner, minter, a, b) = payout(deps.as_ref(), &token_id, &pot, c.owner_bps)?;
    pot.settled = true; pot.outcome = "unselected".into();
    POTS.save(deps.storage, (&token_id, epoch), &pot)?;
    Ok(Response::new().add_attribute("action", "settle").add_messages(msgs).add_event(Event::new("scribe-settle")
        .add_attribute("token_id", token_id).add_attribute("epoch", epoch.to_string()).add_attribute("outcome", "unselected")
        .add_attribute("paid_owner", format!("{a}{} -> {owner}", pot.denom)).add_attribute("paid_minter", format!("{b}{} -> {minter}", pot.denom))))
}

pub fn claim(deps: DepsMut, env: Env, info: MessageInfo, token_id: String, epoch: u64) -> Result<Response, ContractError> {
    let c = cfg(deps.as_ref());
    if epoch >= c.epoch_at(env.block.height) { return Err(ContractError::GenericMsg { m: "claim after the epoch ends".into() }); }
    let mut p = PARTICIPANTS.load(deps.storage, (epoch, &token_id, &info.sender))?;
    if p.claimed { return Err(ContractError::GenericMsg { m: "already claimed".into() }); }
    p.claimed = true;
    PARTICIPANTS.save(deps.storage, (epoch, &token_id, &info.sender), &p)?;
    Ok(Response::new().add_attribute("action", "claim_participation").add_event(Event::new("participant")
        .add_attribute("epoch", epoch.to_string()).add_attribute("token_id", &token_id).add_attribute("proposer", info.sender.to_string())
        .add_attribute("participant_id", format!("e{epoch}-orb{token_id}")).add_attribute("proposals", p.proposals.to_string())))
}

pub fn set_speed(deps: DepsMut, info: MessageInfo, token_id: String, speed_pct: Option<u16>) -> Result<Response, ContractError> {
    if owner_of(deps.as_ref(), &token_id)? != info.sender { return Err(ContractError::Unauthorized {}); }
    match speed_pct {
        Some(s) if (MIN_SPEED..=MAX_SPEED).contains(&s) => SPEED.save(deps.storage, &token_id, &s)?,
        Some(_) => return Err(ContractError::GenericMsg { m: format!("speed_pct must be {MIN_SPEED}..={MAX_SPEED}") }),
        None => SPEED.remove(deps.storage, &token_id),
    }
    Ok(Response::new().add_attribute("action", "set_speed").add_event(Event::new("scribe-speed")
        .add_attribute("token_id", token_id).add_attribute("speed_pct", speed_pct.map(|s| s.to_string()).unwrap_or_else(|| "default".into()))))
}

pub fn q_epoch(deps: Deps, env: &Env) -> EpochInfo {
    let c = cfg(deps); let e = c.epoch_at(env.block.height);
    EpochInfo { height: env.block.height, epoch: e, start_height: c.epoch_start(e), end_height: c.epoch_start(e + 1), config: c }
}
pub fn q_proposals(deps: Deps, token_id: String, epoch: u64) -> Result<Vec<ProposalView>, ContractError> {
    Ok(PROPOSALS.prefix((&token_id, epoch)).range(deps.storage, None, None, Order::Ascending)
        .map(|r| r.map(|(index, p)| ProposalView { index, proposer: p.proposer, text: p.text, fee: p.fee, height: p.height }))
        .collect::<Result<_, _>>()?)
}
pub fn q_history(deps: Deps, token_id: String) -> Result<Vec<MessageRecord>, ContractError> {
    Ok(SELECTED.prefix(&token_id).range(deps.storage, None, None, Order::Ascending).map(|r| r.map(|(_, m)| m)).collect::<Result<_, _>>()?)
}
pub fn q_pot(deps: Deps, token_id: String, epoch: u64) -> Result<Option<EpochPot>, ContractError> {
    Ok(POTS.may_load(deps.storage, (&token_id, epoch))?)
}
pub fn q_participants(deps: Deps, epoch: u64, limit: Option<u32>) -> Result<Vec<ParticipantView>, ContractError> {
    let limit = limit.unwrap_or(50).min(200) as usize;
    Ok(PARTICIPANTS.sub_prefix(epoch).range(deps.storage, None::<Bound<(&str, &Addr)>>, None, Order::Ascending).take(limit)
        .map(|r| r.map(|((t, a), p)| ParticipantView { epoch, token_id: t, proposer: a, proposals: p.proposals, claimed: p.claimed }))
        .collect::<Result<_, _>>()?)
}

/// Scale a derived `dur` ("s.cc") by a speed percent (100 = as derived; 200 = twice as fast).
pub fn scale_dur(dur: &str, speed_pct: u16) -> String {
    let (s, c) = dur.split_once('.').unwrap_or((dur, "0"));
    let centis = s.parse::<u64>().unwrap_or(0) * 100 + c.parse::<u64>().unwrap_or(0);
    let scaled = (centis * 100 / speed_pct.max(1) as u64).max(1);
    format!("{}.{:02}", scaled / 100, scaled % 100)
}
