//! Scaffold authenticator. Real Poseidon membership proofs are intentionally deferred.
//! Future: plug circuit verifying keys into `on_auth_request`.
//!
//! Note: package directory keeps the historical `terp-zkposiedon` spelling.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult};
use cw2::set_contract_version;
use terp_auth::AuthSudoMsg;

const CONTRACT_NAME: &str = "crates.io:terp-zkposiedon";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cw_serde]
pub struct InstantiateMsg {}

#[cw_serde]
pub enum ExecuteMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {}

pub type SudoMsg = AuthSudoMsg;

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: InstantiateMsg,
) -> Result<Response, StdError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "zkposeidon_instantiate"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: ExecuteMsg,
) -> Result<Response, StdError> {
    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(_deps: Deps, _env: Env, _msg: QueryMsg) -> StdResult<Binary> {
    Err(StdError::msg("no queries"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(_deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, StdError> {
    let action = match msg {
        AuthSudoMsg::OnAuthAdded(_) => "zkposeidon_on_auth_added",
        AuthSudoMsg::OnAuthRemoved(_) => "zkposeidon_on_auth_removed",
        AuthSudoMsg::Authenticate(_) => "zkposeidon_authenticate",
        AuthSudoMsg::Track(_) => "zkposeidon_track",
        AuthSudoMsg::ConfirmExecution(_) => "zkposeidon_confirm",
    };
    Ok(Response::new().add_attribute("action", action))
}
