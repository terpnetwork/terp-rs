use cosmwasm_std::{
    Addr, Binary, Deps, DepsMut, Env, MessageInfo, Order, Response, StdResult, Storage,
    to_json_binary,
};
use cw_lc_registry::digest::put_bytes;
use cw_lc_registry::label::validate_label;
use cw_lc_registry::msg::QueryMsg as RegistryQuery;
use cw_lc_registry::state::{ClientRecord, OperatorInfo, StoredReport};

use crate::committee::{self, Committee, derive_committee, parse_key};
use crate::error::ContractError;
use crate::msg::*;
use crate::state::*;

const CONTRACT_NAME: &str = "crates.io:cw-lc-rotation";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ROLE: &str = "rotation";
pub const DOMAIN_ACTIVATE_V1: &[u8] = b"terp/lc/rotation/activate/v1";
pub const DOMAIN_MEMBERS_V1: &[u8] = b"terp/lc/rotation/members/v1";

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let target = validate_label(&msg.label, ROLE)?.to_string();
    let policy = msg.policy.unwrap_or_default();
    policy.validate()?;
    let admin = match msg.admin {
        Some(a) => deps.api.addr_validate(&a)?,
        None => info.sender,
    };
    let registry = deps.api.addr_validate(&msg.registry)?;
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin,
            label: msg.label.clone(),
            target,
            registry,
            client_id: msg.client_id,
            aggregator: None,
            policy,
        },
    )?;
    ACTIVE.save(deps.storage, &None)?;
    PENDING.save(deps.storage, &None)?;
    LAST_EPOCH.save(deps.storage, &0)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("label", msg.label))
}

/// Eligible `(operator, weight)` from the registry: client attestors that are
/// registered, advertise the policy capability, have weight > 0 and reported
/// within `active_window_blocks`.
pub fn eligible(deps: Deps, env: &Env, cfg: &Config) -> StdResult<Vec<(Addr, u64)>> {
    let client: ClientRecord = deps.querier.query_wasm_smart(
        &cfg.registry,
        &RegistryQuery::Client {
            client_id: cfg.client_id.clone(),
        },
    )?;
    let mut out = Vec::new();
    for a in client.attestors {
        let Ok(op) = deps.querier.query_wasm_smart::<OperatorInfo>(
            &cfg.registry,
            &RegistryQuery::Operator {
                operator: a.to_string(),
            },
        ) else {
            continue; // deregistered
        };
        if op.weight == 0 || !op.capabilities.iter().any(|c| c == &cfg.policy.capability) {
            continue;
        }
        let report: Option<StoredReport> = deps.querier.query_wasm_smart(
            &cfg.registry,
            &RegistryQuery::NodeState {
                client_id: cfg.client_id.clone(),
                operator: a.to_string(),
            },
        )?;
        let fresh = report.is_some_and(|r| {
            env.block.height.saturating_sub(r.reported_height) <= cfg.policy.active_window_blocks
        });
        if fresh {
            out.push((op.operator, op.weight));
        }
    }
    Ok(out)
}

pub fn members_hash(deps: Deps, c: &Committee) -> [u8; 32] {
    let mut buf = Vec::with_capacity(64 + c.members.len() * 64);
    put_bytes(&mut buf, DOMAIN_MEMBERS_V1);
    buf.extend_from_slice(&c.total_shares.to_be_bytes());
    buf.extend_from_slice(&c.threshold.to_be_bytes());
    for m in &c.members {
        put_bytes(&mut buf, m.operator.as_bytes());
        buf.extend_from_slice(&m.first_share.to_be_bytes());
        buf.extend_from_slice(&m.shares.to_be_bytes());
    }
    deps.api.blake3_256(&buf)
}

pub fn activation_digest(
    deps: Deps,
    env: &Env,
    cfg: &Config,
    e: &Epoch,
    group_key: &[u8],
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(256);
    put_bytes(&mut buf, DOMAIN_ACTIVATE_V1);
    put_bytes(&mut buf, env.block.chain_id.as_bytes());
    put_bytes(&mut buf, env.contract.address.as_bytes());
    put_bytes(&mut buf, cfg.client_id.as_bytes());
    buf.extend_from_slice(&e.epoch.to_be_bytes());
    put_bytes(&mut buf, group_key);
    put_bytes(&mut buf, e.members_hash.as_slice());
    deps.api.blake3_256(&buf)
}

fn pending_live(storage: &dyn Storage, env: &Env, cfg: &Config) -> StdResult<Option<Epoch>> {
    let Some(id) = PENDING.load(storage)? else {
        return Ok(None);
    };
    let e = EPOCHS.load(storage, id)?;
    if env.block.height > e.proposed_at + cfg.policy.dkg_timeout_blocks {
        return Ok(None);
    }
    Ok(Some(e))
}

/// Snapshot the registry set into a new pending epoch (expiring any stale one).
fn propose(
    deps: &mut DepsMut,
    env: &Env,
    cfg: &Config,
    reason: ProposalReason,
) -> Result<Epoch, ContractError> {
    if let Some(id) = PENDING.load(deps.storage)? {
        let mut old = EPOCHS.load(deps.storage, id)?;
        old.status = EpochStatus::Expired;
        old.ended_at = Some(env.block.height);
        EPOCHS.save(deps.storage, id, &old)?;
    }
    let c = derive_committee(eligible(deps.as_ref(), env, cfg)?, &cfg.policy)?;
    let id = LAST_EPOCH.load(deps.storage)? + 1;
    let e = Epoch {
        epoch: id,
        status: EpochStatus::Pending,
        reason,
        members_hash: Binary::from(members_hash(deps.as_ref(), &c).to_vec()),
        members: c.members,
        total_shares: c.total_shares,
        threshold: c.threshold,
        group_key: None,
        scheme: None,
        proposed_at: env.block.height,
        activated_at: None,
        ended_at: None,
    };
    EPOCHS.save(deps.storage, id, &e)?;
    LAST_EPOCH.save(deps.storage, &id)?;
    PENDING.save(deps.storage, &Some(id))?;
    Ok(e)
}

/// Fault the active epoch (fallback mode) and try to propose the next one.
fn fault_active(
    mut deps: DepsMut,
    env: &Env,
    cfg: &Config,
    epoch: u64,
    reason: String,
    next: ProposalReason,
) -> Result<Response, ContractError> {
    let mut e = EPOCHS.load(deps.storage, epoch)?;
    if e.status != EpochStatus::Active {
        return Err(ContractError::NoActiveEpoch {});
    }
    e.status = EpochStatus::Faulted {
        reason: reason.clone(),
    };
    e.ended_at = Some(env.block.height);
    EPOCHS.save(deps.storage, epoch, &e)?;
    ACTIVE.save(deps.storage, &None)?;
    // If the set is too small right now the fault still stands: the aggregator
    // serves the registry k-of-n tip until someone bootstraps.
    let next_id = match propose(&mut deps, env, cfg, next) {
        Ok(n) => n.epoch.to_string(),
        Err(err) => format!("none ({err})"),
    };
    Ok(Response::new()
        .add_attribute("action", "fault_epoch")
        .add_attribute("epoch", epoch.to_string())
        .add_attribute("reason", reason)
        .add_attribute("next_pending", next_id))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    match msg {
        ExecuteMsg::SetAggregator { aggregator } => {
            if info.sender != cfg.admin {
                return Err(ContractError::Unauthorized {});
            }
            if cfg.aggregator.is_some() {
                return Err(ContractError::AggregatorSet {});
            }
            cfg.aggregator = Some(deps.api.addr_validate(&aggregator)?);
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "set_aggregator"))
        }
        ExecuteMsg::UpdatePolicy { policy } => {
            if info.sender != cfg.admin {
                return Err(ContractError::Unauthorized {});
            }
            policy.validate()?;
            cfg.policy = policy;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "update_policy"))
        }
        ExecuteMsg::Bootstrap {} => {
            if ACTIVE.load(deps.storage)?.is_some() {
                return Err(ContractError::AlreadyActive {});
            }
            if pending_live(deps.storage, &env, &cfg)?.is_some() {
                return Err(ContractError::PendingInProgress {});
            }
            let e = propose(&mut deps, &env, &cfg, ProposalReason::Bootstrap)?;
            Ok(pending_response("bootstrap", &e))
        }
        ExecuteMsg::RotateShard { note } => {
            let active = ACTIVE
                .load(deps.storage)?
                .ok_or(ContractError::NoActiveEpoch {})?;
            let a = EPOCHS.load(deps.storage, active)?;
            if info.sender != cfg.admin && a.member(&info.sender).is_none() {
                return Err(ContractError::Unauthorized {});
            }
            if pending_live(deps.storage, &env, &cfg)?.is_some() {
                return Err(ContractError::PendingInProgress {});
            }
            let e = propose(
                &mut deps,
                &env,
                &cfg,
                ProposalReason::RotateShard {
                    by: info.sender,
                    note,
                },
            )?;
            Ok(pending_response("rotate_shard", &e))
        }
        ExecuteMsg::ReportLivenessFault {} => {
            let agg = cfg
                .aggregator
                .clone()
                .ok_or(ContractError::NoAggregator {})?;
            let l: LivenessResponse = deps
                .querier
                .query_wasm_smart(agg, &AggregatorQuery::Liveness {})?;
            let (Some(epoch), true) = (l.active_epoch, l.stalled) else {
                return Err(ContractError::NotStalled {});
            };
            fault_active(
                deps,
                &env,
                &cfg,
                epoch,
                "liveness".into(),
                ProposalReason::LivenessFault,
            )
        }
        ExecuteMsg::ReportFault { epoch, note } => {
            if Some(&info.sender) != cfg.aggregator.as_ref() {
                return Err(ContractError::Unauthorized {});
            }
            if ACTIVE.load(deps.storage)? != Some(epoch) {
                // Already rotated away: evidence is kept by the aggregator.
                return Ok(Response::new().add_attribute("action", "fault_ignored"));
            }
            fault_active(
                deps,
                &env,
                &cfg,
                epoch,
                format!("equivocation: {note}"),
                ProposalReason::Equivocation { note },
            )
        }
        ExecuteMsg::AckEpoch { epoch, group_key } => {
            let e = pending_live(deps.storage, &env, &cfg)?
                .filter(|e| e.epoch == epoch)
                .ok_or(ContractError::NotPending(epoch))?;
            if e.member(&info.sender).is_none() {
                return Err(ContractError::NotMember(epoch));
            }
            parse_key(&group_key)?;
            ACKS.save(deps.storage, (epoch, &info.sender), &group_key)?;
            Ok(Response::new()
                .add_attribute("action", "ack_epoch")
                .add_attribute("epoch", epoch.to_string())
                .add_attribute("member", info.sender))
        }
        ExecuteMsg::ActivateEpoch {
            epoch,
            group_key,
            proof_sig,
            handoff_sig,
        } => {
            let Some(mut e) = PENDING
                .load(deps.storage)?
                .filter(|id| *id == epoch)
                .map(|id| EPOCHS.load(deps.storage, id))
                .transpose()?
            else {
                return Err(ContractError::NotPending(epoch));
            };
            if env.block.height > e.proposed_at + cfg.policy.dkg_timeout_blocks {
                return Err(ContractError::Expired(epoch));
            }
            // Key parameters must be exactly the derived committee.
            let (scheme, t, n) = parse_key(&group_key)?;
            if t != e.threshold || n != e.total_shares {
                return Err(ContractError::KeyMismatch {
                    t,
                    n,
                    want_t: e.threshold,
                    want_n: e.total_shares,
                });
            }
            // ≥ threshold shares acknowledged this exact key.
            let mut acked: u32 = 0;
            for m in &e.members {
                if ACKS.may_load(deps.storage, (epoch, &m.operator))?.as_ref() == Some(&group_key) {
                    acked += m.shares as u32;
                }
            }
            if acked < e.threshold as u32 {
                return Err(ContractError::BelowThreshold {
                    have: acked,
                    need: e.threshold as u32,
                });
            }
            let digest = activation_digest(deps.as_ref(), &env, &cfg, &e, &group_key);
            if !committee::verify(deps.api, &group_key, &digest, &proof_sig) {
                return Err(ContractError::BadSignature {});
            }
            // Planned rotation: the outgoing key endorses the incoming one.
            if let Some(prev_id) = ACTIVE.load(deps.storage)? {
                let mut prev = EPOCHS.load(deps.storage, prev_id)?;
                let sig = handoff_sig.ok_or(ContractError::HandoffRequired {})?;
                let prev_key = prev.group_key.clone().ok_or(ContractError::BadKey {})?;
                if !committee::verify(deps.api, &prev_key, &digest, &sig) {
                    return Err(ContractError::BadSignature {});
                }
                prev.status = EpochStatus::Retired;
                prev.ended_at = Some(env.block.height);
                EPOCHS.save(deps.storage, prev_id, &prev)?;
            }
            e.status = EpochStatus::Active;
            e.group_key = Some(group_key);
            e.scheme = Some(scheme);
            e.activated_at = Some(env.block.height);
            EPOCHS.save(deps.storage, epoch, &e)?;
            ACTIVE.save(deps.storage, &Some(epoch))?;
            PENDING.save(deps.storage, &None)?;
            Ok(Response::new()
                .add_attribute("action", "activate_epoch")
                .add_attribute("epoch", epoch.to_string())
                .add_attribute("threshold", format!("{}/{}", e.threshold, e.total_shares)))
        }
    }
}

fn pending_response(action: &str, e: &Epoch) -> Response {
    Response::new()
        .add_attribute("action", action)
        .add_attribute("epoch", e.epoch.to_string())
        .add_attribute("members", e.members.len().to_string())
        .add_attribute("threshold", format!("{}/{}", e.threshold, e.total_shares))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let cfg = CONFIG.load(deps.storage)?;
    match msg {
        QueryMsg::Config {} => to_json_binary(&cfg),
        QueryMsg::Epoch { epoch } => to_json_binary(&EPOCHS.load(deps.storage, epoch)?),
        QueryMsg::ActiveEpoch {} => to_json_binary(
            &ACTIVE
                .load(deps.storage)?
                .map(|id| EPOCHS.load(deps.storage, id))
                .transpose()?,
        ),
        QueryMsg::PendingEpoch {} => to_json_binary(&pending_live(deps.storage, &env, &cfg)?),
        QueryMsg::Acks { epoch } => {
            let acks = ACKS
                .prefix(epoch)
                .range(deps.storage, None, None, Order::Ascending)
                .map(|r| {
                    r.map(|(a, k)| AckView {
                        operator: a.to_string(),
                        group_key: k,
                    })
                })
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&acks)
        }
        QueryMsg::PreviewCommittee {} => {
            let c = derive_committee(eligible(deps, &env, &cfg)?, &cfg.policy)
                .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
            to_json_binary(&c)
        }
        QueryMsg::ActivationDigest { epoch, group_key } => {
            let e = EPOCHS.load(deps.storage, epoch)?;
            to_json_binary(&Binary::from(
                activation_digest(deps, &env, &cfg, &e, &group_key).to_vec(),
            ))
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
