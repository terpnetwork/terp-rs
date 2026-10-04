use abstract_std::objects::gov_type::GovernanceDetails;
use abstract_std::objects::ownership::{self, Ownership};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    entry_point, Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult,
};
use cw_storage_plus::Item;
use thiserror::Error;
#[cfg(not(target_arch = "wasm32"))]
pub mod interface;

pub const CONTRACT_NAME: &str = "crates.io:ownership-verifier";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Storage constant for the contract's ownership
const OWNERSHIP: Item<Ownership<String>> = Item::new("ownership");

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
}
#[cw_serde]
pub struct InstantiateMsg {
    pub ownership: GovernanceDetails<String>,
}

impl InstantiateMsg {
    pub fn new(contract: &String, token: &str) -> Self {
        Self {
            ownership: GovernanceDetails::NFT {
                collection_addr: contract.into(),
                token_id: token.into(),
            },
        }
    }
}

#[cw_serde]
#[derive(cw_orch::ExecuteFns)]
pub enum ExecuteMsg {
    UpdateOwnership {
        ownership: GovernanceDetails<String>,
    },
}

#[cw_serde]
#[derive(QueryResponses, cw_orch::QueryFns)]
pub enum QueryMsg {
    #[returns(Ownership<Addr>)]
    Ownership {},
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    OWNERSHIP.save(
        deps.storage,
        &Ownership {
            owner: msg.ownership,
            pending_owner: None,
            pending_expiry: None,
        },
    )?;
    Ok(Response::new().add_attribute("method", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateOwnership { ownership } => {
            OWNERSHIP.save(
                deps.storage,
                &Ownership {
                    owner: ownership,
                    pending_owner: None,
                    pending_expiry: None,
                },
            )?;
            Ok(Response::new())
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => {
            cosmwasm_std::to_json_binary(&ownership::get_ownership(deps.storage)?)
        }
    }
}
