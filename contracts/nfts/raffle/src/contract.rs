use cosmwasm_std::{
    entry_point, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response, StdError, StdResult,
};
use cw2::set_contract_version;

use crate::error::ContractError;
use crate::execute::DRAW_REPLY_ID;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};

pub const CONTRACT_NAME: &str = "nftfi-raffle";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    crate::execute::instantiate(deps, info, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    crate::execute::execute(deps, env, info, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    crate::query::query(deps, env, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> Result<Response, ContractError> {
    if msg.id != DRAW_REPLY_ID {
        return Err(ContractError::Std(StdError::msg("unknown reply")));
    }
    let result = msg.result.into_result().map_err(StdError::msg)?;
    crate::execute::reply_draw(deps, env, &result.events)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    crate::execute::sudo(deps, msg)
}
