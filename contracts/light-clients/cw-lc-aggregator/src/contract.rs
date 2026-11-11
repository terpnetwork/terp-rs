use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, WasmMsg, to_json_binary,
};
use cw_lc_registry::digest::put_bytes;
use cw_lc_registry::label::validate_label;
use cw_lc_registry::msg::{ConsensusTipResponse, QueryMsg as RegistryQuery};
use cw_lc_rotation::committee;
use cw_lc_rotation::msg::{
    ExecuteMsg as RotationExecute, LivenessResponse, QueryMsg as RotationQuery,
};
use cw_lc_rotation::state::Epoch;

use crate::error::ContractError;
use crate::msg::*;
use crate::state::*;

const CONTRACT_NAME: &str = "crates.io:cw-lc-aggregator";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ROLE: &str = "aggregator";
pub const DOMAIN_PRECONF_V1: &[u8] = b"terp/lc/preconf/v1";

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let target = validate_label(&msg.label, ROLE)?.to_string();
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            label: msg.label.clone(),
            target,
            registry: deps.api.addr_validate(&msg.registry)?,
            rotation: deps.api.addr_validate(&msg.rotation)?,
            client_id: msg.client_id,
            confirm_depth: msg.confirm_depth,
            stall_blocks: msg.stall_blocks.max(1),
            max_confirm_walk: msg.max_confirm_walk.unwrap_or(256),
        },
    )?;
    LIVENESS.save(
        deps.storage,
        &Liveness {
            last_preconf_block: None,
            latest_height: None,
            latest_epoch: None,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("label", msg.label))
}

fn hex32(field: &str, v: &str) -> Result<(), ContractError> {
    match hex::decode(v) {
        Ok(b) if b.len() == 32 => Ok(()),
        _ => Err(ContractError::BadHex(field.into())),
    }
}

pub fn preconf_digest(deps: Deps, env: &Env, cfg: &Config, epoch: u64, h: &Header) -> [u8; 32] {
    let mut buf = Vec::with_capacity(256);
    put_bytes(&mut buf, DOMAIN_PRECONF_V1);
    put_bytes(&mut buf, env.block.chain_id.as_bytes());
    put_bytes(&mut buf, env.contract.address.as_bytes());
    put_bytes(&mut buf, cfg.client_id.as_bytes());
    buf.extend_from_slice(&epoch.to_be_bytes());
    buf.extend_from_slice(&h.height.to_be_bytes());
    put_bytes(&mut buf, h.hash.to_ascii_lowercase().as_bytes());
    put_bytes(&mut buf, h.prev_hash.to_ascii_lowercase().as_bytes());
    put_bytes(&mut buf, h.chain_work.to_ascii_lowercase().as_bytes());
    match &h.payment_root {
        Some(r) => {
            buf.push(1);
            put_bytes(&mut buf, r.to_ascii_lowercase().as_bytes());
        }
        None => buf.push(0),
    }
    deps.api.blake3_256(&buf)
}

fn active_epoch(deps: Deps, cfg: &Config) -> StdResult<Option<Epoch>> {
    deps.querier
        .query_wasm_smart(&cfg.rotation, &RotationQuery::ActiveEpoch {})
}

pub fn liveness(deps: Deps, env: &Env) -> StdResult<LivenessResponse> {
    let cfg = CONFIG.load(deps.storage)?;
    let l = LIVENESS.load(deps.storage)?;
    let active = active_epoch(deps, &cfg)?;
    let stalled = active.as_ref().is_some_and(|e| {
        let since_preconf = l
            .last_preconf_block
            .filter(|_| l.latest_epoch == Some(e.epoch));
        let last = since_preconf.unwrap_or(0).max(e.activated_at.unwrap_or(0));
        env.block.height.saturating_sub(last) > cfg.stall_blocks
    });
    Ok(LivenessResponse {
        active_epoch: active.map(|e| e.epoch),
        last_preconf_block: l.last_preconf_block,
        latest_height: l.latest_height,
        stalled,
    })
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    match msg {
        ExecuteMsg::SubmitPreconf {
            epoch,
            header,
            signature,
        } => {
            let active = active_epoch(deps.as_ref(), &cfg)?
                .filter(|e| e.epoch == epoch)
                .ok_or(ContractError::EpochNotActive(epoch))?;
            hex32("hash", &header.hash)?;
            hex32("prev_hash", &header.prev_hash)?;
            if let Some(r) = &header.payment_root {
                hex32("payment_root", r)?;
            }
            if header.chain_work.is_empty() || hex::decode(&header.chain_work).is_err() {
                return Err(ContractError::BadHex("chain_work".into()));
            }
            let key = active
                .group_key
                .as_ref()
                .ok_or(ContractError::EpochNotActive(epoch))?;
            let digest = preconf_digest(deps.as_ref(), &env, &cfg, epoch, &header);
            if !committee::verify(deps.api, key, &digest, &signature) {
                return Err(ContractError::BadSignature {});
            }
            let h = header.height;
            if let Some(prev) = PRECONFS.may_load(deps.storage, h)? {
                if prev.header.hash.eq_ignore_ascii_case(&header.hash) {
                    return Ok(Response::new()
                        .add_attribute("action", "preconf_duplicate")
                        .add_attribute("height", h.to_string()));
                }
                if prev.status == PreconfStatus::Confirmed {
                    return Err(ContractError::ConflictsConfirmed(h));
                }
                if prev.epoch == epoch {
                    // Two valid group signatures, one height, one epoch.
                    if !EVIDENCE.has(deps.storage, epoch) {
                        EVIDENCE.save(
                            deps.storage,
                            epoch,
                            &Equivocation {
                                epoch,
                                height: h,
                                first: prev.header,
                                first_sig: prev.signature,
                                second: header,
                                second_sig: signature,
                                recorded_at: env.block.height,
                            },
                        )?;
                    }
                    let fault = WasmMsg::Execute {
                        contract_addr: cfg.rotation.to_string(),
                        msg: to_json_binary(&RotationExecute::ReportFault {
                            epoch,
                            note: format!("height {h}"),
                        })?,
                        funds: vec![],
                    };
                    return Ok(Response::new()
                        .add_message(fault)
                        .add_attribute("action", "equivocation")
                        .add_attribute("epoch", epoch.to_string())
                        .add_attribute("height", h.to_string()));
                }
                // Older epoch, unconfirmed: the new committee may reorg it.
            }
            if let Some(p) = h.checked_sub(1)
                && let Some(parent) = PRECONFS.may_load(deps.storage, p)?
                && parent.epoch == epoch
                && !parent.header.hash.eq_ignore_ascii_case(&header.prev_hash)
            {
                return Err(ContractError::BrokenLink(p));
            }
            PRECONFS.save(
                deps.storage,
                h,
                &PreconfRecord {
                    header,
                    epoch,
                    signature,
                    submitted_at: env.block.height,
                    status: PreconfStatus::Preconfirmed,
                },
            )?;
            LIVENESS.update(deps.storage, |mut l| -> StdResult<_> {
                l.last_preconf_block = Some(env.block.height);
                if l.latest_epoch != Some(epoch) || l.latest_height.is_none_or(|x| h > x) {
                    l.latest_height = Some(h);
                }
                l.latest_epoch = Some(epoch);
                Ok(l)
            })?;
            Ok(Response::new()
                .add_attribute("action", "preconf")
                .add_attribute("epoch", epoch.to_string())
                .add_attribute("height", h.to_string()))
        }
        ExecuteMsg::Confirm { height } => {
            let tip: ConsensusTipResponse = deps.querier.query_wasm_smart(
                &cfg.registry,
                &RegistryQuery::ConsensusTip {
                    client_id: cfg.client_id.clone(),
                },
            )?;
            let tip = tip.tip.ok_or(ContractError::NoTip {})?;
            if tip.height < height + cfg.confirm_depth {
                return Err(ContractError::NotDeepEnough {
                    height,
                    tip: tip.height,
                    depth: cfg.confirm_depth,
                });
            }
            if tip.height - height > cfg.max_confirm_walk as u64 {
                return Err(ContractError::TooDeep(cfg.max_confirm_walk));
            }
            // Walk the preconfirmed chain down from the agreed tip.
            let mut want = tip.hash.clone();
            let mut records = Vec::new();
            for h in (height..=tip.height).rev() {
                let r = PRECONFS
                    .may_load(deps.storage, h)?
                    .filter(|r| r.header.hash.eq_ignore_ascii_case(&want))
                    .ok_or(ContractError::ChainMismatch(h))?;
                want = r.header.prev_hash.clone();
                records.push((h, r));
            }
            let upto = tip.height - cfg.confirm_depth;
            let mut n = 0u32;
            for (h, mut r) in records {
                if h <= upto && r.status != PreconfStatus::Confirmed {
                    r.status = PreconfStatus::Confirmed;
                    PRECONFS.save(deps.storage, h, &r)?;
                    n += 1;
                }
            }
            Ok(Response::new()
                .add_attribute("action", "confirm")
                .add_attribute("from", height.to_string())
                .add_attribute("to", upto.to_string())
                .add_attribute("confirmed", n.to_string()))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let cfg = CONFIG.load(deps.storage)?;
    match msg {
        QueryMsg::Config {} => to_json_binary(&cfg),
        QueryMsg::Preconf { height } => to_json_binary(&PRECONFS.may_load(deps.storage, height)?),
        QueryMsg::Liveness {} => to_json_binary(&liveness(deps, &env)?),
        QueryMsg::Evidence { epoch } => to_json_binary(&EVIDENCE.may_load(deps.storage, epoch)?),
        QueryMsg::PreconfDigest { epoch, header } => to_json_binary(&Binary::from(
            preconf_digest(deps, &env, &cfg, epoch, &header).to_vec(),
        )),
        QueryMsg::BestHeader {} => {
            let live = liveness(deps, &env)?;
            let l = LIVENESS.load(deps.storage)?;
            if let (Some(epoch), false) = (live.active_epoch, live.stalled)
                && l.latest_epoch == Some(epoch)
                && let Some(h) = l.latest_height
                && let Some(r) = PRECONFS.may_load(deps.storage, h)?
                && r.epoch == epoch
            {
                return to_json_binary(&BestHeaderResponse {
                    source: HeaderSource::Preconf,
                    height: Some(h),
                    hash: Some(r.header.hash),
                    epoch: Some(epoch),
                    status: Some(r.status),
                });
            }
            let tip: ConsensusTipResponse = deps.querier.query_wasm_smart(
                &cfg.registry,
                &RegistryQuery::ConsensusTip {
                    client_id: cfg.client_id,
                },
            )?;
            to_json_binary(&match tip.tip {
                Some(t) => BestHeaderResponse {
                    source: HeaderSource::RegistryFallback,
                    height: Some(t.height),
                    hash: Some(t.hash),
                    epoch: None,
                    status: None,
                },
                None => BestHeaderResponse {
                    source: HeaderSource::None,
                    height: None,
                    hash: None,
                    epoch: None,
                    status: None,
                },
            })
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(
    deps: DepsMut,
    _env: Env,
    _msg: MigrateMsg,
    _info: cosmwasm_std::MigrateInfo,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "migrate"))
}
