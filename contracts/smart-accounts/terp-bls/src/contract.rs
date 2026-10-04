use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};
use cw2::set_contract_version;
use terp_account::traits::default::TerpAccountTrait;

use crate::error::ContractError;
use crate::msg::{BlsConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{BlsConfig, BLS_CONFIG};
use crate::verify::validate_key;
use crate::TerpAccountBls;

const CONTRACT_NAME: &str = "crates.io:terp-bls";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_KEYS: usize = 10;

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    if msg.operator_pubkeys.is_empty() {
        return Err(ContractError::InvalidConfig {
            reason: "at least one operator key".into(),
        });
    }
    if msg.operator_pubkeys.len() > MAX_KEYS {
        return Err(ContractError::TooManyKeys {});
    }
    if msg.threshold == 0 || msg.threshold as usize > msg.operator_pubkeys.len() {
        return Err(ContractError::InvalidConfig {
            reason: "threshold must be 1..=key count".into(),
        });
    }
    for key in &msg.operator_pubkeys {
        validate_key(deps.api, &msg.variant, key.as_slice())?;
    }
    let owner = msg.owner.unwrap_or_else(|| info.sender.to_string());
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(owner.as_str()))?;
    BLS_CONFIG.save(
        deps.storage,
        &BlsConfig {
            operator_keys: msg.operator_pubkeys,
            threshold: msg.threshold,
            variant: msg.variant.clone(),
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "bls_instantiate")
        .add_attribute("variant", msg.variant.as_str()))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let cfg = BLS_CONFIG.load(deps.storage)?;
            cosmwasm_std::to_json_binary(&BlsConfigResponse {
                variant: cfg.variant,
                threshold: cfg.threshold,
                operator_pubkeys: cfg.operator_keys,
            })
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateOwnership(action) => {
            let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
            Ok(Response::new().add_attributes(ownership.into_attributes()))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(
    deps: DepsMut,
    env: Env,
    msg: <TerpAccountBls as TerpAccountTrait>::SudoMsg,
) -> Result<Response, ContractError> {
    TerpAccountBls::process_sudo_auth(deps, env, &msg)
}
