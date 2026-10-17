//! This module contains the CosmWasm entrypoints for the 08-wasm smart contract.

use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, MigrateInfo, Response};
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::custom_query::CrosslinkCustomQuery;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, Migration, QueryMsg, SudoMsg};
use crate::{instantiate, query, sudo, ContractError};

/// The version of the contract's state.
const STATE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");

/// The instantiate entry point for the CosmWasm contract.
///
/// # Errors
///
/// Will return an error if the client state or consensus state cannot be deserialized.
#[cfg_attr(not(feature = "library"), entry_point)]
#[allow(clippy::needless_pass_by_value)]
pub fn instantiate(
    deps: DepsMut<CrosslinkCustomQuery>,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, STATE_VERSION)?;
    instantiate::client(deps.storage, msg)?;
    Ok(Response::default())
}

/// The sudo entry point for the CosmWasm contract.
///
/// Routes the message to the appropriate handler.
///
/// # Errors
///
/// Will return an error if the handler returns an error.
#[cfg_attr(not(feature = "library"), entry_point)]
#[allow(clippy::needless_pass_by_value)]
pub fn sudo(
    deps: DepsMut<CrosslinkCustomQuery>,
    _env: Env,
    msg: SudoMsg,
) -> Result<Response, ContractError> {
    let result = match msg {
        SudoMsg::VerifyMembership(verify_membership_msg) => {
            sudo::verify_membership(deps.as_ref(), verify_membership_msg)?
        }
        SudoMsg::VerifyNonMembership(verify_non_membership_msg) => {
            sudo::verify_non_membership(deps.as_ref(), verify_non_membership_msg)?
        }
        SudoMsg::UpdateState(update_state_msg) => sudo::update_state(deps, update_state_msg)?,
        SudoMsg::UpdateStateOnMisbehaviour(misbehaviour_msg) => {
            sudo::misbehaviour(deps, misbehaviour_msg)?
        }
        SudoMsg::VerifyUpgradeAndUpdateState(_) => {
            return Err(ContractError::InvalidClientMessage);
        }
        SudoMsg::MigrateClientStore(_) => {
            return Err(ContractError::InvalidClientMessage);
        }
    };

    Ok(Response::default().set_data(result))
}

/// Execute entry point is not used in this contract.
#[cfg_attr(not(feature = "library"), entry_point)]
#[allow(clippy::needless_pass_by_value, clippy::missing_errors_doc)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    Err(ContractError::InvalidClientMessage)
}

/// The query entry point for the CosmWasm contract.
///
/// Routes the message to the appropriate handler.
///
/// # Errors
///
/// Will return an error if the handler returns an error.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(
    deps: Deps<CrosslinkCustomQuery>,
    env: Env,
    msg: QueryMsg,
) -> Result<Binary, ContractError> {
    match msg {
        QueryMsg::VerifyClientMessage(verify_client_message_msg) => {
            query::verify_client_message(deps, env, verify_client_message_msg)
        }
        QueryMsg::CheckForMisbehaviour(check_for_misbehaviour_msg) => {
            query::check_for_misbehaviour(deps, env, check_for_misbehaviour_msg)
        }
        QueryMsg::TimestampAtHeight(timestamp_at_height_msg) => {
            query::timestamp_at_height(deps, timestamp_at_height_msg)
        }
        QueryMsg::Status(_) => query::status(deps),
    }
}

/// The migrate entry point for the CosmWasm contract.
///
/// # Errors
///
/// Will return an error if the state version is not newer than the current one.
#[cfg_attr(not(feature = "library"), entry_point)]
#[allow(clippy::needless_pass_by_value)]
pub fn migrate(
    deps: DepsMut<CrosslinkCustomQuery>,
    _env: Env,
    msg: MigrateMsg,
    _info: MigrateInfo,
) -> Result<Response, ContractError> {
    cw2::ensure_from_older_version(deps.storage, CONTRACT_NAME, STATE_VERSION)?;

    match msg.migration {
        Migration::CodeOnly => {}
        Migration::Reinstantiate(instantiate_msg) => {
            instantiate::client(deps.storage, instantiate_msg)?;
        }
    }

    Ok(Response::default())
}