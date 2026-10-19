//! DAO DAO proposal-module surface: bounties are the module's proposals.
//!
//! Message shapes follow dao-proposal-single (`Info`, `Dao`, `Proposal`, `ListProposals`,
//! `ReverseProposals`, `ProposalCount`, `NextProposalId`, `ProposalCreationPolicy`, `ProposalHooks`,
//! `UpdatePreProposeInfo`, `Add/RemoveProposalHook`) and reuse the vendored dao-dao packages
//! (dao-interface, dao-voting, dao-hooks, cw-hooks) instead of copies.
use cosmwasm_std::{
    Addr, Binary, Deps, DepsMut, MessageInfo, Order, Reply, Response, StdResult, Storage, SubMsg,
    to_json_binary,
};
use cw_storage_plus::Bound;
use dao_hooks::proposal::{
    new_proposal_hooks, proposal_completed_hooks, proposal_status_changed_hooks,
};
use dao_voting::pre_propose::{PreProposeInfo, ProposalCreationPolicy};
use dao_voting::reply::TaggedReplyId;
use dao_voting::status::Status;

use crate::error::ContractError;
use crate::msg::{BountyProposalListResponse, BountyProposalResponse};
use crate::state::{
    BOUNTY_SEQ, Bounty, BountyStatus, CREATION_POLICY, DAO, PROPOSAL_HOOKS, bounties,
};

const DEFAULT_LIMIT: u64 = 30;
const MAX_LIMIT: u64 = 100;

/// Saves DAO + the initial creation policy and returns the pre-propose instantiate submessage(s).
pub fn init(storage: &mut dyn Storage, dao: Addr, info: PreProposeInfo) -> StdResult<Vec<SubMsg>> {
    let (policy, msgs) = info.into_initial_policy_and_messages(dao.clone())?;
    DAO.save(storage, &dao)?;
    CREATION_POLICY.save(storage, &policy)?;
    Ok(msgs)
}

fn only_dao(deps: Deps, info: &MessageInfo) -> Result<(), ContractError> {
    if info.sender == DAO.load(deps.storage)? {
        Ok(())
    } else {
        Err(ContractError::Unauthorized("dao".into()))
    }
}

pub fn exec_update_pre_propose_info(
    deps: DepsMut,
    info: MessageInfo,
    new_info: PreProposeInfo,
) -> Result<Response, ContractError> {
    only_dao(deps.as_ref(), &info)?;
    let dao = DAO.load(deps.storage)?;
    let (policy, msgs) = new_info.into_initial_policy_and_messages(dao)?;
    CREATION_POLICY.save(deps.storage, &policy)?;
    Ok(Response::new()
        .add_attribute("action", "update_pre_propose_info")
        .add_submessages(msgs))
}

pub fn exec_add_hook(
    deps: DepsMut,
    info: MessageInfo,
    address: String,
) -> Result<Response, ContractError> {
    only_dao(deps.as_ref(), &info)?;
    let addr = deps.api.addr_validate(&address)?;
    PROPOSAL_HOOKS
        .add_hook(deps.storage, addr)
        .map_err(|e| ContractError::InvalidInput(e.to_string()))?;
    Ok(Response::new()
        .add_attribute("action", "add_proposal_hook")
        .add_attribute("address", address))
}

pub fn exec_remove_hook(
    deps: DepsMut,
    info: MessageInfo,
    address: String,
) -> Result<Response, ContractError> {
    only_dao(deps.as_ref(), &info)?;
    let addr = deps.api.addr_validate(&address)?;
    PROPOSAL_HOOKS
        .remove_hook(deps.storage, addr)
        .map_err(|e| ContractError::InvalidInput(e.to_string()))?;
    Ok(Response::new()
        .add_attribute("action", "remove_proposal_hook")
        .add_attribute("address", address))
}

/// dao-voting status a terminal bounty maps to (for the pre-propose `ProposalCompletedHook`).
fn completed_status(s: &BountyStatus) -> Option<Status> {
    match s {
        BountyStatus::Paid => Some(Status::Executed),
        BountyStatus::Refunded => Some(Status::Rejected),
        BountyStatus::Cancelled | BountyStatus::Expired => Some(Status::Closed),
        _ => None,
    }
}

/// Hooks for a newly created bounty (dao-hooks `NewProposal`).
pub fn new_bounty_hooks(storage: &dyn Storage, id: u64, proposer: &Addr) -> StdResult<Vec<SubMsg>> {
    new_proposal_hooks(PROPOSAL_HOOKS, storage, id, proposer.as_str())
}

/// Hooks for a status change: `ProposalStatusChanged` to every hook, and, on a terminal status,
/// `ProposalCompletedHook` to the pre-propose module (lets a deposit module refund / release).
pub fn status_hooks(
    storage: &dyn Storage,
    id: u64,
    old: &BountyStatus,
    new: &BountyStatus,
) -> StdResult<Vec<SubMsg>> {
    if old == new {
        return Ok(vec![]);
    }
    let mut msgs = proposal_status_changed_hooks(
        PROPOSAL_HOOKS,
        storage,
        id,
        old.as_str().to_string(),
        new.as_str().to_string(),
    )?;
    if let Some(st) = completed_status(new) {
        msgs.extend(proposal_completed_hooks(
            CREATION_POLICY.load(storage)?,
            id,
            st,
        )?);
    }
    Ok(msgs)
}

/// Reply handler, same semantics as dao-proposal-single for the reply ids this module uses.
pub fn reply(deps: DepsMut, msg: Reply) -> Result<Response, ContractError> {
    let id = msg.id;
    match TaggedReplyId::new(id).map_err(|_| ContractError::InvalidReplyId(id))? {
        TaggedReplyId::FailedProposalHook(idx) => {
            let addr = PROPOSAL_HOOKS
                .remove_hook_by_index(deps.storage, idx)
                .map_err(|e| ContractError::InvalidInput(e.to_string()))?;
            Ok(Response::new().add_attribute("removed_proposal_hook", format!("{addr}:{idx}")))
        }
        TaggedReplyId::PreProposeModuleInstantiation => {
            let res = cw_reply_helper::parse_reply_instantiate_data(msg)
                .map_err(|e| ContractError::InvalidInput(e.to_string()))?;
            let module = deps.api.addr_validate(&res.contract_address)?;
            CREATION_POLICY.save(
                deps.storage,
                &ProposalCreationPolicy::Module { addr: module },
            )?;
            let r =
                Response::new().add_attribute("update_pre_propose_module", res.contract_address);
            Ok(match res.data {
                Some(d) => r.set_data(d),
                None => r,
            })
        }
        TaggedReplyId::FailedPreProposeModuleHook => match CREATION_POLICY.load(deps.storage)? {
            ProposalCreationPolicy::Anyone {} => Err(ContractError::InvalidReplyId(id)),
            ProposalCreationPolicy::Module { addr } => {
                // Same as upstream: a pre-propose module that errors on a hook is removed (fail open).
                CREATION_POLICY.save(deps.storage, &ProposalCreationPolicy::Anyone {})?;
                Ok(Response::new().add_attribute("failed_prepropose_hook", addr))
            }
        },
        _ => Err(ContractError::InvalidReplyId(id)),
    }
}

fn to_resp(r: StdResult<(u64, Bounty)>) -> StdResult<BountyProposalResponse> {
    r.map(|(id, proposal)| BountyProposalResponse { id, proposal })
}

pub fn query_info(deps: Deps) -> StdResult<Binary> {
    let info = cw2::get_contract_version(deps.storage)?;
    to_json_binary(&dao_interface::voting::InfoResponse { info })
}

pub fn query_dao(deps: Deps) -> StdResult<Binary> {
    to_json_binary(&DAO.load(deps.storage)?)
}

pub fn query_proposal(deps: Deps, id: u64) -> StdResult<Binary> {
    let proposal = bounties().load(deps.storage, id)?;
    to_json_binary(&BountyProposalResponse { id, proposal })
}

pub fn query_list(deps: Deps, start_after: Option<u64>, limit: Option<u64>) -> StdResult<Binary> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let proposals = bounties()
        .range(
            deps.storage,
            start_after.map(Bound::exclusive),
            None,
            Order::Ascending,
        )
        .take(limit)
        .map(to_resp)
        .collect::<StdResult<Vec<_>>>()?;
    to_json_binary(&BountyProposalListResponse { proposals })
}

pub fn query_reverse(
    deps: Deps,
    start_before: Option<u64>,
    limit: Option<u64>,
) -> StdResult<Binary> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let proposals = bounties()
        .range(
            deps.storage,
            None,
            start_before.map(Bound::exclusive),
            Order::Descending,
        )
        .take(limit)
        .map(to_resp)
        .collect::<StdResult<Vec<_>>>()?;
    to_json_binary(&BountyProposalListResponse { proposals })
}

pub fn query_count(deps: Deps) -> StdResult<Binary> {
    to_json_binary(&BOUNTY_SEQ.load(deps.storage)?)
}

pub fn query_next_id(deps: Deps) -> StdResult<Binary> {
    to_json_binary(&(BOUNTY_SEQ.load(deps.storage)? + 1))
}

pub fn query_policy(deps: Deps) -> StdResult<Binary> {
    to_json_binary(&CREATION_POLICY.load(deps.storage)?)
}

pub fn query_hooks(deps: Deps) -> StdResult<Binary> {
    to_json_binary(&PROPOSAL_HOOKS.query_hooks(deps)?)
}
