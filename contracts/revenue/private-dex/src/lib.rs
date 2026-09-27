//! # cw-private-dex
//!
//! Private swap **settle** contract. Host seam math mirrors pure
//! `terp_seams::dex`; contract wiring mirrors `crates/dex` (astroport-core
//! fork) pair swap / simulation / pause patterns, extended with
//! `proof_instance_verify` for private note spends.
//!
//! See `README.md` for dual-path verify and honest scope.

pub mod contract;
pub mod error;
pub mod instances;
pub mod msg;
pub mod seams;
pub mod state;
pub mod verify;

#[cfg(feature = "interface")]
pub mod interface;

pub use crate::error::ContractError;
pub use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    contract::instantiate(deps, env, info, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    contract::execute(deps, env, info, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    contract::query(deps, env, msg)
}

#[cfg(test)]
mod tests;
