use std::collections::BTreeMap;

use cosmwasm_std::{
    Addr, Binary, Deps, DepsMut, Env, MessageInfo, Order, Response, StdResult, to_json_binary,
};
use cw_storage_plus::Bound;

use crate::digest::report_digest;
use crate::error::ContractError;
use crate::label::{validate_client_id, validate_label};
use crate::msg::*;
use crate::quorum::{QuorumCheck, QuorumVerdict, query_root, verdict};
use crate::state::*;

const CONTRACT_NAME: &str = "crates.io:cw-lc-registry";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ROLE: &str = "registry";
const DEFAULT_LIMIT: u32 = 30;
const MAX_LIMIT: u32 = 100;
const MAX_ATTESTORS: usize = 64;

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let target = validate_label(&msg.label, ROLE)?.to_string();
    let admin = match msg.admin {
        Some(a) => deps.api.addr_validate(&a)?,
        None => info.sender,
    };
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin: admin.clone(),
            label: msg.label.clone(),
            target,
            hash_market_capability: msg
                .hash_market_capability
                .map(check_capability)
                .transpose()?,
            max_report_age_blocks: msg.max_report_age_blocks,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("label", msg.label)
        .add_attribute("admin", admin))
}

fn check_capability(
    c: crate::quorum::HashMarketCapability,
) -> Result<crate::quorum::HashMarketCapability, ContractError> {
    non_empty("hash_market_capability.chain_uid", &c.chain_uid)?;
    non_empty("hash_market_capability.algo", &c.algo)?;
    Ok(c)
}

fn check_pubkey(pk: &Binary) -> Result<(), ContractError> {
    if pk.len() != 32 {
        return Err(ContractError::BadPubkey {});
    }
    Ok(())
}

fn non_empty(field: &str, v: &str) -> Result<(), ContractError> {
    if v.trim().is_empty() {
        return Err(ContractError::Empty(field.into()));
    }
    Ok(())
}

fn check_hex32(field: &str, v: &str) -> Result<(), ContractError> {
    match hex::decode(v) {
        Ok(b) if b.len() == 32 => Ok(()),
        _ => Err(ContractError::BadHex(field.into())),
    }
}

fn check_hex_any(field: &str, v: &str) -> Result<(), ContractError> {
    if v.is_empty() || v.len() > 128 || hex::decode(v).is_err() {
        return Err(ContractError::BadHex(field.into()));
    }
    Ok(())
}

fn validate_attestors(
    deps: Deps,
    attestors: Vec<String>,
    min: u32,
) -> Result<Vec<Addr>, ContractError> {
    let mut out: Vec<Addr> = Vec::with_capacity(attestors.len());
    for a in attestors {
        let addr = deps.api.addr_validate(&a)?;
        if !OPERATORS.has(deps.storage, &addr) {
            return Err(ContractError::OperatorNotFound(a));
        }
        if !out.contains(&addr) {
            out.push(addr);
        }
    }
    let n = out.len() as u32;
    if out.len() > MAX_ATTESTORS || min == 0 || min > n {
        return Err(ContractError::BadThreshold { attestors: n });
    }
    Ok(out)
}

fn owned_client(deps: Deps, client_id: &str, sender: &Addr) -> Result<ClientRecord, ContractError> {
    let c = CLIENTS
        .may_load(deps.storage, client_id)?
        .ok_or_else(|| ContractError::ClientNotFound(client_id.into()))?;
    if c.owner != sender {
        return Err(ContractError::Unauthorized {});
    }
    Ok(c)
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::RegisterOperator {
            peer_id,
            endpoint,
            pubkey,
            capabilities,
        } => {
            if OPERATORS.has(deps.storage, &info.sender) {
                return Err(ContractError::OperatorExists {});
            }
            non_empty("peer_id", &peer_id)?;
            non_empty("endpoint", &endpoint)?;
            check_pubkey(&pubkey)?;
            OPERATORS.save(
                deps.storage,
                &info.sender,
                &OperatorInfo {
                    operator: info.sender.clone(),
                    peer_id: peer_id.clone(),
                    endpoint,
                    pubkey,
                    capabilities,
                    weight: 1,
                    registered_height: env.block.height,
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "register_operator")
                .add_attribute("operator", info.sender)
                .add_attribute("peer_id", peer_id))
        }
        ExecuteMsg::UpdateOperator {
            endpoint,
            pubkey,
            capabilities,
        } => {
            let mut op = OPERATORS
                .may_load(deps.storage, &info.sender)?
                .ok_or_else(|| ContractError::OperatorNotFound(info.sender.to_string()))?;
            if let Some(e) = endpoint {
                non_empty("endpoint", &e)?;
                op.endpoint = e;
            }
            if let Some(pk) = pubkey {
                check_pubkey(&pk)?;
                op.pubkey = pk;
            }
            if let Some(c) = capabilities {
                op.capabilities = c;
            }
            OPERATORS.save(deps.storage, &info.sender, &op)?;
            Ok(Response::new()
                .add_attribute("action", "update_operator")
                .add_attribute("operator", info.sender))
        }
        ExecuteMsg::DeregisterOperator {} => {
            if !OPERATORS.has(deps.storage, &info.sender) {
                return Err(ContractError::OperatorNotFound(info.sender.to_string()));
            }
            OPERATORS.remove(deps.storage, &info.sender);
            // Client attestor lists still name the operator; ConsensusTip skips
            // operators that are no longer registered.
            Ok(Response::new()
                .add_attribute("action", "deregister_operator")
                .add_attribute("operator", info.sender))
        }
        ExecuteMsg::RegisterClient {
            client_id,
            kind,
            target,
            attestors,
            min_attestations,
            ibc_client_id,
        } => {
            validate_client_id(&client_id)?;
            non_empty("target.chain_id", &target.chain_id)?;
            if CLIENTS.has(deps.storage, &client_id) {
                return Err(ContractError::ClientExists(client_id));
            }
            let attestors = validate_attestors(deps.as_ref(), attestors, min_attestations)?;
            let rec = ClientRecord {
                client_id: client_id.clone(),
                owner: info.sender.clone(),
                kind,
                target,
                attestors,
                min_attestations,
                ibc_client_id: ibc_client_id.filter(|s| !s.trim().is_empty()),
                created_height: env.block.height,
            };
            CLIENTS.save(deps.storage, &client_id, &rec)?;
            Ok(Response::new()
                .add_attribute("action", "register_client")
                .add_attribute("client_id", client_id)
                .add_attribute("owner", info.sender))
        }
        ExecuteMsg::SetAttestors {
            client_id,
            attestors,
            min_attestations,
        } => {
            let mut c = owned_client(deps.as_ref(), &client_id, &info.sender)?;
            c.attestors = validate_attestors(deps.as_ref(), attestors, min_attestations)?;
            c.min_attestations = min_attestations;
            CLIENTS.save(deps.storage, &client_id, &c)?;
            Ok(Response::new()
                .add_attribute("action", "set_attestors")
                .add_attribute("client_id", client_id))
        }
        ExecuteMsg::BindIbcClient {
            client_id,
            ibc_client_id,
        } => {
            non_empty("ibc_client_id", &ibc_client_id)?;
            let mut c = owned_client(deps.as_ref(), &client_id, &info.sender)?;
            c.ibc_client_id = Some(ibc_client_id.clone());
            CLIENTS.save(deps.storage, &client_id, &c)?;
            Ok(Response::new()
                .add_attribute("action", "bind_ibc_client")
                .add_attribute("client_id", client_id)
                .add_attribute("ibc_client_id", ibc_client_id))
        }
        ExecuteMsg::ReportNodeState {
            client_id,
            state,
            signature,
        } => {
            let c = CLIENTS
                .may_load(deps.storage, &client_id)?
                .ok_or_else(|| ContractError::ClientNotFound(client_id.clone()))?;
            if !c.attestors.contains(&info.sender) {
                return Err(ContractError::NotAttestor {});
            }
            let op = OPERATORS
                .may_load(deps.storage, &info.sender)?
                .ok_or_else(|| ContractError::OperatorNotFound(info.sender.to_string()))?;
            check_hex32("tip_hash", &state.tip_hash)?;
            check_hex32("prev_hash", &state.prev_hash)?;
            check_hex_any("chain_work", &state.chain_work)?;
            if signature.len() != 64 {
                return Err(ContractError::BadSignatureLen {});
            }
            if let Some(prev) = REPORTS.may_load(deps.storage, (&client_id, &info.sender))?
                && state.observed_at_unix <= prev.state.observed_at_unix
            {
                return Err(ContractError::StaleReport {});
            }
            let digest = report_digest(
                deps.api,
                &env.block.chain_id,
                &env.contract.address,
                &client_id,
                &info.sender,
                &state,
            );
            if !deps
                .api
                .ed25519_verify(&digest, &signature, &op.pubkey)
                .map_err(|_| ContractError::BadSignature {})?
            {
                return Err(ContractError::BadSignature {});
            }
            // hm-vext capability quorum: never accept a tip that contradicts
            // the quorum root at the same height.
            if let Some(cap) = CONFIG.load(deps.storage)?.hash_market_capability {
                let root = query_root(deps.as_ref(), &cap);
                if verdict(root.as_ref(), state.tip_height, &state.tip_hash)
                    == QuorumVerdict::Conflicts
                {
                    return Err(ContractError::QuorumConflict {
                        height: state.tip_height,
                    });
                }
            }
            let height = state.tip_height;
            REPORTS.save(
                deps.storage,
                (&client_id, &info.sender),
                &StoredReport {
                    operator: info.sender.clone(),
                    state,
                    reported_height: env.block.height,
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "report_node_state")
                .add_attribute("client_id", client_id)
                .add_attribute("operator", info.sender)
                .add_attribute("tip_height", height.to_string()))
        }
        ExecuteMsg::SetOperatorWeight { operator, weight } => {
            let cfg = CONFIG.load(deps.storage)?;
            if info.sender != cfg.admin {
                return Err(ContractError::Unauthorized {});
            }
            let a = deps.api.addr_validate(&operator)?;
            let mut op = OPERATORS
                .may_load(deps.storage, &a)?
                .ok_or_else(|| ContractError::OperatorNotFound(operator.clone()))?;
            op.weight = weight;
            OPERATORS.save(deps.storage, &a, &op)?;
            Ok(Response::new()
                .add_attribute("action", "set_operator_weight")
                .add_attribute("operator", a)
                .add_attribute("weight", weight.to_string()))
        }
        ExecuteMsg::UpdateConfig {
            admin,
            hash_market_capability,
            max_report_age_blocks,
        } => {
            let mut cfg = CONFIG.load(deps.storage)?;
            if info.sender != cfg.admin {
                return Err(ContractError::Unauthorized {});
            }
            if let Some(a) = admin {
                cfg.admin = deps.api.addr_validate(&a)?;
            }
            if let Some(h) = hash_market_capability {
                cfg.hash_market_capability = Some(check_capability(h)?);
            }
            if let Some(m) = max_report_age_blocks {
                cfg.max_report_age_blocks = m;
            }
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "update_config"))
        }
    }
}

pub fn consensus_tip(deps: Deps, env: &Env, client_id: &str) -> StdResult<ConsensusTipResponse> {
    let cfg = CONFIG.load(deps.storage)?;
    let c = CLIENTS.load(deps.storage, client_id)?;
    // (height, hash) → (attestors, finalized heights)
    let mut groups: BTreeMap<(u64, String), (Vec<Addr>, Vec<u64>)> = BTreeMap::new();
    let mut fresh = 0u32;
    for a in &c.attestors {
        if !OPERATORS.has(deps.storage, a) {
            continue;
        }
        let Some(r) = REPORTS.may_load(deps.storage, (client_id, a))? else {
            continue;
        };
        if env.block.height.saturating_sub(r.reported_height) > cfg.max_report_age_blocks {
            continue;
        }
        fresh += 1;
        let g = groups
            .entry((r.state.tip_height, r.state.tip_hash.to_ascii_lowercase()))
            .or_default();
        g.0.push(a.clone());
        if let Some(f) = r.state.finalized_height {
            g.1.push(f);
        }
    }
    let tip = groups
        .into_iter()
        .rev()
        .find(|(_, (who, _))| who.len() as u32 >= c.min_attestations)
        .map(|((height, hash), (attestors, mut fins))| {
            // Conservative: the k-th highest finalized height among agreeing attestors.
            fins.sort_unstable_by(|a, b| b.cmp(a));
            let k = c.min_attestations as usize;
            let finalized_height = if fins.len() >= k {
                Some(fins[k - 1])
            } else {
                None
            };
            AgreedTip {
                height,
                hash,
                finalized_height,
                attestors,
            }
        });
    let quorum = cfg.hash_market_capability.map(|cap| {
        let root = query_root(deps, &cap);
        let v = match &tip {
            Some(t) => verdict(root.as_ref(), t.height, &t.hash),
            None if root.is_some() => QuorumVerdict::QuorumAhead,
            None => QuorumVerdict::Unavailable,
        };
        QuorumCheck {
            capability: cap,
            root,
            verdict: v,
        }
    });
    let tip = if quorum.as_ref().is_some_and(QuorumCheck::blocks_tip) {
        None
    } else {
        tip
    };
    Ok(ConsensusTipResponse {
        client_id: client_id.to_string(),
        min_attestations: c.min_attestations,
        fresh_reports: fresh,
        tip,
        quorum,
    })
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Operator { operator } => {
            let a = deps.api.addr_validate(&operator)?;
            to_json_binary(&OPERATORS.load(deps.storage, &a)?)
        }
        QueryMsg::Operators { start_after, limit } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = match start_after {
                Some(s) => Some(deps.api.addr_validate(&s)?),
                None => None,
            };
            let operators = OPERATORS
                .range(
                    deps.storage,
                    start.as_ref().map(Bound::exclusive),
                    None,
                    Order::Ascending,
                )
                .take(limit)
                .map(|r| r.map(|(_, v)| v))
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&OperatorsResponse { operators })
        }
        QueryMsg::Client { client_id } => to_json_binary(&CLIENTS.load(deps.storage, &client_id)?),
        QueryMsg::Clients { start_after, limit } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let clients = CLIENTS
                .range(
                    deps.storage,
                    start_after.as_deref().map(Bound::exclusive),
                    None,
                    Order::Ascending,
                )
                .take(limit)
                .map(|r| r.map(|(_, v)| v))
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&ClientsResponse { clients })
        }
        QueryMsg::NodeState {
            client_id,
            operator,
        } => {
            let a = deps.api.addr_validate(&operator)?;
            to_json_binary(&REPORTS.may_load(deps.storage, (&client_id, &a))?)
        }
        QueryMsg::NodeStates { client_id } => {
            let reports = REPORTS
                .prefix(&client_id)
                .range(deps.storage, None, None, Order::Ascending)
                .take(MAX_LIMIT as usize)
                .map(|r| r.map(|(_, v)| v))
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&NodeStatesResponse { reports })
        }
        QueryMsg::ConsensusTip { client_id } => {
            to_json_binary(&consensus_tip(deps, &env, &client_id)?)
        }
        QueryMsg::QuorumRoot {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(
                &cfg.hash_market_capability
                    .and_then(|cap| query_root(deps, &cap)),
            )
        }
        QueryMsg::ReportDigest {
            client_id,
            operator,
            state,
        } => {
            let a = deps.api.addr_validate(&operator)?;
            let d = report_digest(
                deps.api,
                &env.block.chain_id,
                &env.contract.address,
                &client_id,
                &a,
                &state,
            );
            to_json_binary(&Binary::from(d.to_vec()))
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
