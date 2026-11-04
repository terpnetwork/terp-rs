//! v2 forwarded note claims (V2-SPEC §F).
//!
//! `ClaimNoteForward` pays a proven note over an allowlisted ICS-20 channel to a
//! remote receiver (e.g. a Penumbra address) instead of `info.sender`. The proof
//! destination (public inputs 3/4) is
//! `"hsfwd1" + base64url(sha256(domain, chain, contract, channel, receiver, memo, fallback))`,
//! so every routing field is bound by the existing circuit; changing any of them
//! changes the instances and the proof fails. No circuit change.
//!
//! Refunds: the transfer memo carries `src_callback{address: this contract}`. On a
//! timeout or an error ack, ibc-go first refunds the escrow to this contract, then
//! calls `ibc_source_callback`, which forwards the coin to the bound `fallback`
//! (the frontend MUST use a fresh ephemeral Terp address; not enforced on-chain). If the callback
//! never fires, the coin stays in this contract (never lost, never redirected);
//! the owner may `ResolveForward` after timeout + grace, which can only pay the
//! bound fallback.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    Addr, BankMsg, Binary, Coin, Deps, DepsMut, Env, HexBinary, IbcBasicResponse, IbcMsg,
    IbcSourceCallbackMsg, IbcTimeout, MessageInfo, Order, Reply, Response, StdError, StdResult,
    Storage, SubMsg,
};
use cw_storage_plus::{Bound, Item, Map};
use sha2::{Digest, Sha256};

use crate::epoch::EpochCampaignParams;

pub const FORWARD_REPLY_ID: u64 = 7_301;
pub const FWD_PREFIX: &str = "hsfwd1";
pub const FWD_DOMAIN: &[u8] = b"terp-headstash-forward-v1";
pub const MAX_RECEIVER: usize = 256;
pub const MAX_MEMO: usize = 256;
pub const MAX_CHANNELS: usize = 16;
pub const MIN_TIMEOUT: u64 = 60;
pub const MAX_TIMEOUT: u64 = 7 * 86_400;
/// Owner `ResolveForward` waits this long past the packet timeout.
pub const RESOLVE_GRACE: u64 = 86_400;
pub const MAX_HAL_URI: usize = 512;

#[cw_serde]
pub struct ForwardTarget {
    pub channel_id: String,
    pub receiver: String,
    #[serde(default)]
    pub memo: Option<String>,
    /// Terp address that receives a refund. Freshness is a frontend rule, not enforced here.
    pub fallback: String,
}

#[cw_serde]
pub struct ForwardChannel {
    pub channel_id: String,
    pub timeout_seconds: u64,
    pub enabled: bool,
}

#[cw_serde]
pub struct PendingForward {
    pub channel_id: String,
    pub sequence: u64,
    pub receiver: String,
    pub fallback: Addr,
    pub coin: Coin,
    /// Packet timeout (unix seconds).
    pub timeout: u64,
}

#[cw_serde]
pub struct HalSource {
    /// ipfs://…, ipns://…, ar://…, https://… or a bare host/path. No forced prefix.
    pub uri: String,
    #[serde(default)]
    pub sha256: Option<HexBinary>,
    #[serde(default)]
    pub cid: Option<String>,
}

pub const FORWARD_CHANNELS: Map<&str, ForwardChannel> = Map::new("fwd_channels");
pub const PENDING: Map<(&str, u64), PendingForward> = Map::new("fwd_pending");
pub(crate) const INFLIGHT: Item<PendingForward> = Item::new("fwd_inflight");
pub const HAL_SOURCE: Item<HalSource> = Item::new("hal_source");

fn lp(h: &mut Sha256, b: &[u8]) {
    h.update((b.len() as u32).to_be_bytes());
    h.update(b);
}

fn b64url(b: &[u8]) -> String {
    Binary::from(b.to_vec()).to_base64().replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

/// Proof destination for a forward. 49 bytes, fits the 62-byte destination limbs.
pub fn forward_destination(chain_id: &str, contract: &str, t: &ForwardTarget) -> String {
    let mut h = Sha256::new();
    for f in [FWD_DOMAIN, chain_id.as_bytes(), contract.as_bytes(), t.channel_id.as_bytes(), t.receiver.as_bytes(),
              t.memo.as_deref().unwrap_or("").as_bytes(), t.fallback.as_bytes()] {
        lp(&mut h, f);
    }
    format!("{FWD_PREFIX}{}", b64url(&h.finalize()))
}

fn printable(s: &str) -> bool {
    s.bytes().all(|c| (0x21..=0x7e).contains(&c))
}

fn valid_channel_id(c: &str) -> bool {
    c.strip_prefix("channel-").is_some_and(|n| !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Merged transfer memo: the user's JSON object (or nothing) plus our `src_callback`.
pub fn transfer_memo(user: Option<&str>, contract: &str) -> StdResult<String> {
    let mut obj = match user.map(str::trim).filter(|m| !m.is_empty()) {
        None => serde_json::Map::new(),
        Some(m) => match serde_json::from_str::<serde_json::Value>(m) {
            Ok(serde_json::Value::Object(o)) => o,
            _ => return Err(StdError::msg("memo must be empty or a JSON object")),
        },
    };
    if obj.contains_key("src_callback") || obj.contains_key("dest_callback") {
        return Err(StdError::msg("memo may not set callbacks"));
    }
    obj.insert("src_callback".into(), serde_json::json!({ "address": contract }));
    serde_json::to_string(&serde_json::Value::Object(obj)).map_err(|e| StdError::msg(e.to_string()))
}

/// Stateless checks plus the used-once and allowlist checks. Returns the proof destination,
/// the channel record, the validated fallback and the merged memo.
pub fn check_target(deps: Deps, env: &Env, t: &ForwardTarget) -> StdResult<(String, ForwardChannel, Addr, String)> {
    let ch = FORWARD_CHANNELS
        .may_load(deps.storage, t.channel_id.as_str())?
        .filter(|c| c.enabled)
        .ok_or_else(|| StdError::msg("forward channel not allowed"))?;
    if t.receiver.is_empty() || t.receiver.len() > MAX_RECEIVER || !printable(&t.receiver) {
        return Err(StdError::msg("receiver"));
    }
    if t.memo.as_deref().unwrap_or("").len() > MAX_MEMO {
        return Err(StdError::msg("memo too long"));
    }
    let memo = transfer_memo(t.memo.as_deref(), env.contract.address.as_str())?;
    let fb = deps.api.addr_validate(&t.fallback).map_err(|_| StdError::msg("fallback"))?;
    if fb.as_str() != t.fallback || fb == env.contract.address {
        return Err(StdError::msg("fallback"));
    }
    let dest = forward_destination(&env.block.chain_id, env.contract.address.as_str(), t);
    Ok((dest, ch, fb, memo))
}

#[allow(clippy::too_many_arguments)]
pub fn claim_note_forward(
    mut deps: DepsMut,
    env: &Env,
    campaign: &EpochCampaignParams,
    root: Binary,
    nullifier: Binary,
    amount: u64,
    proof: Binary,
    t: ForwardTarget,
) -> Result<Response, StdError> {
    let (dest, ch, fb, memo) = check_target(deps.as_ref(), env, &t)?;
    // Address freshness (fresh fallback, never-reused receiver) is a frontend rule
    // (V2-SPEC F.5); the contract only enforces one spend per note (nullifier).
    crate::direct::record_claim(deps.branch(), env, campaign, &dest, &root, &nullifier, amount, &proof)?;
    let hcfg = crate::HEADSTASH_CFG.load(deps.storage)?;
    let strategy = crate::epoch::strategy_for(&hcfg.ts, &campaign.denom)?;
    let denom = strategy.denom(&env.contract.address);
    let coin = Coin::new(amount as u128, denom.clone());
    let timeout = env.block.time.plus_seconds(ch.timeout_seconds);
    INFLIGHT.save(deps.storage, &PendingForward {
        channel_id: t.channel_id.clone(),
        sequence: 0,
        receiver: t.receiver.clone(),
        fallback: fb.clone(),
        coin: coin.clone(),
        timeout: timeout.seconds(),
    })?;
    let mut res = Response::new()
        .add_attribute("action", "claim_note_forward")
        .add_attribute("amount", amount.to_string())
        .add_attribute("channel_id", t.channel_id.clone())
        .add_attribute("destination", dest);
    if let crate::tokenfactory::TokenStrategy::NewFungible(_) = strategy {
        res = res.add_message(crate::msg_mint(env.contract.address.to_string(), amount.into(), denom)?);
    }
    let transfer = IbcMsg::Transfer {
        channel_id: t.channel_id,
        to_address: t.receiver,
        amount: coin,
        timeout: IbcTimeout::with_timestamp(timeout),
        memo: Some(memo),
    };
    Ok(res.add_submessage(SubMsg::reply_on_success(transfer, FORWARD_REPLY_ID)))
}

/// MsgTransferResponse { uint64 sequence = 1; }
pub fn transfer_sequence(b: &[u8]) -> StdResult<u64> {
    let mut i = 0;
    while i < b.len() {
        let (key, n) = varint(&b[i..])?;
        i += n;
        match (key >> 3, key & 7) {
            (1, 0) => return varint(&b[i..]).map(|(v, _)| v),
            (_, 0) => i += varint(&b[i..])?.1,
            (_, 2) => {
                let (l, n) = varint(&b[i..])?;
                i += n + l as usize;
            }
            _ => break,
        }
    }
    Err(StdError::msg("MsgTransferResponse: no sequence"))
}

fn varint(b: &[u8]) -> StdResult<(u64, usize)> {
    let mut v = 0u64;
    for (i, x) in b.iter().enumerate().take(10) {
        v |= ((x & 0x7f) as u64) << (7 * i);
        if x & 0x80 == 0 {
            return Ok((v, i + 1));
        }
    }
    Err(StdError::msg("varint"))
}

pub fn reply(deps: DepsMut, msg: Reply) -> Result<Response, StdError> {
    let sub = msg.result.into_result().map_err(StdError::msg)?;
    let data = &sub.msg_responses.first().ok_or_else(|| StdError::msg("MsgTransfer: no msg response"))?.value;
    let seq = transfer_sequence(data.as_slice())?;
    let mut p = INFLIGHT.load(deps.storage)?;
    INFLIGHT.remove(deps.storage);
    p.sequence = seq;
    if PENDING.has(deps.storage, (p.channel_id.as_str(), seq)) {
        return Err(StdError::msg("pending forward exists"));
    }
    PENDING.save(deps.storage, (p.channel_id.as_str(), seq), &p)?;
    Ok(Response::new().add_attribute("forward_sequence", seq.to_string()))
}

fn ack_is_error(data: &[u8]) -> bool {
    // ICS-20: {"result":"AQ=="} on success, {"error":"..."} on failure.
    match serde_json::from_slice::<serde_json::Value>(data) {
        Ok(serde_json::Value::Object(o)) => !o.contains_key("result") || o.contains_key("error"),
        _ => true,
    }
}

fn refund(storage: &mut dyn Storage, key: (&str, u64), why: &str) -> StdResult<IbcBasicResponse> {
    let Some(p) = PENDING.may_load(storage, key)? else {
        return Ok(IbcBasicResponse::new().add_attribute("forward_callback", "unknown_packet"));
    };
    PENDING.remove(storage, key);
    Ok(IbcBasicResponse::new()
        .add_attribute("forward_callback", why)
        .add_attribute("refund_to", p.fallback.to_string())
        .add_attribute("forward_sequence", p.sequence.to_string())
        .add_message(BankMsg::Send { to_address: p.fallback.to_string(), amount: vec![p.coin] }))
}

/// Never errors: a failing callback cannot strand funds any worse than a missing one.
pub fn source_callback(deps: DepsMut, env: Env, msg: IbcSourceCallbackMsg) -> StdResult<IbcBasicResponse> {
    let _ = env;
    match msg {
        IbcSourceCallbackMsg::Acknowledgement(m) => {
            let p = &m.original_packet;
            let key = (p.src.channel_id.as_str(), p.sequence);
            if p.src.port_id != "transfer" {
                return Ok(IbcBasicResponse::new().add_attribute("forward_callback", "ignored_port"));
            }
            if ack_is_error(m.acknowledgement.data.as_slice()) {
                refund(deps.storage, key, "ack_error_refund")
            } else {
                let known = PENDING.has(deps.storage, key);
                PENDING.remove(deps.storage, key);
                Ok(IbcBasicResponse::new()
                    .add_attribute("forward_callback", if known { "ack_success" } else { "unknown_packet" })
                    .add_attribute("forward_sequence", p.sequence.to_string()))
            }
        }
        IbcSourceCallbackMsg::Timeout(m) => {
            let p = &m.packet;
            if p.src.port_id != "transfer" {
                return Ok(IbcBasicResponse::new().add_attribute("forward_callback", "ignored_port"));
            }
            refund(deps.storage, (p.src.channel_id.as_str(), p.sequence), "timeout_refund")
        }
    }
}

/// Owner: refund a forward whose callback never arrived, only to its bound fallback,
/// only after `timeout + RESOLVE_GRACE`. The DAO must first check on the counterparty
/// that the packet was not received (see V2-SPEC §F.7), or the receiver could be paid twice.
pub fn resolve_forward(deps: DepsMut, env: &Env, info: &MessageInfo, channel_id: String, sequence: u64) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| StdError::msg(e.to_string()))?;
    let p = PENDING.may_load(deps.storage, (channel_id.as_str(), sequence))?.ok_or_else(|| StdError::msg("no pending forward"))?;
    if env.block.time.seconds() <= p.timeout + RESOLVE_GRACE {
        return Err(StdError::msg("resolve before timeout + grace"));
    }
    PENDING.remove(deps.storage, (channel_id.as_str(), sequence));
    Ok(Response::new()
        .add_attribute("action", "resolve_forward")
        .add_attribute("refund_to", p.fallback.to_string())
        .add_message(BankMsg::Send { to_address: p.fallback.to_string(), amount: vec![p.coin] }))
}

pub fn set_forward_channel(deps: DepsMut, info: &MessageInfo, channel_id: String, timeout_seconds: u64, enabled: bool) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| StdError::msg(e.to_string()))?;
    if !valid_channel_id(&channel_id) {
        return Err(StdError::msg("channel id"));
    }
    if !(MIN_TIMEOUT..=MAX_TIMEOUT).contains(&timeout_seconds) {
        return Err(StdError::msg("timeout_seconds"));
    }
    if !FORWARD_CHANNELS.has(deps.storage, channel_id.as_str())
        && FORWARD_CHANNELS.keys(deps.storage, None, None, Order::Ascending).count() >= MAX_CHANNELS
    {
        return Err(StdError::msg("too many channels"));
    }
    FORWARD_CHANNELS.save(deps.storage, channel_id.as_str(), &ForwardChannel { channel_id: channel_id.clone(), timeout_seconds, enabled })?;
    Ok(Response::new()
        .add_attribute("action", "set_forward_channel")
        .add_attribute("channel_id", channel_id)
        .add_attribute("enabled", enabled.to_string()))
}

pub fn validate_hal(s: &HalSource) -> StdResult<()> {
    let u = s.uri.as_str();
    if u.is_empty() || u.len() > MAX_HAL_URI {
        return Err(StdError::msg("hal uri length"));
    }
    if !u.bytes().all(|c| (0x21..=0x7e).contains(&c) && !matches!(c, b'"' | b'\'' | b'<' | b'>' | b'\\' | b'`' | b'{' | b'}' | b'|' | b'^')) {
        return Err(StdError::msg("hal uri charset"));
    }
    let lower = u.to_ascii_lowercase();
    if let Some(i) = lower.find(':') {
        let scheme = &lower[..i];
        let is_scheme = !scheme.is_empty() && scheme.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'));
        // host:port (bare form) has digits after ':' and no "//"
        let hostport = lower[i + 1..].split('/').next().is_some_and(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()));
        if is_scheme && !hostport {
            if !matches!(scheme, "ipfs" | "ipns" | "ar" | "https") || !lower[i + 1..].starts_with("//") {
                return Err(StdError::msg("hal uri scheme"));
            }
        }
    }
    for bad in ["javascript:", "data:", "vbscript:", "file:", "blob:"] {
        if lower.contains(bad) {
            return Err(StdError::msg("hal uri scheme"));
        }
    }
    if s.sha256.is_none() && s.cid.is_none() {
        return Err(StdError::msg("hal source needs sha256 or cid"));
    }
    if let Some(h) = &s.sha256 {
        if h.len() != 32 {
            return Err(StdError::msg("hal sha256"));
        }
    }
    if let Some(c) = &s.cid {
        if !(46..=100).contains(&c.len()) || !c.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(StdError::msg("hal cid"));
        }
    }
    Ok(())
}

pub fn set_hal_source(deps: DepsMut, info: &MessageInfo, source: Option<HalSource>) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| StdError::msg(e.to_string()))?;
    match source {
        Some(s) => {
            validate_hal(&s)?;
            HAL_SOURCE.save(deps.storage, &s)?;
            Ok(Response::new().add_attribute("action", "set_hal_source").add_attribute("uri", s.uri))
        }
        None => {
            HAL_SOURCE.remove(deps.storage);
            Ok(Response::new().add_attribute("action", "clear_hal_source"))
        }
    }
}

pub fn query_channels(storage: &dyn Storage) -> StdResult<Vec<ForwardChannel>> {
    FORWARD_CHANNELS.range(storage, None, None, Order::Ascending).take(MAX_CHANNELS).map(|r| r.map(|(_, v)| v)).collect()
}

pub fn query_pending(storage: &dyn Storage, start_after: Option<(String, u64)>, limit: Option<u32>) -> StdResult<Vec<PendingForward>> {
    let lim = limit.unwrap_or(30).min(100) as usize;
    let start = start_after.as_ref().map(|(c, s)| Bound::exclusive((c.as_str(), *s)));
    PENDING.range(storage, start, None, Order::Ascending).take(lim).map(|r| r.map(|(_, v)| v)).collect()
}


#[cfg(test)]
mod frontend_vectors {
    use super::*;
    /// Same vectors as tools/headstash/frontend/test/v2lib.test.mjs (claim page computes the destination locally).
    #[test]
    fn forward_destination_matches_claim_page() {
        let t = |c: &str, r: &str, m: Option<&str>, f: &str| ForwardTarget { channel_id: c.into(), receiver: r.into(), memo: m.map(Into::into), fallback: f.into() };
        assert_eq!(forward_destination("terp-local-1", "terp1contract", &t("channel-7", "penumbra1xyz", None, "terp1fallback")),
                   "hsfwd1l3oJE1L5SZTc5Dc42Mim0GCH3GE4JmKom3DyMt0OmjM");
        assert_eq!(forward_destination("90u-4", "terp1abc", &t("channel-0", "osmo1r", Some("hello"), "terp1fb")),
                   "hsfwd1F_KbPO0Hl2dNEgw3H3QyJAoYhxxqFjsMnHsvN6XmaRY");
        assert_eq!(forward_destination("a", "b", &t("channel-1", "r", Some(""), "f")), forward_destination("a", "b", &t("channel-1", "r", None, "f")));
    }
}
