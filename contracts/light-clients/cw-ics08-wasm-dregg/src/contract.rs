use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw2::set_contract_version;

use crate::error::ContractError;
use crate::msg::{
    BoolResponse, ExecuteMsg, HeightResponse, InstantiateMsg, QueryMsg, RootResponse, StatusResult,
    SudoMsg, VerifyClientMessageResult,
};
use crate::settle::apply_settlement;
use crate::state::{CLIENT_STATE, CONSENSUS_HEAD, PROVEN_ROOTS, WASM_CHECKSUM};
use crate::types::{pack_lanes_hex, parse_vk_hash, SettlementUpdate};

const CONTRACT_NAME: &str = "crates.io:cw-ics08-wasm-dregg";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    parse_vk_hash(&msg.client_state.settlement_vk_hash)
        .map_err(|_| ContractError::InvalidVkHash)?;

    if msg.consensus_state.proven_lanes != msg.client_state.genesis_lanes {
        return Err(ContractError::ContinuityBroken);
    }
    let root = pack_lanes_hex(&msg.client_state.genesis_lanes)
        .map_err(|_| ContractError::InvalidLane)?;
    CLIENT_STATE.save(deps.storage, &msg.client_state)?;
    CONSENSUS_HEAD.save(deps.storage, &msg.consensus_state)?;
    PROVEN_ROOTS.save(deps.storage, &root, &true)?;
    if let Some(cs) = msg.checksum {
        WASM_CHECKSUM.save(deps.storage, &cs)?;
    }

    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("claim_class", "settlement_only")
        .add_attribute("trust_level", "settlement_wrap_only")
        .add_attribute("genesis_root", root)
        .add_attribute("proven_height", msg.consensus_state.proven_height.to_string()))
}

pub fn execute(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Settle(update) => apply_update(deps, env, update),
    }
}

pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::UpdateState { client_message } => {
            let update: SettlementUpdate = cosmwasm_std::from_json(client_message)?;
            apply_update(deps, env, update)
        }
        SudoMsg::VerifyMembership { .. } | SudoMsg::VerifyNonMembership { .. } => {
            Err(ContractError::MembershipNotSupportedYet)
        }
        SudoMsg::UpdateStateOnMisbehaviour { .. } => Err(ContractError::MisbehaviourResidual),
    }
}

fn apply_update(
    deps: DepsMut,
    _env: Env,
    update: SettlementUpdate,
) -> Result<Response, ContractError> {
    let client = CLIENT_STATE.load(deps.storage)?;
    let head = CONSENSUS_HEAD.load(deps.storage)?;
    let old_root = head.proven_root_key.clone();
    let (new_client, new_head) = apply_settlement(&client, &head, &update)?;
    CLIENT_STATE.save(deps.storage, &new_client)?;
    CONSENSUS_HEAD.save(deps.storage, &new_head)?;
    PROVEN_ROOTS.save(deps.storage, &new_head.proven_root_key, &true)?;

    Ok(Response::new()
        .add_attribute("action", "settle")
        .add_attribute("claim_class", "settlement_only")
        .add_attribute("trust_level", "settlement_wrap_only")
        .add_attribute("old_root", old_root)
        .add_attribute("new_root", new_head.proven_root_key.clone())
        .add_attribute("proven_height", new_head.proven_height.to_string())
        .add_attribute("num_turns", update.num_turns.to_string()))
}

pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::ClientState {} => to_json_binary(&CLIENT_STATE.load(deps.storage)?),
        QueryMsg::ConsensusState {} => to_json_binary(&CONSENSUS_HEAD.load(deps.storage)?),
        QueryMsg::ProvenHeight {} => {
            let h = CONSENSUS_HEAD.load(deps.storage)?;
            to_json_binary(&HeightResponse {
                height: h.proven_height,
            })
        }
        QueryMsg::ProvenRoot {} => {
            let h = CONSENSUS_HEAD.load(deps.storage)?;
            to_json_binary(&RootResponse {
                root: h.proven_root_key,
            })
        }
        QueryMsg::IsProvenRoot { root } => {
            let yes = PROVEN_ROOTS.may_load(deps.storage, &root)?.unwrap_or(false);
            to_json_binary(&BoolResponse { yes })
        }
        QueryMsg::Status {} => {
            let c = CLIENT_STATE.load(deps.storage)?;
            let status = if c.frozen_height.is_some() {
                "Frozen"
            } else {
                "Active"
            };
            to_json_binary(&StatusResult {
                status: status.into(),
                claim_class: "settlement_only".into(),
            })
        }
        QueryMsg::VerifyClientMessage { client_message } => {
            let client = CLIENT_STATE.load(deps.storage)?;
            let head = CONSENSUS_HEAD.load(deps.storage)?;
            let result = match cosmwasm_std::from_json::<SettlementUpdate>(&client_message) {
                Ok(update) => match apply_settlement(&client, &head, &update) {
                    Ok(_) => VerifyClientMessageResult {
                        ok: true,
                        reason: None,
                    },
                    Err(e) => VerifyClientMessageResult {
                        ok: false,
                        reason: Some(e.to_string()),
                    },
                },
                Err(e) => VerifyClientMessageResult {
                    ok: false,
                    reason: Some(e.to_string()),
                },
            };
            to_json_binary(&result)
        }
    }
}
