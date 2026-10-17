use av_event_helpers::get_license_addr;
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, BankMsg, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Order, Reply,
    Response, StdResult, SubMsg, WasmMsg,
};
use cosmwasm_std::{Addr, Coin};

use cw2::set_contract_version;
use cw_ave::msg::InstantiateMsg as AvEventInstantiateMsg;
use cw_storage_plus::Bound;
use cw_utils::parse_instantiate_response_data;

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::state::{avevent_contracts, AvEventContract, AVEVENT_CODE_ID, TMP_INSTANTIATOR_INFO};

pub(crate) const CONTRACT_NAME: &str = "crates.io:cw-ave-factory";
pub(crate) const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INSTANTIATE_CONTRACT_REPLY_ID: u64 = 0;
pub const DEFAULT_LIMIT: u32 = 10;
pub const MAX_LIMIT: u32 = 50;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw_ownable::initialize_owner(deps.storage, deps.api, msg.owner.as_deref())?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    AVEVENT_CODE_ID.save(deps.storage, &msg.cw_ave_id)?;

    // HARDCODED LISENSE FEE
    let license_fee = av_event_helpers::get_license_fee(&env.block.chain_id)?;
    if info.funds.iter().any(|e| e == &license_fee) {
        let base_fee = CosmosMsg::Bank(BankMsg::Send {
            to_address: get_license_addr(&env.block.chain_id)?.to_string(),
            amount: vec![license_fee],
        });
        Ok(Response::new()
            .add_attribute("method", "instantiate")
            .add_attribute("creator", info.sender)
            .add_message(base_fee))
    } else {
        Err(ContractError::LicenseFeeRequired { fee: license_fee })
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateNativeAvEventContract {
            instantiate_msg,
            label,
        } => execute_instantiate_native_ave_contract(deps, info, instantiate_msg, label),
        ExecuteMsg::UpdateOwnership(action) => execute_update_owner(deps, info, env, action),
        ExecuteMsg::UpdateCodeId { cw_ave_code_id } => {
            execute_update_code_id(deps, info, cw_ave_code_id)
        }
    }
}

pub fn execute_instantiate_native_ave_contract(
    deps: DepsMut,
    info: MessageInfo,
    instantiate_msg: AvEventInstantiateMsg,
    label: String,
) -> Result<Response, ContractError> {
    // Save instantiator info for use in reply
    TMP_INSTANTIATOR_INFO.save(deps.storage, &info.sender)?;

    instantiate_contract(deps, info.sender, Some(info.funds), instantiate_msg, label)
}

/// `sender` here refers to the initiator of the shistrap, not the
/// literal sender of the message. Practically speaking, this means
/// that it should be set to the sender of the cw20's being vested,
/// and not the cw20 contract when dealing with non-native shistrap.
pub fn instantiate_contract(
    deps: DepsMut,
    sender: Addr,
    funds: Option<Vec<Coin>>,
    instantiate_msg: AvEventInstantiateMsg,
    label: String,
) -> Result<Response, ContractError> {
    // Check sender is contract owner if set
    let ownership = cw_ownable::get_ownership(deps.storage)?;
    if ownership
        .owner
        .as_ref()
        .is_some_and(|owner| *owner != sender)
    {
        return Err(ContractError::Unauthorized {});
    }

    let code_id = AVEVENT_CODE_ID.load(deps.storage)?;

    // Instantiate the specified contract with owner as the admin.
    let instantiate = WasmMsg::Instantiate {
        admin: Some(instantiate_msg.event_curator.clone()),
        code_id,
        msg: to_json_binary(&instantiate_msg)?,
        funds: funds.unwrap_or_default(),
        label,
    };

    let msg = SubMsg::reply_on_success(instantiate, INSTANTIATE_CONTRACT_REPLY_ID);

    Ok(Response::default()
        .add_attribute("action", "instantiate_cw_shistrap")
        .add_submessage(msg))
}

pub fn execute_update_owner(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    action: cw_ownable::Action,
) -> Result<Response, ContractError> {
    let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
    Ok(Response::default().add_attributes(ownership.into_attributes()))
}

pub fn execute_update_code_id(
    deps: DepsMut,
    info: MessageInfo,
    shistrap_code_id: u64,
) -> Result<Response, ContractError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    AVEVENT_CODE_ID.save(deps.storage, &shistrap_code_id)?;
    Ok(Response::default()
        .add_attribute("action", "update_code_id")
        .add_attribute("shistrap_code_id", shistrap_code_id.to_string()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::ListAvEventContracts { start_after, limit } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_after.as_deref().map(Bound::exclusive);

            let res: Vec<AvEventContract> = avevent_contracts()
                .range(deps.storage, start, None, Order::Ascending)
                .take(limit)
                .flat_map(|vc| Ok::<AvEventContract, ContractError>(vc?.1))
                .collect();

            Ok(to_json_binary(&res)?)
        }
        QueryMsg::ListAvEventContractsReverse {
            start_before,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_before.as_deref().map(Bound::exclusive);

            let res: Vec<AvEventContract> = avevent_contracts()
                .range(deps.storage, None, start, Order::Descending)
                .take(limit)
                .flat_map(|vc| Ok::<AvEventContract, ContractError>(vc?.1))
                .collect();

            Ok(to_json_binary(&res)?)
        }
        QueryMsg::ListAvEventContractsByInstantiator {
            instantiator,
            start_after,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_after.map(Bound::<String>::exclusive);

            // Validate owner address
            deps.api.addr_validate(&instantiator)?;

            let res: Vec<AvEventContract> = avevent_contracts()
                .idx
                .instantiator
                .prefix(instantiator)
                .range(deps.storage, start, None, Order::Ascending)
                .take(limit)
                .flat_map(|vc| Ok::<AvEventContract, ContractError>(vc?.1))
                .collect();

            Ok(to_json_binary(&res)?)
        }
        QueryMsg::ListAvEventContractsByInstantiatorReverse {
            instantiator,
            start_before,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_before.map(Bound::<String>::exclusive);

            // Validate owner address
            deps.api.addr_validate(&instantiator)?;

            let res: Vec<AvEventContract> = avevent_contracts()
                .idx
                .instantiator
                .prefix(instantiator)
                .range(deps.storage, None, start, Order::Descending)
                .take(limit)
                .flat_map(|vc| Ok::<AvEventContract, ContractError>(vc?.1))
                .collect();

            Ok(to_json_binary(&res)?)
        }
        // QueryMsg::ListAvEventContractsByToken {
        //     recipient,
        //     start_after,
        //     limit,
        // } => {
        //     let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
        //     let start = start_after.map(Bound::<String>::exclusive);

        //     // Validate recipient address
        //     deps.api.addr_validate(&recipient)?;

        //     let res: Vec<AvEventContract> = avevent_contracts()
        //         .idx
        //         .shit
        //         .prefix(recipient)
        //         .range(deps.storage, start, None, Order::Ascending)
        //         .take(limit)
        //         .flat_map(|vc| Ok::<AvEventContract, ContractError>(vc?.1))
        //         .collect();

        //     Ok(to_json_binary(&res)?)
        // }
        // QueryMsg::ListAvEventContractsByTokenReverse {
        //     recipient,
        //     start_before,
        //     limit,
        // } => {
        //     let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
        //     let start = start_before.map(Bound::<String>::exclusive);

        //     // Validate recipient address
        //     deps.api.addr_validate(&recipient)?;

        //     let res: Vec<AvEventContract> = avevent_contracts()
        //         .idx
        //         .prefix(recipient)
        //         .range(deps.storage, None, start, Order::Descending)
        //         .take(limit)
        //         .flat_map(|vc| Ok::<AvEventContract, ContractError>(vc?.1))
        //         .collect();

        //     Ok(to_json_binary(&res)?)
        // }
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::CodeId {} => to_json_binary(&AVEVENT_CODE_ID.load(deps.storage)?),
    }
}

fn contract_addr_from_instantiate_reply(deps: Deps, msg: Reply) -> Result<Addr, ContractError> {
    let resp = msg
        .result
        .into_result()
        .map_err(|e| ContractError::Std(cosmwasm_std::StdError::msg(e)))?;
    for r in &resp.msg_responses {
        if r.type_url.contains("MsgInstantiateContractResponse") {
            if let Ok(parsed) = parse_instantiate_response_data(&r.value) {
                return Ok(deps.api.addr_validate(&parsed.contract_address)?);
            }
        }
    }
    #[allow(deprecated)]
    if let Some(data) = resp.data.as_ref() {
        if let Ok(parsed) = parse_instantiate_response_data(data) {
            return Ok(deps.api.addr_validate(&parsed.contract_address)?);
        }
    }
    for ev in &resp.events {
        for attr in &ev.attributes {
            if attr.key == "_contract_address" || attr.key == "contract_address" {
                return Ok(deps.api.addr_validate(&attr.value)?);
            }
        }
    }
    Err(ContractError::Std(cosmwasm_std::StdError::msg(
        "instantiate reply missing contract address",
    )))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    match msg.id {
        INSTANTIATE_CONTRACT_REPLY_ID => {
            let contract_addr = contract_addr_from_instantiate_reply(deps.as_ref(), msg)?;

            let instantiator = TMP_INSTANTIATOR_INFO.load(deps.storage)?;

            // Save shistrap contract payment info
            avevent_contracts().save(
                deps.storage,
                contract_addr.as_ref(),
                &AvEventContract {
                    instantiator: instantiator.to_string(),
                    contract: contract_addr.to_string(),
                },
            )?;

            // Clear tmp instatiator info
            TMP_INSTANTIATOR_INFO.remove(deps.storage);

            Ok(Response::default().add_attribute("new_ave_contract", contract_addr))
        }
        _ => Err(ContractError::UnknownReplyId { id: msg.id }),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::new())
}
