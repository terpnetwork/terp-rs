#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    Addr, BankMsg, Binary, Coin, Deps, DepsMut, Empty, Env, Event, MessageInfo, Order, Reply,
    Response, StdResult, Timestamp, Uint256, to_json_binary,
};
use cw_storage_plus::Bound;
use cw2::set_contract_version;
use dao_voting::pre_propose::{PreProposeInfo, ProposalCreationPolicy};

use crate::cid::validate_cid;
use crate::dao;
use crate::error::ContractError;
use crate::msg::{
    AllowlistResponse, BountiesResponse, EligibilityResponse, ExecuteMsg, InstantiateMsg,
    MigrateMsg, ProposerGateMsg, QueryMsg, SubmissionsResponse, SubmitEligibilityResponse,
    UploadEligibilityResponse,
};
use crate::state::{
    ALLOWLIST, BOUNTY_REFS, BOUNTY_SEQ, Bounty, BountyStatus, CONFIG, CREATION_POLICY, Config,
    CreationGate, DAO, DenomRule, ProposerGate, SUBMISSIONS, Submission, bounties,
};

pub const CONTRACT_NAME: &str = "crates.io:terp-bounty-escrow";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Custom event type; CosmWasm emits it as `wasm-terp_bounty`.
pub const EVENT_TYPE: &str = "terp_bounty";
pub const MAINNET_CHAIN_ID: &str = "morocco-1";
pub const MAX_FEE_BPS: u16 = 1000;
/// Hard cap on the on-chain summary, in bytes of UTF-8 (not characters: 280 ASCII chars, ~93 CJK chars).
/// Why 280: the Bounty is loaded and re-saved on every state change and returned by list queries, so its
/// size multiplies. At wasmd's default gas schedule (30 gas/byte written, 10 gas/byte of tx, plus ~3 gas/byte
/// per read) a full 280-byte summary costs roughly 11k gas on create plus ~1k per later read/write: small next to
/// the ~150-200k gas of a create. It also bounds a `bounties` page (limit 100) to <= 28 KB of summary text.
/// It is long enough for a tweet-size abstract; everything else is in the off-chain details doc (details_hash).
pub const SUMMARY_MAX_BYTES: usize = 280;
const DEFAULT_LIMIT: u32 = 30;
const MAX_LIMIT: u32 = 100;

// ---------------------------------------------------------------- instantiate

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    #[cfg(feature = "dev-chain-guard")]
    if env.block.chain_id == MAINNET_CHAIN_ID {
        return Err(ContractError::MainnetGuard(env.block.chain_id));
    }
    let _ = &env;
    nonpayable(&info)?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let fee_collector = msg
        .fee_collector
        .map(|a| deps.api.addr_validate(&a))
        .transpose()?;
    let creation_gate = msg.creation_gate.unwrap_or_default();
    validate_creation_gate(&creation_gate)?;
    let dao_addr = match msg.dao {
        Some(d) => deps.api.addr_validate(&d)?,
        None => info.sender.clone(),
    };
    let cfg = Config {
        owner: deps.api.addr_validate(&msg.owner)?,
        approver: deps.api.addr_validate(&msg.approver)?,
        gate: validate_gate(deps.as_ref(), msg.gate, &creation_gate)?,
        denoms: msg.denoms,
        allow_partial_payouts: msg.allow_partial_payouts,
        fee_bps: msg.fee_bps,
        fee_collector,
        min_duration_secs: msg.min_duration_secs,
        max_duration_secs: msg.max_duration_secs,
        params_version: msg.params_version,
        params_placeholder: msg.params_placeholder,
        min_submit_stake: msg.min_submit_stake,
        min_upload_stake: msg.min_upload_stake,
        creation_gate,
    };
    validate_config(&cfg)?;
    CONFIG.save(deps.storage, &cfg)?;
    BOUNTY_SEQ.save(deps.storage, &0)?;
    for a in msg.initial_allowlist {
        let addr = deps.api.addr_validate(&a)?;
        ALLOWLIST.save(deps.storage, &addr, &Empty {})?;
    }
    let pre_propose_msgs = dao::init(
        deps.storage,
        dao_addr.clone(),
        msg.pre_propose_info
            .unwrap_or(PreProposeInfo::AnyoneMayPropose {}),
    )?;
    Ok(Response::new()
        .add_submessages(pre_propose_msgs)
        .add_attribute("action", "instantiate")
        .add_attribute("dao", dao_addr)
        .add_attribute("owner", cfg.owner)
        .add_attribute("approver", cfg.approver)
        .add_attribute("params_version", cfg.params_version)
        .add_attribute("params_placeholder", cfg.params_placeholder.to_string())
        .add_attribute("min_submit_stake", cfg.min_submit_stake.to_string())
        .add_attribute("min_upload_stake", cfg.min_upload_stake.to_string()))
}

fn validate_creation_gate(g: &CreationGate) -> Result<(), ContractError> {
    match g {
        CreationGate::ProposerGate {} | CreationGate::Anyone {} => Ok(()),
        CreationGate::MinStake { amount } if amount.is_zero() => Err(ContractError::InvalidInput(
            "creation_gate.min_stake.amount must be > 0".into(),
        )),
        CreationGate::MinStake { .. } => Ok(()),
        // TODO(bounties-c): private counterparty-escrow deposit proofs (DESIGN.md §9). Not built.
        CreationGate::CounterpartyEscrowProof { chain } => Err(ContractError::Unimplemented(
            format!("counterparty escrow deposit proofs ({chain}) are a TODO, see DESIGN.md §9"),
        )),
    }
}

fn validate_gate(
    deps: Deps,
    g: ProposerGateMsg,
    creation_gate: &CreationGate,
) -> Result<ProposerGate, ContractError> {
    if let Some(ms) = g.min_stake {
        if ms.is_zero() {
            return Err(ContractError::InvalidInput(
                "min_stake must be > 0 when set".into(),
            ));
        }
    }
    let gate = ProposerGate {
        use_allowlist: g.use_allowlist,
        min_stake: g.min_stake,
        cw4_group: g
            .cw4_group
            .map(|a| deps.api.addr_validate(&a))
            .transpose()?,
    };
    // The ProposerGate must enable something only while it is the creation gate in use.
    if matches!(creation_gate, CreationGate::ProposerGate {})
        && !gate.use_allowlist
        && gate.min_stake.is_none()
        && gate.cw4_group.is_none()
    {
        return Err(ContractError::InvalidInput(
            "at least one proposer gate must be enabled".into(),
        ));
    }
    Ok(gate)
}

fn validate_config(cfg: &Config) -> Result<(), ContractError> {
    if cfg.params_version.trim().is_empty() || cfg.params_version.len() > 128 {
        return Err(ContractError::InvalidInput(
            "params_version must be 1..=128 chars".into(),
        ));
    }
    if cfg.denoms.is_empty() {
        return Err(ContractError::InvalidInput(
            "at least one reward denom is required".into(),
        ));
    }
    let mut seen: Vec<&str> = vec![];
    for d in &cfg.denoms {
        if d.denom.is_empty() || d.min_amount.is_zero() || d.min_amount > d.max_amount {
            return Err(ContractError::InvalidInput(format!(
                "bad denom rule for {}",
                d.denom
            )));
        }
        if seen.contains(&d.denom.as_str()) {
            return Err(ContractError::InvalidInput(format!(
                "duplicate denom {}",
                d.denom
            )));
        }
        seen.push(&d.denom);
    }
    if cfg.fee_bps > MAX_FEE_BPS {
        return Err(ContractError::InvalidInput(format!(
            "fee_bps > {MAX_FEE_BPS}"
        )));
    }
    if cfg.fee_bps > 0 && cfg.fee_collector.is_none() {
        return Err(ContractError::InvalidInput(
            "fee_collector required when fee_bps > 0".into(),
        ));
    }
    if cfg.max_duration_secs == 0 || cfg.min_duration_secs > cfg.max_duration_secs {
        return Err(ContractError::InvalidInput("bad duration bounds".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------- execute

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    if !matches!(msg, ExecuteMsg::CreateBounty { .. }) {
        nonpayable(&info)?;
    }
    // DAO DAO proposal hooks: compare the bounty status before/after the handler, so every
    // state transition fires `ProposalStatusChanged` without touching each handler.
    let is_create = matches!(msg, ExecuteMsg::CreateBounty { .. });
    let tracked = tracked_id(&msg);
    let before = match tracked {
        Some(id) => bounties().may_load(deps.storage, id)?.map(|b| b.status),
        None => None,
    };
    let mut res = dispatch(deps.branch(), env, info, msg)?;
    if is_create {
        let id = BOUNTY_SEQ.load(deps.storage)?;
        let b = bounties().load(deps.storage, id)?;
        res = res.add_submessages(dao::new_bounty_hooks(deps.storage, id, &b.proposer)?);
    } else if let (Some(id), Some(old)) = (tracked, before) {
        let new = bounties().load(deps.storage, id)?.status;
        res = res.add_submessages(dao::status_hooks(deps.storage, id, &old, &new)?);
    }
    Ok(res)
}

fn tracked_id(msg: &ExecuteMsg) -> Option<u64> {
    match msg {
        ExecuteMsg::Approve { id }
        | ExecuteMsg::Reject { id, .. }
        | ExecuteMsg::Cancel { id }
        | ExecuteMsg::Assign { id, .. }
        | ExecuteMsg::Unassign { id }
        | ExecuteMsg::Submit { id, .. }
        | ExecuteMsg::RequestChanges { id, .. }
        | ExecuteMsg::Accept { id }
        | ExecuteMsg::Payout { id, .. }
        | ExecuteMsg::SettleRemainder { id }
        | ExecuteMsg::Expire { id } => Some(*id),
        _ => None,
    }
}

fn dispatch(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateBounty {
            bounty_ref,
            title,
            summary,
            details_hash,
            details_cid,
            reward,
            expires_at,
            proposer,
        } => exec_create(
            deps,
            env,
            info,
            bounty_ref,
            title,
            summary,
            details_hash,
            details_cid,
            reward,
            expires_at,
            proposer,
        ),
        ExecuteMsg::UpdatePreProposeInfo { info: new_info } => {
            dao::exec_update_pre_propose_info(deps, info, new_info)
        }
        ExecuteMsg::AddProposalHook { address } => dao::exec_add_hook(deps, info, address),
        ExecuteMsg::RemoveProposalHook { address } => dao::exec_remove_hook(deps, info, address),
        ExecuteMsg::Approve { id } => exec_approve(deps, env, info, id),
        ExecuteMsg::Reject { id, reason } => exec_reject(deps, env, info, id, reason),
        ExecuteMsg::Cancel { id } => exec_cancel(deps, env, info, id),
        ExecuteMsg::Assign { id, assignee } => exec_assign(deps, env, info, id, assignee),
        ExecuteMsg::Unassign { id } => exec_unassign(deps, env, info, id),
        ExecuteMsg::Submit {
            id,
            submission_hash,
            cid,
        } => exec_submit(deps, env, info, id, submission_hash, cid),
        ExecuteMsg::RequestChanges { id, reason } => {
            exec_request_changes(deps, env, info, id, reason)
        }
        ExecuteMsg::Accept { id } => exec_accept(deps, env, info, id),
        ExecuteMsg::Payout { id, amount } => exec_payout(deps, env, info, id, amount),
        ExecuteMsg::SettleRemainder { id } => exec_settle(deps, env, info, id),
        ExecuteMsg::Expire { id } => exec_expire(deps, env, info, id),
        ExecuteMsg::UpdateConfig {
            owner,
            approver,
            gate,
            denoms,
            allow_partial_payouts,
            fee_bps,
            fee_collector,
            min_duration_secs,
            max_duration_secs,
            params_version,
            params_placeholder,
            min_submit_stake,
            min_upload_stake,
            creation_gate,
        } => {
            let mut cfg = CONFIG.load(deps.storage)?;
            only(&info.sender, &cfg.owner, "owner")?;
            let gov_change = approver.is_some()
                || gate.is_some()
                || denoms.is_some()
                || allow_partial_payouts.is_some()
                || fee_bps.is_some()
                || fee_collector.is_some()
                || min_duration_secs.is_some()
                || max_duration_secs.is_some()
                || params_placeholder.is_some()
                || min_submit_stake.is_some()
                || min_upload_stake.is_some()
                || creation_gate.is_some();
            match &params_version {
                Some(v) if *v == cfg.params_version => {
                    return Err(ContractError::InvalidInput(
                        "params_version must change with the parameters".into(),
                    ));
                }
                None if gov_change => {
                    return Err(ContractError::InvalidInput(
                        "a new params_version is required to change governance parameters".into(),
                    ));
                }
                _ => {}
            }
            if let Some(v) = params_version {
                cfg.params_version = v;
            }
            if let Some(p) = params_placeholder {
                cfg.params_placeholder = p;
            }
            if let Some(o) = owner {
                cfg.owner = deps.api.addr_validate(&o)?;
            }
            if let Some(a) = approver {
                cfg.approver = deps.api.addr_validate(&a)?;
            }
            if let Some(cg) = creation_gate {
                validate_creation_gate(&cg)?;
                cfg.creation_gate = cg;
            }
            if let Some(g) = gate {
                cfg.gate = validate_gate(deps.as_ref(), g, &cfg.creation_gate)?;
            }
            if gate_none_enabled(&cfg) {
                return Err(ContractError::InvalidInput(
                    "at least one proposer gate must be enabled".into(),
                ));
            }
            if let Some(d) = denoms {
                cfg.denoms = d;
            }
            if let Some(p) = allow_partial_payouts {
                cfg.allow_partial_payouts = p;
            }
            if let Some(f) = fee_bps {
                cfg.fee_bps = f;
            }
            if let Some(fc) = fee_collector {
                cfg.fee_collector = Some(deps.api.addr_validate(&fc)?);
            }
            if let Some(m) = min_duration_secs {
                cfg.min_duration_secs = m;
            }
            if let Some(m) = max_duration_secs {
                cfg.max_duration_secs = m;
            }
            if let Some(m) = min_submit_stake {
                cfg.min_submit_stake = m;
            }
            if let Some(m) = min_upload_stake {
                cfg.min_upload_stake = m;
            }
            validate_config(&cfg)?;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_event(
                Event::new(EVENT_TYPE)
                    .add_attribute("action", "update_config")
                    .add_attribute("actor", info.sender)
                    .add_attribute("owner", cfg.owner)
                    .add_attribute("approver", cfg.approver)
                    .add_attribute("params_version", cfg.params_version)
                    .add_attribute("params_placeholder", cfg.params_placeholder.to_string())
                    .add_attribute("min_submit_stake", cfg.min_submit_stake.to_string())
                    .add_attribute("min_upload_stake", cfg.min_upload_stake.to_string()),
            ))
        }
        ExecuteMsg::UpdateAllowlist { add, remove } => {
            let cfg = CONFIG.load(deps.storage)?;
            only(&info.sender, &cfg.owner, "owner")?;
            for a in &add {
                ALLOWLIST.save(deps.storage, &deps.api.addr_validate(a)?, &Empty {})?;
            }
            for r in &remove {
                ALLOWLIST.remove(deps.storage, &deps.api.addr_validate(r)?);
            }
            Ok(Response::new().add_event(
                Event::new(EVENT_TYPE)
                    .add_attribute("action", "update_allowlist")
                    .add_attribute("actor", info.sender)
                    .add_attribute("added", add.join(","))
                    .add_attribute("removed", remove.join(",")),
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn exec_create(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bounty_ref: String,
    title: String,
    summary: String,
    details_hash: String,
    details_cid: Option<String>,
    reward: Coin,
    expires_at: Timestamp,
    proposer: Option<String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;

    // 1. who may create: the dao-voting creation policy first (pre-propose module), then, with no
    //    module installed, the configured creation gate (anyone / min stake / ProposerGate).
    let policy = CREATION_POLICY.load(deps.storage)?;
    let (proposer, via) = match (proposer, &policy) {
        (None, ProposalCreationPolicy::Anyone {}) => {
            let via = creation_gate_check(deps.as_ref(), &cfg, &info.sender)?;
            (info.sender.clone(), via)
        }
        (Some(p), ProposalCreationPolicy::Module { addr }) if *addr == info.sender => (
            deps.api.addr_validate(&p)?,
            "pre_propose_module".to_string(),
        ),
        (_, ProposalCreationPolicy::Module { .. }) => {
            return Err(ContractError::Unauthorized("pre-propose module".into()));
        }
        (Some(_), ProposalCreationPolicy::Anyone {}) => {
            return Err(ContractError::InvalidInput(
                "proposer may only be set by the pre-propose module".into(),
            ));
        }
    };

    // 2. inputs
    if bounty_ref.is_empty() || bounty_ref.len() > 64 {
        return Err(ContractError::InvalidInput(
            "bounty_ref must be 1..=64 chars".into(),
        ));
    }
    if title.trim().is_empty() || title.len() > 140 {
        return Err(ContractError::InvalidInput(
            "title must be 1..=140 chars".into(),
        ));
    }
    validate_summary(&summary)?;
    if !is_sha256_hex(&details_hash) {
        return Err(ContractError::InvalidHash);
    }
    if let Some(c) = &details_cid {
        validate_cid(c).map_err(ContractError::InvalidCid)?;
    }
    if BOUNTY_REFS.has(deps.storage, &bounty_ref) {
        return Err(ContractError::DuplicateRef(bounty_ref));
    }
    let rule: &DenomRule = cfg
        .denoms
        .iter()
        .find(|d| d.denom == reward.denom)
        .ok_or_else(|| ContractError::DenomNotAllowed(reward.denom.clone()))?;
    if reward.amount < rule.min_amount || reward.amount > rule.max_amount {
        return Err(ContractError::AmountOutOfRange {
            denom: reward.denom.clone(),
            amount: reward.amount.to_string(),
            min: rule.min_amount.to_string(),
            max: rule.max_amount.to_string(),
        });
    }
    let now = env.block.time;
    let min_exp = now.plus_seconds(cfg.min_duration_secs);
    let max_exp = now.plus_seconds(cfg.max_duration_secs);
    if expires_at < min_exp || expires_at > max_exp {
        return Err(ContractError::InvalidInput(format!(
            "expires_at must be in [{min_exp}, {max_exp}]"
        )));
    }

    // 3. deposit must be exactly [reward] in the same message
    if info.funds.len() != 1 || info.funds[0] != reward {
        return Err(ContractError::FundsMismatch {
            expected: reward.to_string(),
            got: info
                .funds
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(","),
        });
    }

    // 4. only now is the bounty recorded, as Pending
    let id = BOUNTY_SEQ.update(deps.storage, |s| -> StdResult<u64> { Ok(s + 1) })?;
    let b = Bounty {
        id,
        bounty_ref: bounty_ref.clone(),
        title,
        summary,
        details_hash,
        details_cid,
        proposer,
        reward: reward.clone(),
        paid: Uint256::zero(),
        refunded: Uint256::zero(),
        status: BountyStatus::Pending,
        assignee: None,
        created_at: now,
        created_height: env.block.height,
        expires_at,
        submission_count: 0,
        updated_at: now,
        params_version: cfg.params_version.clone(),
    };
    bounties().save(deps.storage, id, &b)?;
    BOUNTY_REFS.save(deps.storage, &bounty_ref, &id)?;

    Ok(Response::new()
        .add_attribute("action", "create_bounty")
        .add_event(
            base_event("create", &b, None, &info.sender)
                .add_attribute("proposer", b.proposer.as_str())
                .add_attribute("denom", &b.reward.denom)
                .add_attribute("amount", b.reward.amount.to_string())
                .add_attribute("expires_at", b.expires_at.seconds().to_string())
                .add_attribute("eligible_via", via)
                .add_attribute("cap", rule.max_amount.to_string())
                .add_attribute("params_version", &cfg.params_version)
                .add_attribute("params_placeholder", cfg.params_placeholder.to_string())
                .add_attribute("summary", &b.summary)
                .add_attribute("details_hash", &b.details_hash)
                .add_attribute("details_cid", b.details_cid.clone().unwrap_or_default()),
        ))
}

fn exec_approve(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    only(&info.sender, &cfg.approver, "approver")?;
    let mut b = load(deps.as_ref(), id)?;
    require(&b, &[BountyStatus::Pending], "approve")?;
    not_expired(&env, &b)?;
    let prev = transition(deps, &env, &mut b, BountyStatus::Open)?;
    Ok(Response::new().add_event(base_event("approve", &b, Some(prev), &info.sender)))
}

fn exec_reject(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
    reason: Option<String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    only(&info.sender, &cfg.approver, "approver")?;
    let mut b = load(deps.as_ref(), id)?;
    require(&b, &[BountyStatus::Pending], "reject")?;
    let (msg, amt) = refund_all(&mut b);
    let prev = transition(deps, &env, &mut b, BountyStatus::Refunded)?;
    Ok(Response::new().add_messages(msg).add_event(
        base_event("reject", &b, Some(prev), &info.sender)
            .add_attribute("refund_to", b.proposer.as_str())
            .add_attribute("refund_amount", amt.to_string())
            .add_attribute("reason", reason.unwrap_or_default()),
    ))
}

fn exec_cancel(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut b = load(deps.as_ref(), id)?;
    if info.sender == cfg.approver {
        require(
            &b,
            &[
                BountyStatus::Pending,
                BountyStatus::Open,
                BountyStatus::Assigned,
            ],
            "cancel",
        )?;
    } else if info.sender == b.proposer {
        require(&b, &[BountyStatus::Pending, BountyStatus::Open], "cancel")?;
    } else {
        return Err(ContractError::Unauthorized("proposer or approver".into()));
    }
    let (msg, amt) = refund_all(&mut b);
    let prev = transition(deps, &env, &mut b, BountyStatus::Cancelled)?;
    Ok(Response::new().add_messages(msg).add_event(
        base_event("cancel", &b, Some(prev), &info.sender)
            .add_attribute("refund_to", b.proposer.as_str())
            .add_attribute("refund_amount", amt.to_string()),
    ))
}

fn exec_assign(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
    assignee: String,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut b = load(deps.as_ref(), id)?;
    proposer_or_approver(&info.sender, &b, &cfg)?;
    require(&b, &[BountyStatus::Open], "assign")?;
    not_expired(&env, &b)?;
    let who = deps.api.addr_validate(&assignee)?;
    if who == b.proposer {
        return Err(ContractError::InvalidInput(
            "proposer cannot be the assignee".into(),
        ));
    }
    b.assignee = Some(who.clone());
    let prev = transition(deps, &env, &mut b, BountyStatus::Assigned)?;
    Ok(Response::new().add_event(
        base_event("assign", &b, Some(prev), &info.sender).add_attribute("assignee", who),
    ))
}

fn exec_unassign(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut b = load(deps.as_ref(), id)?;
    proposer_or_approver(&info.sender, &b, &cfg)?;
    require(&b, &[BountyStatus::Assigned], "unassign")?;
    let old = b.assignee.take().map(|a| a.to_string()).unwrap_or_default();
    let prev = transition(deps, &env, &mut b, BountyStatus::Open)?;
    Ok(Response::new().add_event(
        base_event("unassign", &b, Some(prev), &info.sender)
            .add_attribute("previous_assignee", old),
    ))
}

fn exec_submit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
    hash: String,
    cid: String,
) -> Result<Response, ContractError> {
    let mut b = load(deps.as_ref(), id)?;
    if b.assignee.as_ref() != Some(&info.sender) {
        return Err(ContractError::Unauthorized("assignee".into()));
    }
    require(&b, &[BountyStatus::Assigned], "submit")?;
    not_expired(&env, &b)?;
    if !is_sha256_hex(&hash) {
        return Err(ContractError::InvalidHash);
    }
    validate_cid(&cid).map_err(ContractError::InvalidCid)?;
    // submission stake gate (governance param; 0 = off)
    let cfg = CONFIG.load(deps.storage)?;
    if !cfg.min_submit_stake.is_zero() {
        let (staked, denom) = bonded(deps.as_ref(), &info.sender)?;
        if staked < cfg.min_submit_stake {
            return Err(ContractError::BelowSubmitStake {
                who: info.sender.to_string(),
                staked: staked.to_string(),
                min: cfg.min_submit_stake.to_string(),
                denom,
            });
        }
    }
    b.submission_count += 1;
    let sub = Submission {
        bounty_id: id,
        seq: b.submission_count,
        submitter: info.sender.clone(),
        hash: hash.clone(),
        cid: cid.clone(),
        submitted_at: env.block.time,
        height: env.block.height,
    };
    SUBMISSIONS.save(deps.storage, (id, sub.seq), &sub)?;
    let prev = transition(deps, &env, &mut b, BountyStatus::Submitted)?;
    Ok(Response::new().add_event(
        base_event("submit", &b, Some(prev), &info.sender)
            .add_attribute("submission_seq", sub.seq.to_string())
            .add_attribute("submission_hash", hash)
            .add_attribute("submission_cid", cid),
    ))
}

fn exec_request_changes(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
    reason: Option<String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut b = load(deps.as_ref(), id)?;
    proposer_or_approver(&info.sender, &b, &cfg)?;
    require(&b, &[BountyStatus::Submitted], "request_changes")?;
    let prev = transition(deps, &env, &mut b, BountyStatus::Assigned)?;
    Ok(Response::new().add_event(
        base_event("request_changes", &b, Some(prev), &info.sender)
            .add_attribute("reason", reason.unwrap_or_default()),
    ))
}

fn exec_accept(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    only(&info.sender, &cfg.approver, "approver")?;
    let mut b = load(deps.as_ref(), id)?;
    require(&b, &[BountyStatus::Submitted], "accept")?;
    let prev = transition(deps, &env, &mut b, BountyStatus::Accepted)?;
    Ok(Response::new().add_event(
        base_event("accept", &b, Some(prev), &info.sender)
            .add_attribute("submission_seq", b.submission_count.to_string()),
    ))
}

fn exec_payout(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
    amount: Option<Uint256>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    only(&info.sender, &cfg.approver, "approver")?;
    let mut b = load(deps.as_ref(), id)?;
    require(&b, &[BountyStatus::Accepted], "payout")?;
    let remaining = b.remaining();
    let amt = amount.unwrap_or(remaining);
    if amt.is_zero() {
        return Err(ContractError::InvalidInput(
            "payout amount must be > 0".into(),
        ));
    }
    if amt > remaining {
        return Err(ContractError::AmountExceedsRemaining {
            amount: amt.to_string(),
            remaining: remaining.to_string(),
        });
    }
    if amt < remaining && !cfg.allow_partial_payouts {
        return Err(ContractError::PartialNotAllowed);
    }
    let assignee = b
        .assignee
        .clone()
        .ok_or_else(|| ContractError::InvalidInput("no assignee".into()))?;
    let fee = amt.multiply_ratio(cfg.fee_bps as u128, 10_000u128);
    let net = amt.checked_sub(fee)?;
    let denom = b.reward.denom.clone();
    let mut msgs = vec![];
    if !net.is_zero() {
        msgs.push(BankMsg::Send {
            to_address: assignee.to_string(),
            amount: vec![Coin::new(net, &denom)],
        });
    }
    if !fee.is_zero() {
        let fc = cfg.fee_collector.clone().expect("validated in config");
        msgs.push(BankMsg::Send {
            to_address: fc.to_string(),
            amount: vec![Coin::new(fee, &denom)],
        });
    }
    b.paid = b.paid.checked_add(amt)?;
    let full = b.remaining().is_zero();
    let mut ev;
    if full {
        let prev = transition(deps, &env, &mut b, BountyStatus::Paid)?;
        ev = base_event("payout", &b, Some(prev), &info.sender);
    } else {
        b.updated_at = env.block.time;
        bounties().save(deps.storage, id, &b)?;
        ev = base_event("payout", &b, Some(BountyStatus::Accepted), &info.sender);
    }
    ev = ev
        .add_attribute("recipient", assignee)
        .add_attribute("denom", denom)
        .add_attribute("gross", amt.to_string())
        .add_attribute("fee", fee.to_string())
        .add_attribute("net", net.to_string())
        .add_attribute("paid_total", b.paid.to_string())
        .add_attribute("remaining", b.remaining().to_string())
        .add_attribute("partial", (!full).to_string());
    Ok(Response::new().add_messages(msgs).add_event(ev))
}

fn exec_settle(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    only(&info.sender, &cfg.approver, "approver")?;
    let mut b = load(deps.as_ref(), id)?;
    require(&b, &[BountyStatus::Accepted], "settle_remainder")?;
    if b.paid.is_zero() {
        return Err(ContractError::InvalidInput(
            "nothing paid yet; use payout".into(),
        ));
    }
    let (msg, amt) = refund_all(&mut b);
    let prev = transition(deps, &env, &mut b, BountyStatus::Paid)?;
    Ok(Response::new().add_messages(msg).add_event(
        base_event("settle_remainder", &b, Some(prev), &info.sender)
            .add_attribute("refund_to", b.proposer.as_str())
            .add_attribute("refund_amount", amt.to_string()),
    ))
}

fn exec_expire(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: u64,
) -> Result<Response, ContractError> {
    let mut b = load(deps.as_ref(), id)?;
    require(
        &b,
        &[
            BountyStatus::Pending,
            BountyStatus::Open,
            BountyStatus::Assigned,
        ],
        "expire",
    )?;
    if env.block.time < b.expires_at {
        return Err(ContractError::NotExpired);
    }
    let (msg, amt) = refund_all(&mut b);
    let prev = transition(deps, &env, &mut b, BountyStatus::Expired)?;
    Ok(Response::new().add_messages(msg).add_event(
        base_event("expire", &b, Some(prev), &info.sender)
            .add_attribute("refund_to", b.proposer.as_str())
            .add_attribute("refund_amount", amt.to_string()),
    ))
}

// ---------------------------------------------------------------- helpers

fn nonpayable(info: &MessageInfo) -> Result<(), ContractError> {
    if info.funds.is_empty() {
        Ok(())
    } else {
        Err(ContractError::NonPayable)
    }
}

fn only(sender: &Addr, who: &Addr, role: &str) -> Result<(), ContractError> {
    if sender == who {
        Ok(())
    } else {
        Err(ContractError::Unauthorized(role.into()))
    }
}

fn proposer_or_approver(sender: &Addr, b: &Bounty, cfg: &Config) -> Result<(), ContractError> {
    if sender == b.proposer || sender == cfg.approver {
        Ok(())
    } else {
        Err(ContractError::Unauthorized("proposer or approver".into()))
    }
}

fn load(deps: Deps, id: u64) -> Result<Bounty, ContractError> {
    Ok(bounties().load(deps.storage, id)?)
}

fn require(b: &Bounty, allowed: &[BountyStatus], action: &str) -> Result<(), ContractError> {
    if allowed.contains(&b.status) {
        Ok(())
    } else {
        Err(ContractError::InvalidTransition {
            from: b.status.as_str().into(),
            action: action.into(),
        })
    }
}

fn not_expired(env: &Env, b: &Bounty) -> Result<(), ContractError> {
    if env.block.time >= b.expires_at {
        Err(ContractError::Expired)
    } else {
        Ok(())
    }
}

fn transition(
    deps: DepsMut,
    env: &Env,
    b: &mut Bounty,
    to: BountyStatus,
) -> Result<BountyStatus, ContractError> {
    let prev = std::mem::replace(&mut b.status, to);
    b.updated_at = env.block.time;
    bounties().save(deps.storage, b.id, b)?;
    Ok(prev)
}

/// Returns the bank refund of everything still escrowed (to the proposer).
fn refund_all(b: &mut Bounty) -> (Vec<BankMsg>, Uint256) {
    let amt = b.remaining();
    b.refunded += amt;
    if amt.is_zero() {
        return (vec![], amt);
    }
    (
        vec![BankMsg::Send {
            to_address: b.proposer.to_string(),
            amount: vec![Coin::new(amt, &b.reward.denom)],
        }],
        amt,
    )
}

fn base_event(action: &str, b: &Bounty, prev: Option<BountyStatus>, actor: &Addr) -> Event {
    Event::new(EVENT_TYPE)
        .add_attribute("action", action)
        .add_attribute("bounty_id", b.id.to_string())
        .add_attribute("bounty_ref", &b.bounty_ref)
        .add_attribute("status", b.status.as_str())
        .add_attribute("prev_status", prev.map(|p| p.as_str()).unwrap_or(""))
        .add_attribute("actor", actor.as_str())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Total delegations of `who` in the chain bond denom (StakingQuery::BondedDenom + AllDelegations).
pub fn bonded(deps: Deps, who: &Addr) -> StdResult<(Uint256, String)> {
    let bond = deps.querier.query_bonded_denom()?;
    let total = deps
        .querier
        .query_all_delegations(who.as_str())?
        .into_iter()
        .filter(|d| d.amount.denom == bond)
        .try_fold(Uint256::zero(), |acc, d| acc.checked_add(d.amount.amount))?;
    Ok((total, bond))
}

/// summary: 1..=SUMMARY_MAX_BYTES bytes of UTF-8 (String is valid UTF-8 already), trimmed-non-empty,
/// no control characters (keeps events/logs single-line and unambiguous).
fn validate_summary(s: &str) -> Result<(), ContractError> {
    if s.trim().is_empty() || s.len() > SUMMARY_MAX_BYTES {
        return Err(ContractError::InvalidInput(format!(
            "summary must be 1..={SUMMARY_MAX_BYTES} bytes of UTF-8, got {}",
            s.len()
        )));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(ContractError::InvalidInput(
            "summary must not contain control characters (newlines, tabs, …)".into(),
        ));
    }
    Ok(())
}

fn gate_none_enabled(cfg: &Config) -> bool {
    matches!(cfg.creation_gate, CreationGate::ProposerGate {})
        && !cfg.gate.use_allowlist
        && cfg.gate.min_stake.is_none()
        && cfg.gate.cw4_group.is_none()
}

/// Creation gate while no pre-propose module is installed. Returns the `eligible_via` label.
fn creation_gate_check(deps: Deps, cfg: &Config, who: &Addr) -> Result<String, ContractError> {
    match &cfg.creation_gate {
        CreationGate::ProposerGate {} => {
            let elig = eligibility(deps, cfg, who)?;
            if !elig.eligible {
                return Err(ContractError::NotEligible(who.to_string()));
            }
            Ok(elig.via.unwrap_or_default())
        }
        CreationGate::Anyone {} => Ok("anyone".into()),
        CreationGate::MinStake { amount } => {
            let (staked, _) = bonded(deps, who)?;
            if staked < *amount {
                return Err(ContractError::NotEligible(who.to_string()));
            }
            Ok("creation_min_stake".into())
        }
        CreationGate::CounterpartyEscrowProof { chain } => Err(ContractError::Unimplemented(
            format!("counterparty escrow deposit proofs ({chain}) are a TODO, see DESIGN.md §9"),
        )),
    }
}

pub fn eligibility(deps: Deps, cfg: &Config, who: &Addr) -> StdResult<EligibilityResponse> {
    if cfg.gate.use_allowlist && ALLOWLIST.has(deps.storage, who) {
        return Ok(EligibilityResponse {
            eligible: true,
            via: Some("allowlist".into()),
            staked: None,
        });
    }
    let mut staked = None;
    if let Some(min) = cfg.gate.min_stake {
        let (total, _) = bonded(deps, who)?;
        staked = Some(total);
        if total >= min {
            return Ok(EligibilityResponse {
                eligible: true,
                via: Some("min_stake".into()),
                staked,
            });
        }
    }
    if let Some(group) = &cfg.gate.cw4_group {
        let w = cw4::Cw4Contract::new(group.clone()).is_member(&deps.querier, who, None)?;
        if w.unwrap_or(0) > 0 {
            return Ok(EligibilityResponse {
                eligible: true,
                via: Some("cw4".into()),
                staked,
            });
        }
    }
    Ok(EligibilityResponse {
        eligible: false,
        via: None,
        staked,
    })
}

// ---------------------------------------------------------------- query

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Bounty { id } => to_json_binary(&bounties().load(deps.storage, id)?),
        QueryMsg::BountyByRef { bounty_ref } => {
            let id = BOUNTY_REFS.load(deps.storage, &bounty_ref)?;
            to_json_binary(&bounties().load(deps.storage, id)?)
        }
        QueryMsg::Bounties {
            status,
            proposer,
            start_after,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let min = start_after.map(Bound::exclusive);
            let proposer = proposer.map(|p| deps.api.addr_validate(&p)).transpose()?;
            let items: Vec<Bounty> = match (status, proposer) {
                // status index, optionally filtered by proposer (bounded by `limit` matches)
                (Some(s), p) => bounties()
                    .idx
                    .status
                    .prefix(s.as_str().to_string())
                    .range(deps.storage, min, None, Order::Ascending)
                    .map(|r| r.map(|(_, b)| b))
                    .filter(|r| match (r, &p) {
                        (Ok(b), Some(p)) => b.proposer == *p,
                        _ => true,
                    })
                    .take(limit)
                    .collect::<StdResult<Vec<_>>>()?,
                (None, Some(p)) => bounties()
                    .idx
                    .proposer
                    .prefix(p)
                    .range(deps.storage, min, None, Order::Ascending)
                    .take(limit)
                    .map(|r| r.map(|(_, b)| b))
                    .collect::<StdResult<Vec<_>>>()?,
                (None, None) => bounties()
                    .range(deps.storage, min, None, Order::Ascending)
                    .take(limit)
                    .map(|r| r.map(|(_, b)| b))
                    .collect::<StdResult<Vec<_>>>()?,
            };
            to_json_binary(&BountiesResponse { bounties: items })
        }
        QueryMsg::Submissions {
            id,
            start_after,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let subs = SUBMISSIONS
                .prefix(id)
                .range(
                    deps.storage,
                    start_after.map(Bound::exclusive),
                    None,
                    Order::Ascending,
                )
                .take(limit)
                .map(|r| r.map(|(_, s)| s))
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&SubmissionsResponse { submissions: subs })
        }
        QueryMsg::CanSubmit { address } => {
            let cfg = CONFIG.load(deps.storage)?;
            let who = deps.api.addr_validate(&address)?;
            let (staked, bond_denom) = bonded(deps, &who)?;
            to_json_binary(&SubmitEligibilityResponse {
                eligible: staked >= cfg.min_submit_stake,
                staked,
                min_submit_stake: cfg.min_submit_stake,
                bond_denom,
            })
        }
        QueryMsg::CanUpload { address } => {
            let cfg = CONFIG.load(deps.storage)?;
            let who = deps.api.addr_validate(&address)?;
            let (staked, bond_denom) = bonded(deps, &who)?;
            to_json_binary(&UploadEligibilityResponse {
                eligible: staked >= cfg.min_upload_stake,
                staked,
                min_upload_stake: cfg.min_upload_stake,
                bond_denom,
            })
        }
        QueryMsg::IsEligible { address } => {
            let cfg = CONFIG.load(deps.storage)?;
            let who = deps.api.addr_validate(&address)?;
            to_json_binary(&eligibility(deps, &cfg, &who)?)
        }
        QueryMsg::Allowlist { start_after, limit } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_after
                .map(|s| deps.api.addr_validate(&s))
                .transpose()?;
            let addrs = ALLOWLIST
                .keys(
                    deps.storage,
                    start.as_ref().map(Bound::exclusive),
                    None,
                    Order::Ascending,
                )
                .take(limit)
                .map(|r| r.map(|a| a.to_string()))
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&AllowlistResponse { addresses: addrs })
        }
        QueryMsg::Info {} => dao::query_info(deps),
        QueryMsg::Dao {} => dao::query_dao(deps),
        QueryMsg::Proposal { proposal_id } => dao::query_proposal(deps, proposal_id),
        QueryMsg::ListProposals { start_after, limit } => dao::query_list(deps, start_after, limit),
        QueryMsg::ReverseProposals {
            start_before,
            limit,
        } => dao::query_reverse(deps, start_before, limit),
        QueryMsg::ProposalCount {} => dao::query_count(deps),
        QueryMsg::NextProposalId {} => dao::query_next_id(deps),
        QueryMsg::ProposalCreationPolicy {} => dao::query_policy(deps),
        QueryMsg::ProposalHooks {} => dao::query_hooks(deps),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    dao::reply(deps, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    // Instances stored before the DAO module: the owner (DAO core in prod) becomes `dao`, creation stays open.
    if !DAO.exists(deps.storage) {
        let owner = CONFIG.load(deps.storage)?.owner;
        DAO.save(deps.storage, &owner)?;
    }
    if !CREATION_POLICY.exists(deps.storage) {
        CREATION_POLICY.save(deps.storage, &ProposalCreationPolicy::Anyone {})?;
    }
    Ok(Response::new().add_attribute("action", "migrate"))
}
