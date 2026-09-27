//! Primitive Path A egress: `proof_instance_verify(zkid, proof, instances)`.
//!
//! Used by `terp-orch` so Stwo / Flock (and later circuits) share
//! one contract surface. Circuit VKs / footers stay on the cw-orch circuit
//! interfaces, not in this crate.

pub mod error;
pub mod msg;

pub use error::ContractError;
pub use msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg};

use cosmwasm_std::{to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cw2::set_contract_version;
use cw_storage_plus::Item;

const CONTRACT_NAME: &str = "crates.io:cw-zk-verify-demo";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const ZKID: Item<u64> = Item::new("zkid");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    ZKID.save(deps.storage, &msg.zkid)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("zkid", msg.zkid.to_string()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Verify { proof, instances } => execute_verify(deps, proof, instances),
    }
}

fn execute_verify(
    deps: DepsMut,
    proof: Binary,
    instances: Binary,
) -> Result<Response, ContractError> {
    let zkid = ZKID.load(deps.storage)?;
    #[cfg(feature = "zk-host")]
    {
        match deps
            .api
            .proof_instance_verify(zkid, proof.as_slice(), instances.as_slice())
        {
            Ok(true) => Ok(Response::new()
                .add_attribute("action", "verify")
                .add_attribute("zkid", zkid.to_string())),
            Ok(false) => Err(ContractError::VerifyFailed {
                reason: "proof_instance_verify returned false".into(),
            }),
            Err(e) => Err(ContractError::VerifyFailed {
                reason: format!("proof_instance_verify: {e}"),
            }),
        }
    }
    #[cfg(not(feature = "zk-host"))]
    {
        let _ = (deps, proof, instances, zkid);
        Err(ContractError::ZkHostRequired)
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let zkid = ZKID.load(deps.storage)?;
            to_json_binary(&ConfigResponse { zkid })
        }
    }
}
