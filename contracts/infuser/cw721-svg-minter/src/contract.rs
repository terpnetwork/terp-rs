#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo, Order, Response, StdResult, WasmMsg,
    instantiate2_address, to_json_binary,
};
use cw_storage_plus::Bound;
use cw2::set_contract_version;
use sha2::{Digest, Sha256};

use crate::error::ContractError;
use crate::msg::MigrateMsg;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{
    COLLECTION_NONCE, DEFAULT_SERIES_WINDOW_SECS, DREGG_CHANNEL, DREGG_IBC_ONLY, DREGG_PORT,
    ORACLE_PRICING, RUNTIME_MINTERS, SERIES, SERIES_RECENT, SVG_CODE_ID, SeriesConfig, SvgCollection,
    svg_collections,
};

pub(crate) const CONTRACT_NAME: &str = "cw-svg-minter";
pub(crate) const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_LIMIT: u32 = 10;
pub const MAX_LIMIT: u32 = 50;

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    cw_ownable::initialize_owner(deps.storage, deps.api, msg.owner.as_deref())?;
    SVG_CODE_ID.save(deps.storage, &msg.svg_code_id)?;
    COLLECTION_NONCE.save(deps.storage, &0u64)?;
    DREGG_IBC_ONLY.save(deps.storage, &msg.dregg_ibc_only)?;
    for a in msg.runtime_minters {
        let addr = deps.api.addr_validate(&a)?;
        RUNTIME_MINTERS.save(deps.storage, &addr, &())?;
    }
    if let Some(p) = msg.oracle_pricing {
        p.validate(deps.api)
            .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
        ORACLE_PRICING.save(deps.storage, &p)?;
    }
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("creator", info.sender)
        .add_attribute("dregg_ibc_only", msg.dregg_ibc_only.to_string()))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    if let Some(ch) = msg.dregg_channel {
        DREGG_CHANNEL.save(deps.storage, &ch)?;
    }
    if let Some(p) = msg.dregg_port {
        DREGG_PORT.save(deps.storage, &p)?;
    }
    if let Some(only) = msg.dregg_ibc_only {
        DREGG_IBC_ONLY.save(deps.storage, &only)?;
    }
    if let Some(addrs) = msg.runtime_minters {
        for a in addrs {
            let addr = deps.api.addr_validate(&a)?;
            RUNTIME_MINTERS.save(deps.storage, &addr, &())?;
        }
    }
    Ok(Response::new()
        .add_attribute("action", "migrate")
        .add_attribute("to_version", CONTRACT_VERSION))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateSvgCollection {
            instantiate_msg,
            label,
        } => execute_create_svg_collection(deps, env, info, instantiate_msg, label),
        ExecuteMsg::CreateSeriesCollection {
            series,
            instantiate_msg,
            label,
        } => create_collection(deps, env, info, instantiate_msg, label, Some(series)),
        ExecuteMsg::SetSeries { series, config } => {
            cw_ownable::assert_owner(deps.storage, &info.sender)?;
            match &config {
                Some(c) => {
                    for o in &c.operators {
                        deps.api.addr_validate(o)?;
                    }
                    SERIES.save(deps.storage, &series, c)?
                }
                None => SERIES.remove(deps.storage, &series),
            }
            Ok(Response::new()
                .add_attribute("action", "set_series")
                .add_attribute("series", series)
                .add_attribute("removed", config.is_none().to_string()))
        }
        ExecuteMsg::UpdateOwnership(action) => {
            let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
            Ok(Response::new().add_attributes(ownership.into_attributes()))
        }
        ExecuteMsg::UpdateCodeId { code_id } => execute_update_code_id(deps, info, code_id),
        ExecuteMsg::RecvDreggPacket { packet } => {
            if !crate::state::RUNTIME_MINTERS.has(deps.storage, &info.sender) {
                return Err(ContractError::Unauthorized {});
            }
            crate::ibc::accept_execute(deps, packet.clone())?;
            let mut res = Response::new().add_attribute("action", "recv_dregg_packet");
            if !packet.collection.is_empty() {
                let mint = br#"{"update_extension":{"msg":{"mint":{"alloc":0,"amnt":1,"proof_hashes":[],"ext":null}}}}"#;
                res = res.add_message(WasmMsg::Execute {
                    contract_addr: packet.collection,
                    msg: mint.to_vec().into(),
                    funds: vec![],
                });
            }
            Ok(res)
        }
        ExecuteMsg::BindDreggChannel { channel, port } => {
            if !crate::state::RUNTIME_MINTERS.has(deps.storage, &info.sender) {
                return Err(ContractError::Unauthorized {});
            }
            crate::state::DREGG_CHANNEL.save(deps.storage, &channel)?;
            crate::state::DREGG_PORT.save(deps.storage, &port)?;
            Ok(Response::new().add_attribute("action", "bind_dregg_channel"))
        }
        ExecuteMsg::UpdateOraclePricing { pricing } => {
            cw_ownable::assert_owner(deps.storage, &info.sender)?;
            match &pricing {
                Some(p) => {
                    p.validate(deps.api)
                        .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
                    ORACLE_PRICING.save(deps.storage, p)?
                }
                None => ORACLE_PRICING.remove(deps.storage),
            }
            Ok(Response::new()
                .add_attribute("action", "update_oracle_pricing")
                .add_attribute("removed", pricing.is_none().to_string()))
        }
    }
}

pub fn execute_create_svg_collection(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    instantiate_msg: cw721_svg::InstantiateMsg,
    label: String,
) -> Result<Response, ContractError> {
    create_collection(deps, env, info, instantiate_msg, label, None)
}

/// Next unix second at which `cfg` allows a creation, given `recent` (oldest first).
pub fn series_next_allowed(cfg: &SeriesConfig, recent: &[u64]) -> Option<u64> {
    let mut next: Option<u64> = None;
    if let (Some(min), Some(last)) = (cfg.min_interval_secs, recent.last()) {
        next = Some(last + min);
    }
    if let Some(max) = cfg.max_per_window {
        let w = cfg.window_secs.unwrap_or(DEFAULT_SERIES_WINDOW_SECS);
        let n = recent.len();
        if max == 0 {
            return Some(u64::MAX);
        }
        if n >= max as usize {
            // the oldest of the last `max` entries must leave the window
            let t = recent[n - max as usize] + w;
            next = Some(next.map_or(t, |x| x.max(t)));
        }
    }
    next
}

/// Enforce the series cadence at `now` and return the pruned + appended history.
fn check_series(
    deps: &DepsMut,
    sender: &cosmwasm_std::Addr,
    series: &str,
    now: u64,
    msg: &cw721_svg::InstantiateMsg,
) -> Result<Vec<u64>, ContractError> {
    let cfg = SERIES
        .may_load(deps.storage, series)?
        .ok_or_else(|| ContractError::UnknownSeries {
            series: series.to_string(),
        })?;
    let is_owner = cw_ownable::is_owner(deps.storage, sender)?;
    if !cfg.operators.is_empty() && !is_owner && !cfg.operators.iter().any(|o| o == sender.as_str())
    {
        return Err(ContractError::Unauthorized {});
    }
    if let Some(max) = cfg.max_mint_window_secs {
        match msg.collection_info_extension.mint_end_time {
            Some(off) if off.seconds() <= max => {}
            _ => {
                return Err(ContractError::SeriesMintWindow {
                    series: series.to_string(),
                    max,
                });
            }
        }
    }
    let keep = cfg
        .window_secs
        .unwrap_or(DEFAULT_SERIES_WINDOW_SECS)
        .max(cfg.min_interval_secs.unwrap_or(0));
    let mut recent: Vec<u64> = SERIES_RECENT
        .may_load(deps.storage, series)?
        .unwrap_or_default()
        .into_iter()
        .filter(|t| now < t.saturating_add(keep))
        .collect();
    // window count only looks at entries inside the window
    let w = cfg.window_secs.unwrap_or(DEFAULT_SERIES_WINDOW_SECS);
    let in_window: Vec<u64> = recent.iter().copied().filter(|t| now < t + w).collect();
    if let (Some(min), Some(last)) = (cfg.min_interval_secs, recent.last()) {
        if now < last + min {
            return Err(ContractError::SeriesTooSoon {
                series: series.to_string(),
                next_at: last + min,
            });
        }
    }
    if let Some(max) = cfg.max_per_window {
        if in_window.len() >= max as usize {
            let next_at = if max == 0 {
                u64::MAX
            } else {
                in_window[in_window.len() - max as usize] + w
            };
            return Err(ContractError::SeriesWindowFull {
                series: series.to_string(),
                max,
                window: w,
                next_at,
            });
        }
    }
    recent.push(now);
    Ok(recent)
}

fn create_collection(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    instantiate_msg: cw721_svg::InstantiateMsg,
    label: String,
    series: Option<String>,
) -> Result<Response, ContractError> {
    let mut is_series_operator = false;
    if let Some(s) = &series {
        let recent = check_series(&deps, &info.sender, s, env.block.time.seconds(), &instantiate_msg)?;
        SERIES_RECENT.save(deps.storage, s, &recent)?;
        is_series_operator = true;
    }
    // If owner is set, only owner (or an authorized series operator) may create collections.
    let ownership = cw_ownable::get_ownership(deps.storage)?;
    if !is_series_operator
        && ownership
            .owner
            .as_ref()
            .is_some_and(|owner| *owner != info.sender)
    {
        return Err(ContractError::Unauthorized {});
    }
    if is_series_operator && ownership.owner.as_ref().is_some_and(|o| *o != info.sender) {
        // owned factory: an open (operator-less) series still needs the owner
        let cfg = SERIES.load(deps.storage, series.as_deref().unwrap_or_default())?;
        if cfg.operators.is_empty() {
            return Err(ContractError::Unauthorized {});
        }
    }

    // Fail early (before instantiate2) on bad templates, for both input paths.
    let (template, _) = cw_svg_pack::unpack(&instantiate_msg.collection_info_extension.packed)
        .map_err(|e| ContractError::Std(cosmwasm_std::StdError::msg(e.to_string())))?;
    cw_svg::validate_animated_paint(&template)?;

    let code_id = SVG_CODE_ID.load(deps.storage)?;

    // Fetch code checksum required for instantiate2 address derivation.
    let code_info = deps.querier.query_wasm_code_info(code_id)?;

    // The "creator" in the instantiate2 formula is the factory contract itself.
    let sender = info.sender.clone();
    let creator = deps.api.addr_canonicalize(env.contract.address.as_str())?;
    let tru_creator = match &instantiate_msg.creator {
        Some(c) => c,
        None => &sender.to_string(),
    };

    // Build a unique, deterministic salt: sha256(sender_bytes || nonce_le_bytes).
    // Using the sender ensures different creators can't collide even at the same nonce.
    let nonce = COLLECTION_NONCE.load(deps.storage)?;
    let mut hasher = Sha256::new();
    hasher.update(sender.as_bytes());
    hasher.update(nonce.to_le_bytes());
    let salt_bytes: [u8; 32] = hasher.finalize().into();
    let salt = Binary::from(salt_bytes.as_slice());

    // Derive the address before dispatching — no reply handler needed.
    let predicted_canonical =
        instantiate2_address(code_info.checksum.as_slice(), &creator, salt.as_slice())
            .map_err(|e| ContractError::Instantiate2AddressError { msg: e.to_string() })?;
    let predicted_addr = deps.api.addr_humanize(&predicted_canonical)?;
    // Record the collection before sending the sub-message.
    svg_collections().save(
        deps.storage,
        predicted_addr.as_ref(),
        &SvgCollection {
            contract: predicted_addr.to_string(),
            creator: tru_creator.to_string(),
            name: instantiate_msg.name.clone(),
            symbol: instantiate_msg.symbol.clone(),
            series: series.clone(),
        },
    )?;

    // Advance nonce so the next collection gets a different salt.
    COLLECTION_NONCE.save(deps.storage, &(nonce + 1))?;

    let wasm_msg = WasmMsg::Instantiate2 {
        admin: Some(tru_creator.to_string()),
        code_id,
        label,
        msg: to_json_binary(&instantiate_msg)?,
        funds: info.funds,
        salt,
    };

    let mut res = Response::new()
        .add_attribute("action", "create_svg_collection")
        .add_attribute("contract", predicted_addr.to_string())
        .add_attribute("creator", info.sender.to_string())
        .add_attribute("series", series.unwrap_or_default())
        .add_message(wasm_msg);
    // Factory default pricing: the factory is the collection's instantiator, so it
    // may set it right after instantiate2 in the same tx.
    if let Some(pricing) = ORACLE_PRICING.may_load(deps.storage)? {
        res = res.add_message(WasmMsg::Execute {
            contract_addr: predicted_addr.to_string(),
            msg: to_json_binary(&cw721_svg::ExecuteMsg::UpdateExtension {
                msg: cw721_svg::SvgExecuteMsgExt::SetOraclePricing {
                    pricing: Some(pricing),
                },
            })?,
            funds: vec![],
        });
    }
    Ok(res)
}

pub fn execute_update_code_id(
    deps: DepsMut,
    info: MessageInfo,
    code_id: u64,
) -> Result<Response, ContractError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    SVG_CODE_ID.save(deps.storage, &code_id)?;
    Ok(Response::new()
        .add_attribute("action", "update_code_id")
        .add_attribute("code_id", code_id.to_string()))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let msg = match msg.into_oracle_query() {
        Ok(oracle) => {
            let pricing = ORACLE_PRICING.may_load(deps.storage)?;
            return terp_oracle::handle_oracle_query(deps, &env, pricing.as_ref(), oracle);
        }
        Err(msg) => msg,
    };
    match msg {
        QueryMsg::Series { series } => {
            let config = SERIES.may_load(deps.storage, &series)?;
            let recent = SERIES_RECENT.may_load(deps.storage, &series)?.unwrap_or_default();
            let now = env.block.time.seconds();
            let next_allowed_at = config.as_ref().and_then(|c| {
                let w = c.window_secs.unwrap_or(DEFAULT_SERIES_WINDOW_SECS);
                let live: Vec<u64> = recent
                    .iter()
                    .copied()
                    .filter(|t| c.min_interval_secs.is_some_and(|m| now < t + m) || now < t + w)
                    .collect();
                series_next_allowed(c, &live).filter(|t| *t > now)
            });
            to_json_binary(&crate::msg::SeriesInfo {
                config,
                recent,
                next_allowed_at,
            })
        }
        QueryMsg::ListSvgCollections { start_after, limit } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_after.as_deref().map(Bound::exclusive);

            let res: Vec<SvgCollection> = svg_collections()
                .range(deps.storage, start, None, Order::Ascending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::ListSvgCollectionsReverse {
            start_before,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_before.as_deref().map(Bound::exclusive);

            let res: Vec<SvgCollection> = svg_collections()
                .range(deps.storage, None, start, Order::Descending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::ListSvgCollectionsByCreator {
            creator,
            start_after,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_after.map(Bound::<String>::exclusive);

            deps.api.addr_validate(&creator)?;

            let res: Vec<SvgCollection> = svg_collections()
                .idx
                .creator
                .prefix(creator)
                .range(deps.storage, start, None, Order::Ascending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::ListSvgCollectionsByCreatorReverse {
            creator,
            start_before,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_before.map(Bound::<String>::exclusive);

            deps.api.addr_validate(&creator)?;

            let res: Vec<SvgCollection> = svg_collections()
                .idx
                .creator
                .prefix(creator)
                .range(deps.storage, None, start, Order::Descending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::CodeId {} => to_json_binary(&SVG_CODE_ID.load(deps.storage)?),
        QueryMsg::DreggIbc {} => to_json_binary(&crate::msg::DreggIbcStatus {
            channel: DREGG_CHANNEL.may_load(deps.storage)?,
            port: DREGG_PORT.may_load(deps.storage)?,
            ibc_only: DREGG_IBC_ONLY.may_load(deps.storage)?.unwrap_or(false),
        }),
        _ => Err(cosmwasm_std::StdError::msg("unknown query")),
    }
}
