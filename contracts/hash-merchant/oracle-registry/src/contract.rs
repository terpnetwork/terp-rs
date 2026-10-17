use std::str::FromStr;

use cosmwasm_std::{
    entry_point, to_json_binary, BankMsg, Binary, Coin, Decimal, Deps, DepsMut, Env, MessageInfo,
    Order, Response, Uint128,
};

use crate::error::ContractError;
use crate::msg::*;
use crate::state::*;

const CONTRACT_NAME: &str = "crates.io:cw-oracle-registry";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ALGO_PRICE_MEDIAN_V1: &str = "price_median_v1";

#[entry_point]
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
    CONFIG.save(
        deps.storage,
        &Config {
            admin,
            open_submit: msg.open_submit,
            bounty: msg.bounty,
            note,
        },
    )?;
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "instantiate_oracle_registry"))
}

#[entry_point]
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
            market_id,
            source,
            source_chain,
            pool_id,
            height,
            mid,
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
        ExecuteMsg::Callback(cb) => execute_polytone_callback(deps, env, info, cb),
    }
}

#[entry_point]
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
                chain_uid,
                SourceKind::VeQuorum,
                "hashmerchant".into(),
                None,
                height,
                root,
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
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cb: PolytoneCallbackMessage,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if let Some(note) = &cfg.note {
        if info.sender != *note {
            return Err(ContractError::Unauthorized {});
        }
    }
    let hint: CallbackHint = cosmwasm_std::from_json(&cb.initiator_msg)?;
    let bins = match cb.result {
        PolytoneCallback::Query(PolytoneQueryResult::Success(v)) => v,
        _ => return Err(ContractError::InvalidMid {}),
    };
    let raw = bins.first().ok_or(ContractError::InvalidMid {})?;
    let mid = mid_from_query_binary(raw)?;
    record_observation(
        deps,
        env,
        cb.initiator,
        hint.market_id,
        SourceKind::Polytone,
        hint.source_chain,
        hint.pool_id,
        hint.height,
        mid,
    )
}

#[derive(serde::Deserialize)]
struct TwapBody {
    arithmetic_twap: Option<String>,
    mid: Option<String>,
}

fn mid_from_query_binary(raw: &Binary) -> Result<String, ContractError> {
    if let Ok(v) = cosmwasm_std::from_json::<TwapBody>(raw) {
        if let Some(s) = v.arithmetic_twap.or(v.mid) {
            if !s.trim().is_empty() {
                return Ok(s);
            }
        }
    }
    let as_str = String::from_utf8(raw.to_vec()).map_err(|_| ContractError::InvalidMid {})?;
    let trimmed = as_str.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err(ContractError::InvalidMid {});
    }
    Ok(trimmed.to_string())
}

fn execute_submit(
    deps: DepsMut,
    env: Env,
    recorder: cosmwasm_std::Addr,
    market_id: String,
    source: SourceKind,
    source_chain: String,
    pool_id: Option<u64>,
    height: u64,
    mid: String,
) -> Result<Response, ContractError> {
    may_submit(
        deps.as_ref(),
        &MessageInfo {
            sender: recorder.clone(),
            funds: vec![],
        },
    )?;
    record_observation(
        deps,
        env,
        recorder,
        market_id,
        source,
        source_chain,
        pool_id,
        height,
        mid,
    )
}

fn record_observation(
    deps: DepsMut,
    env: Env,
    recorder: cosmwasm_std::Addr,
    market_id: String,
    source: SourceKind,
    source_chain: String,
    pool_id: Option<u64>,
    height: u64,
    mid: String,
) -> Result<Response, ContractError> {
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
    if let Some(bounty) = cfg.bounty.clone() {
        if !bounty.amount.is_zero() {
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
    }
    Ok(res)
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    match msg {
        QueryMsg::Config {} => {
            let c = CONFIG.load(deps.storage)?;
            Ok(to_json_binary(&ConfigResponse {
                admin: c.admin,
                open_submit: c.open_submit,
                bounty: c.bounty,
                note: c.note,
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
