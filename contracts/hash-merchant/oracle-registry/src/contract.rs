use std::str::FromStr;

use cosmwasm_std::{
    BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, Ibc2Msg, Ibc2PacketAckMsg,
    Ibc2PacketReceiveMsg, Ibc2PacketSendMsg, Ibc2PacketTimeoutMsg, Ibc2Payload, IbcBasicResponse,
    IbcReceiveResponse, MessageInfo, Order, Response, StdAck, Timestamp, Uint128, from_json,
    to_json_binary,
};

use crate::error::ContractError;
use crate::msg::*;
use crate::state::*;

const CONTRACT_NAME: &str = "crates.io:cw-oracle-registry";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ALGO_PRICE_MEDIAN_V1: &str = "price_median_v1";

/// `sha256("UNIVERSAL_ERROR_ACKNOWLEDGEMENT")`. ibc-go and Penumbra write this
/// when the receiving application returns an error. It is not a price.
pub const UNIVERSAL_ERROR_ACK: [u8; 32] = [
    0x47, 0x74, 0xd4, 0xa5, 0x75, 0x99, 0x3f, 0x96, 0x3b, 0x1c, 0x06, 0x57, 0x37, 0x36, 0x61, 0x7a,
    0x45, 0x7a, 0xbe, 0xf8, 0x58, 0x91, 0x78, 0xdb, 0x8d, 0x10, 0xc9, 0x4b, 0x4a, 0xb5, 0x11, 0xab,
];

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let admin = deps.api.addr_validate(&msg.admin)?;
    let note = match msg.note {
        Some(n) => Some(deps.api.addr_validate(&n)?),
        None => None,
    };
    let ibc2_source_client = msg.ibc2_source_client.filter(|s| !s.trim().is_empty());
    CONFIG.save(
        deps.storage,
        &Config {
            admin,
            open_submit: msg.open_submit,
            bounty: msg.bounty,
            note,
            ibc2_source_client: ibc2_source_client.clone(),
            ibc2_dest_port: ibc2_source_client.map(|_| DEX_QUOTE_PORT.to_string()),
        },
    )?;
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "instantiate_oracle_registry"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Submit {
            market_id,
            source,
            source_chain,
            pool_id,
            height,
            mid,
        } => execute_submit(
            deps,
            env,
            info.sender,
            ExecuteSubmitParams {
                market_id,
                source,
                source_chain,
                pool_id,
                height,
                mid,
            },
        ),
        ExecuteMsg::SetRelayer { addr, allowed } => {
            only_admin(deps.as_ref(), &info)?;
            let a = deps.api.addr_validate(&addr)?;
            RELAYERS.save(deps.storage, a.as_str(), &allowed)?;
            Ok(Response::new().add_attribute("action", "set_relayer"))
        }
        ExecuteMsg::SetBounty { bounty } => {
            only_admin(deps.as_ref(), &info)?;
            CONFIG.update(deps.storage, |mut c| -> Result<_, ContractError> {
                c.bounty = bounty;
                Ok(c)
            })?;
            Ok(Response::new().add_attribute("action", "set_bounty"))
        }
        ExecuteMsg::SetNote { note } => {
            only_admin(deps.as_ref(), &info)?;
            let note = deps.api.addr_validate(&note)?;
            CONFIG.update(deps.storage, |mut c| -> Result<_, ContractError> {
                c.note = Some(note);
                Ok(c)
            })?;
            Ok(Response::new().add_attribute("action", "set_note"))
        }
        ExecuteMsg::RegisterIbcV2 { source_client } => {
            execute_register_ibc2(deps, info, source_client)
        }
        ExecuteMsg::RequestQuote {
            market_id,
            pair,
            position_id,
        } => execute_request_quote(deps, env, info, market_id, pair, position_id),
        ExecuteMsg::RequestQuotes { quotes } => execute_request_quotes(deps, env, quotes),
        ExecuteMsg::Callback(cb) => execute_polytone_callback(deps, env, info, cb),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::HashMerchant {
            chain_uid,
            algo,
            root,
            height,
            ..
        } => {
            if algo != ALGO_PRICE_MEDIAN_V1 {
                return Ok(Response::new().add_attribute("skip", "not_price_bound"));
            }
            let admin = CONFIG.load(deps.storage)?.admin;
            execute_submit(
                deps,
                env,
                admin,
                ExecuteSubmitParams {
                    market_id: chain_uid,
                    source: SourceKind::VeQuorum,
                    source_chain: "hashmerchant".into(),
                    pool_id: None,
                    height,
                    mid: root,
                },
            )
        }
    }
}

fn only_admin(deps: Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.admin {
        return Err(ContractError::Unauthorized {});
    }
    Ok(())
}

fn may_submit(deps: Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if cfg.open_submit || info.sender == cfg.admin {
        return Ok(());
    }
    if RELAYERS.may_load(deps.storage, info.sender.as_str())? == Some(true) {
        return Ok(());
    }
    Err(ContractError::Unauthorized {})
}

fn execute_polytone_callback(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cb: PolytoneCallbackMessage,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if let Some(note) = &cfg.note
        && info.sender != *note
    {
        return Err(ContractError::Unauthorized {});
    }
    let hint: CallbackHint = cosmwasm_std::from_json(&cb.initiator_msg)?;
    let bins = match cb.result {
        PolytoneCallback::Query(PolytoneQueryResult::Success(v)) => v,
        _ => return Err(ContractError::InvalidMid {}),
    };
    let raw = bins.first().ok_or(ContractError::InvalidMid {})?;
    let mid = mid_from_query_binary(raw)?;
    record_observation(
        &mut deps,
        env,
        cb.initiator,
        RecordObservationParams {
            market_id: hint.market_id,
            source: SourceKind::Polytone,
            source_chain: hint.source_chain,
            pool_id: hint.pool_id,
            height: hint.height,
            mid,
            position_id: None,
        },
    )
}

#[derive(serde::Deserialize)]
struct TwapBody {
    arithmetic_twap: Option<String>,
    mid: Option<String>,
}

fn mid_from_query_binary(raw: &Binary) -> Result<String, ContractError> {
    if let Ok(v) = cosmwasm_std::from_json::<TwapBody>(raw)
        && let Some(s) = v.arithmetic_twap.or(v.mid)
        && !s.trim().is_empty()
    {
        return Ok(s);
    }
    let as_str = String::from_utf8(raw.to_vec()).map_err(|_| ContractError::InvalidMid {})?;
    let trimmed = as_str.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err(ContractError::InvalidMid {});
    }
    Ok(trimmed.to_string())
}

/// Arguments for [`execute_submit`].
struct ExecuteSubmitParams {
    pub market_id: String,
    pub source: SourceKind,
    pub source_chain: String,
    pub pool_id: Option<u64>,
    pub height: u64,
    pub mid: String,
}

fn execute_submit(
    mut deps: DepsMut,
    env: Env,
    recorder: cosmwasm_std::Addr,
    args: ExecuteSubmitParams,
) -> Result<Response, ContractError> {
    let ExecuteSubmitParams {
        market_id,
        source,
        source_chain,
        pool_id,
        height,
        mid,
    } = args;
    may_submit(
        deps.as_ref(),
        &MessageInfo {
            sender: recorder.clone(),
            funds: vec![],
        },
    )?;
    record_observation(
        &mut deps,
        env,
        recorder,
        RecordObservationParams {
            market_id,
            source,
            source_chain,
            pool_id,
            height,
            mid,
            position_id: None,
        },
    )
}

/// Arguments for [`record_observation`].
struct RecordObservationParams {
    pub market_id: String,
    pub source: SourceKind,
    pub source_chain: String,
    pub pool_id: Option<u64>,
    pub height: u64,
    pub mid: String,
    pub position_id: Option<String>,
}

fn record_observation(
    deps: &mut DepsMut,
    env: Env,
    recorder: cosmwasm_std::Addr,
    args: RecordObservationParams,
) -> Result<Response, ContractError> {
    let RecordObservationParams {
        market_id,
        source,
        source_chain,
        pool_id,
        height,
        mid,
        position_id,
    } = args;
    if market_id.trim().is_empty() {
        return Err(ContractError::EmptyMarket {});
    }
    if height == 0 {
        return Err(ContractError::ZeroHeight {});
    }
    if BY_HEIGHT
        .may_load(deps.storage, (&market_id, height))?
        .is_some()
    {
        return Err(ContractError::AlreadyRecorded {});
    }
    let mid = Decimal::from_str(mid.trim()).map_err(|_| ContractError::InvalidMid {})?;
    let obs = Observation {
        market_id: market_id.clone(),
        source,
        source_chain,
        pool_id,
        height,
        mid,
        recorder: recorder.clone(),
        position_id,
    };
    BY_HEIGHT.save(deps.storage, (&market_id, height), &obs)?;
    let head = HEAD.may_load(deps.storage, &market_id)?.unwrap_or(0);
    if height >= head {
        HEAD.save(deps.storage, &market_id, &height)?;
    }

    let mut res = Response::new()
        .add_attribute("action", "submit")
        .add_attribute("market_id", &market_id)
        .add_attribute("height", height.to_string())
        .add_attribute("first", "true");

    let cfg = CONFIG.load(deps.storage)?;
    if let Some(bounty) = cfg.bounty.clone()
        && !bounty.amount.is_zero()
    {
        let bal = deps
            .querier
            .query_balance(&env.contract.address, &bounty.denom)?;
        if bal.amount >= bounty.amount {
            res = res.add_message(BankMsg::Send {
                to_address: recorder.to_string(),
                amount: vec![Coin {
                    denom: bounty.denom,
                    amount: bounty.amount,
                }],
            });
        }
    }
    Ok(res)
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    match msg {
        QueryMsg::Config {} => {
            let c = CONFIG.load(deps.storage)?;
            Ok(to_json_binary(&ConfigResponse {
                admin: c.admin,
                open_submit: c.open_submit,
                bounty: c.bounty,
                note: c.note,
                ibc2_source_client: c.ibc2_source_client,
                ibc2_dest_port: c.ibc2_dest_port,
            })?)
        }
        QueryMsg::Latest { market_id } => {
            let h = HEAD.load(deps.storage, &market_id)?;
            let o = BY_HEIGHT.load(deps.storage, (&market_id, h))?;
            Ok(to_json_binary(&to_resp(&o))?)
        }
        QueryMsg::History { market_id, limit } => {
            let limit = limit.unwrap_or(32).min(128) as usize;
            let observations: Vec<ObservationResponse> = BY_HEIGHT
                .prefix(&market_id)
                .range(deps.storage, None, None, Order::Descending)
                .take(limit)
                .filter_map(|r| r.ok().map(|(_, o)| to_resp(&o)))
                .collect();
            Ok(to_json_binary(&HistoryResponse { observations })?)
        }
        QueryMsg::Twap {
            market_id,
            from_height,
            to_height,
        } => {
            let mut rows: Vec<Observation> = BY_HEIGHT
                .prefix(&market_id)
                .range(deps.storage, None, None, Order::Ascending)
                .filter_map(|r| r.ok().map(|(_, o)| o))
                .filter(|o| o.height >= from_height && o.height <= to_height)
                .collect();
            if rows.is_empty() {
                return Err(ContractError::EmptyWindow {});
            }
            rows.sort_by_key(|o| o.height);
            let twap = height_weighted_twap(&rows);
            Ok(to_json_binary(&TwapResponse {
                market_id,
                from_height,
                to_height,
                n: rows.len() as u32,
                twap,
                volume_hint: Uint128::zero(),
            })?)
        }
    }
}

fn to_resp(o: &Observation) -> ObservationResponse {
    ObservationResponse {
        market_id: o.market_id.clone(),
        source: o.source.clone(),
        source_chain: o.source_chain.clone(),
        pool_id: o.pool_id,
        height: o.height,
        mid: o.mid,
        recorder: o.recorder.clone(),
        position_id: o.position_id.clone(),
    }
}

/// Piecewise-constant TWAP: each mid holds until the next observation height.
pub fn height_weighted_twap(rows: &[Observation]) -> Decimal {
    if rows.len() == 1 {
        return rows[0].mid;
    }
    let mut acc = Decimal::zero();
    let mut weight = 0u64;
    for w in rows.windows(2) {
        let dt = w[1].height.saturating_sub(w[0].height).max(1);
        acc += w[0].mid * Decimal::from_ratio(dt, 1u64);
        weight += dt;
    }
    let last_dt = 1u64;
    acc += rows[rows.len() - 1].mid * Decimal::from_ratio(last_dt, 1u64);
    weight += last_dt;
    if weight == 0 {
        return rows[rows.len() - 1].mid;
    }
    acc / Decimal::from_ratio(weight, 1u64)
}

fn execute_register_ibc2(
    deps: DepsMut,
    info: MessageInfo,
    source_client: String,
) -> Result<Response, ContractError> {
    only_admin(deps.as_ref(), &info)?;
    let source_client = source_client.trim().to_string();
    if source_client.is_empty() {
        return Err(ContractError::Ibc2Unregistered {});
    }
    CONFIG.update(deps.storage, |mut c| -> Result<_, ContractError> {
        c.ibc2_source_client = Some(source_client);
        c.ibc2_dest_port = Some(DEX_QUOTE_PORT.to_string());
        Ok(c)
    })?;
    Ok(Response::new()
        .add_attribute("action", "register_ibc2")
        .add_attribute("dest_port", DEX_QUOTE_PORT))
}

fn execute_request_quote(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    market_id: String,
    pair: DexPair,
    position_id: String,
) -> Result<Response, ContractError> {
    if market_id.trim().is_empty() {
        return Err(ContractError::EmptyMarket {});
    }
    if pair.asset_1.trim().is_empty() || pair.asset_2.trim().is_empty() {
        return Err(ContractError::EmptyPair {});
    }
    let height = env.block.height;
    if height == 0 {
        return Err(ContractError::ZeroHeight {});
    }
    if BY_HEIGHT
        .may_load(deps.storage, (&market_id, height))?
        .is_some()
    {
        return Err(ContractError::AlreadyRecorded {});
    }
    if PENDING_QUOTE
        .may_load(deps.storage, (&market_id, height))?
        .is_some()
    {
        return Err(ContractError::QuoteInFlight {});
    }
    let cfg = CONFIG.load(deps.storage)?;
    let source_client = cfg
        .ibc2_source_client
        .ok_or(ContractError::Ibc2Unregistered {})?;
    let req = DexQuoteRequest {
        market_id: market_id.clone(),
        height,
        pair,
        position_id,
    };
    PENDING_QUOTE.save(deps.storage, (&market_id, height), &req)?;
    let payload = Ibc2Payload::new(
        ibc2_source_port(&env.contract.address),
        DEX_QUOTE_PORT.to_string(),
        DEX_QUOTE_VERSION.to_string(),
        DEX_QUOTE_ENCODING.to_string(),
        to_json_binary(&req)?,
    );
    let timeout: Timestamp = env.block.time.plus_seconds(600);
    let msg = Ibc2Msg::SendPacket {
        source_client,
        timeout,
        payloads: vec![payload],
    };
    Ok(Response::new()
        .add_message(CosmosMsg::from(msg))
        .add_attribute("action", "request_quote")
        .add_attribute("market_id", market_id)
        .add_attribute("height", height.to_string()))
}

fn execute_request_quotes(
    deps: DepsMut,
    env: Env,
    quotes: Vec<DexQuoteRequestItem>,
) -> Result<Response, ContractError> {
    if quotes.is_empty() || quotes.len() > DEX_QUOTE_BATCH_MAX {
        return Err(ContractError::BatchLimit {});
    }
    let height = env.block.height;
    if height == 0 {
        return Err(ContractError::ZeroHeight {});
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut reqs = Vec::with_capacity(quotes.len());
    for quote in quotes {
        if quote.market_id.trim().is_empty() {
            return Err(ContractError::EmptyMarket {});
        }
        if quote.pair.asset_1.trim().is_empty() || quote.pair.asset_2.trim().is_empty() {
            return Err(ContractError::EmptyPair {});
        }
        if !seen.insert(quote.market_id.clone()) {
            return Err(ContractError::QuoteInFlight {});
        }
        if BY_HEIGHT
            .may_load(deps.storage, (&quote.market_id, height))?
            .is_some()
        {
            return Err(ContractError::AlreadyRecorded {});
        }
        if PENDING_QUOTE
            .may_load(deps.storage, (&quote.market_id, height))?
            .is_some()
        {
            return Err(ContractError::QuoteInFlight {});
        }
        reqs.push(DexQuoteRequest {
            market_id: quote.market_id,
            height,
            pair: quote.pair,
            position_id: quote.position_id,
        });
    }
    let cfg = CONFIG.load(deps.storage)?;
    let source_client = cfg
        .ibc2_source_client
        .ok_or(ContractError::Ibc2Unregistered {})?;
    for req in &reqs {
        PENDING_QUOTE.save(deps.storage, (&req.market_id, height), req)?;
    }
    let batch = DexQuoteBatch { quotes: reqs };
    let payload = Ibc2Payload::new(
        ibc2_source_port(&env.contract.address),
        DEX_QUOTE_PORT.to_string(),
        DEX_QUOTE_VERSION.to_string(),
        DEX_QUOTE_ENCODING.to_string(),
        to_json_binary(&batch)?,
    );
    let timeout: Timestamp = env.block.time.plus_seconds(600);
    let msg = Ibc2Msg::SendPacket {
        source_client,
        timeout,
        payloads: vec![payload],
    };
    Ok(Response::new()
        .add_message(CosmosMsg::from(msg))
        .add_attribute("action", "request_quotes")
        .add_attribute("quotes", batch_len(&batch))
        .add_attribute("height", height.to_string()))
}

fn batch_len(batch: &DexQuoteBatch) -> String {
    batch.quotes.len().to_string()
}

/// wasmd `PortIDForContractV2`: `"wasm2"` plus the address with its bech32 hrp removed.
/// The separator `1` stays, so `terp1qqq` becomes `wasm21qqq`.
pub fn ibc2_source_port(addr: &cosmwasm_std::Addr) -> String {
    let s = addr.as_str();
    match s.split_once('1') {
        Some((_, data)) => format!("wasm21{data}"),
        None => format!("wasm2{s}"),
    }
}

/// Posted price `p/q` as the decimal string `record_observation` already parses.
///
/// `p` and `q` are base units. `5penumbra` is 6 decimals, so a sell of 100
/// base `uterp` at that display price arrives as `p = 500000000`, `q = 100`
/// and this returns `5000000`.
pub fn mid_from_posted_price(p: &str, q: &str) -> Result<String, ContractError> {
    let p = cosmwasm_std::Uint128::from_str(p.trim()).map_err(|_| ContractError::InvalidMid {})?;
    let q = cosmwasm_std::Uint128::from_str(q.trim()).map_err(|_| ContractError::InvalidMid {})?;
    if q.is_zero() {
        return Err(ContractError::InvalidMid {});
    }
    Ok(Decimal::from_ratio(p, q).to_string())
}

/// Decode a dex-quote success ack. Universal error and `{"error":...}` are `NoQuote`.
pub fn parse_dex_quote_ack(raw: &[u8]) -> Result<DexQuoteAck, ContractError> {
    if raw.is_empty() || raw == UNIVERSAL_ERROR_ACK {
        return Err(ContractError::NoQuote {});
    }
    if let Ok(ack) = from_json::<DexQuoteAck>(raw) {
        return Ok(ack);
    }
    if let Ok(std) = from_json::<StdAck>(raw) {
        return match std {
            StdAck::Error(_) => Err(ContractError::NoQuote {}),
            StdAck::Success(bin) => {
                from_json::<DexQuoteAck>(&bin).map_err(|_| ContractError::InvalidMid {})
            }
        };
    }
    Err(ContractError::InvalidMid {})
}

fn skip_ack(reason: &str) -> IbcBasicResponse {
    IbcBasicResponse::new()
        .add_attribute("action", "dex_quote_ack")
        .add_attribute("skip", reason)
}

fn clear_pending(deps: DepsMut, req: &DexQuoteRequest) -> Result<(), ContractError> {
    PENDING_QUOTE.remove(deps.storage, (&req.market_id, req.height));
    Ok(())
}

fn into_ibc(res: Response) -> IbcBasicResponse {
    let mut out = IbcBasicResponse::new();
    for attr in res.attributes {
        out = out.add_attribute(attr.key, attr.value);
    }
    for sub in res.messages {
        out = out.add_message(sub.msg);
    }
    out
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn ibc2_packet_send(
    deps: DepsMut,
    _env: Env,
    msg: Ibc2PacketSendMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let client = cfg
        .ibc2_source_client
        .ok_or(ContractError::Ibc2Unregistered {})?;
    if msg.source_client != client || msg.payload.destination_port != DEX_QUOTE_PORT {
        return Err(ContractError::Ibc2Unregistered {});
    }
    if msg.payload.version != DEX_QUOTE_VERSION || msg.payload.encoding != DEX_QUOTE_ENCODING {
        return Err(ContractError::InvalidMid {});
    }
    for req in outbound_quotes(&msg.payload.value)? {
        let pending = PENDING_QUOTE
            .may_load(deps.storage, (&req.market_id, req.height))?
            .ok_or(ContractError::QuoteInFlight {})?;
        if pending != req {
            return Err(ContractError::QuoteInFlight {});
        }
    }
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc2_send_ok"))
}

fn outbound_quotes(value: &Binary) -> Result<Vec<DexQuoteRequest>, ContractError> {
    if let Ok(one) = from_json::<DexQuoteRequest>(value) {
        return Ok(vec![one]);
    }
    if let Ok(batch) = from_json::<DexQuoteBatch>(value) {
        if batch.quotes.is_empty() || batch.quotes.len() > DEX_QUOTE_BATCH_MAX {
            return Err(ContractError::BatchLimit {});
        }
        return Ok(batch.quotes);
    }
    Err(ContractError::InvalidMid {})
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn ibc2_packet_receive(
    _deps: DepsMut,
    _env: Env,
    _msg: Ibc2PacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    Err(ContractError::Ibc2Unregistered {})
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn ibc2_packet_ack(
    mut deps: DepsMut,
    env: Env,
    msg: Ibc2PacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let Some(client) = cfg.ibc2_source_client else {
        return Ok(skip_ack("unregistered"));
    };
    if msg.source_client != client {
        return Ok(skip_ack("client"));
    }
    if msg.data.destination_port != DEX_QUOTE_PORT
        || msg.data.version != DEX_QUOTE_VERSION
        || msg.data.encoding != DEX_QUOTE_ENCODING
    {
        return Ok(skip_ack("payload"));
    }
    if let Ok(batch) = from_json::<DexQuoteBatch>(&msg.data.value) {
        return ack_batch(deps, env, client, batch, msg.acknowledgement.as_slice());
    }
    let req: DexQuoteRequest = match from_json(&msg.data.value) {
        Ok(req) => req,
        Err(_) => return Ok(skip_ack("payload")),
    };
    let pending = PENDING_QUOTE.may_load(deps.storage, (&req.market_id, req.height))?;
    if pending.as_ref() != Some(&req) {
        return Ok(skip_ack("no_pending"));
    }
    let ack = match parse_dex_quote_ack(msg.acknowledgement.as_slice()) {
        Ok(ack) => ack,
        Err(ContractError::NoQuote {}) | Err(ContractError::InvalidMid {}) => {
            clear_pending(deps.branch(), &req)?;
            return Ok(skip_ack("no_price"));
        }
        Err(e) => return Err(e),
    };
    if ack.pair != req.pair
        || ack.position_id.is_empty()
        || ack.reserves.r1.trim().is_empty()
        || ack.reserves.r2.trim().is_empty()
        || (!req.position_id.is_empty() && ack.position_id != req.position_id)
    {
        clear_pending(deps.branch(), &req)?;
        return Ok(skip_ack("mismatch"));
    }
    let mid = match mid_from_posted_price(&ack.p, &ack.q) {
        Ok(mid) => mid,
        Err(_) => {
            clear_pending(deps.branch(), &req)?;
            return Ok(skip_ack("price"));
        }
    };
    let position_id = ack.position_id.clone();
    let recorder = env.contract.address.clone();
    clear_pending(deps.branch(), &req)?;
    let res = record_observation(
        &mut deps,
        env,
        recorder,
        RecordObservationParams {
            market_id: req.market_id,
            source: SourceKind::DexQuote,
            source_chain: client,
            pool_id: None,
            height: req.height,
            mid,
            position_id: Some(position_id),
        },
    )?;
    Ok(into_ibc(res))
}

fn ack_batch(
    mut deps: DepsMut,
    env: Env,
    client: String,
    batch: DexQuoteBatch,
    raw: &[u8],
) -> Result<IbcBasicResponse, ContractError> {
    let items = match from_json::<DexQuoteBatchAck>(raw) {
        Ok(ack) => ack.quotes,
        Err(_) => {
            for req in &batch.quotes {
                clear_pending(deps.branch(), req)?;
            }
            return Ok(skip_ack("no_price"));
        }
    };
    let mut recorded = 0u32;
    for req in batch.quotes {
        let pending = PENDING_QUOTE.may_load(deps.storage, (&req.market_id, req.height))?;
        if pending.as_ref() != Some(&req) {
            continue;
        }
        let Some(item) = items
            .iter()
            .find(|item| item.market_id == req.market_id && item.height == req.height)
        else {
            clear_pending(deps.branch(), &req)?;
            continue;
        };
        if !item.error.is_empty() {
            clear_pending(deps.branch(), &req)?;
            continue;
        }
        let Some(pair) = item.pair.clone() else {
            clear_pending(deps.branch(), &req)?;
            continue;
        };
        let Some(reserves) = item.reserves.clone() else {
            clear_pending(deps.branch(), &req)?;
            continue;
        };
        if pair != req.pair
            || item.position_id.is_empty()
            || reserves.r1.trim().is_empty()
            || reserves.r2.trim().is_empty()
            || (!req.position_id.is_empty() && item.position_id != req.position_id)
        {
            clear_pending(deps.branch(), &req)?;
            continue;
        }
        let mid = match mid_from_posted_price(&item.p, &item.q) {
            Ok(mid) => mid,
            Err(_) => {
                clear_pending(deps.branch(), &req)?;
                continue;
            }
        };
        let position_id = item.position_id.clone();
        let market_id = req.market_id.clone();
        let height = req.height;
        clear_pending(deps.branch(), &req)?;
        match record_observation(
            &mut deps,
            env.clone(),
            env.contract.address.clone(),
            RecordObservationParams {
                market_id,
                source: SourceKind::DexQuote,
                source_chain: client.clone(),
                pool_id: None,
                height,
                mid,
                position_id: Some(position_id),
            },
        ) {
            Ok(_) => recorded += 1,
            Err(ContractError::AlreadyRecorded {}) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(IbcBasicResponse::new()
        .add_attribute("action", "dex_quote_ack")
        .add_attribute("recorded", recorded.to_string()))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn ibc2_packet_timeout(
    mut deps: DepsMut,
    _env: Env,
    msg: Ibc2PacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    if let Ok(quotes) = outbound_quotes(&msg.payload.value) {
        for req in quotes {
            clear_pending(deps.branch(), &req)?;
        }
    }
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc2_timeout"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(
    _deps: DepsMut,
    _env: Env,
    _msg: MigrateMsg,
    _info: cosmwasm_std::MigrateInfo,
) -> Result<Response, ContractError> {
    Ok(Response::new().add_attribute("action", "migrate"))
}

#[cfg(test)]
mod migrate_tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env};

    #[test]
    fn migrate_ok() {
        let mut deps = mock_dependencies();
        let admin = deps.api.addr_make("admin");
        let res = migrate(
            deps.as_mut(),
            mock_env(),
            MigrateMsg {},
            cosmwasm_std::MigrateInfo {
                sender: admin,
                old_migrate_version: None,
            },
        )
        .unwrap();
        assert_eq!(res.attributes[0].value, "migrate");
    }
}
