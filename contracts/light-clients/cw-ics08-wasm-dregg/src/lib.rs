#![doc = include_str!("../README.md")]

pub mod contract;
pub mod error;
pub mod msg;
pub mod settle;
pub mod state;
pub mod types;

pub use error::ContractError;
pub use types::{ClientState, ConsensusState, SettlementUpdate};

#[cfg(not(feature = "library"))]
pub use crate::entry_points::*;

#[cfg(not(feature = "library"))]
mod entry_points {
    use cosmwasm_std::{
        entry_point, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response, StdResult,
    };

    use crate::error::ContractError;
    use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};

    #[entry_point]
    pub fn instantiate(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: InstantiateMsg,
    ) -> Result<Response, ContractError> {
        crate::contract::instantiate(deps, env, info, msg)
    }

    #[entry_point]
    pub fn execute(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: ExecuteMsg,
    ) -> Result<Response, ContractError> {
        crate::contract::execute(deps, env, info, msg)
    }

    #[entry_point]
    pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
        crate::contract::sudo(deps, env, msg)
    }

    #[entry_point]
    pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
        crate::contract::query(deps, env, msg)
    }

    #[entry_point]
    pub fn reply(_deps: DepsMut, _env: Env, _msg: Reply) -> Result<Response, ContractError> {
        Ok(Response::default())
    }
}
