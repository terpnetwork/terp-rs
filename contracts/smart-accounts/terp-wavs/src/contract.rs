use terp_account::traits::default::TerpAccountTrait;

use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};
use cw2::set_contract_version;

use crate::{
    msg::{ExecuteMsg, InstantiateMsg, QueryMsg},
    state::{BlsMetadata, WAVS_PUBKEY},
    TerpAccountWavs, ContractError,
};

use cosmwasm_std::entry_point;

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:terp-wavs";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Can only be called by governance
/// TODO: implement interface into bs-accounts
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    if msg.wavs_operator_pubkeys.len() > 10 {
        return Err(ContractError::TooManyWavsKeys {});
    }

    cw_ownable::initialize_owner(
        deps.storage,
        deps.api,
        Some(msg.owner.unwrap_or(info.sender).as_str()),
    )?;

    WAVS_PUBKEY.save(
        deps.storage,
        &BlsMetadata {
            operator_keys: msg.wavs_operator_pubkeys,
            threshold: msg.threshold,
        },
    )?;
    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(_deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {}
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {}
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(
    deps: DepsMut,
    env: Env,
    msg: <TerpAccountWavs as TerpAccountTrait>::SudoMsg,
) -> Result<Response, ContractError> {
    TerpAccountWavs::process_sudo_auth(deps, env, &msg)
}

// TODO: implement entrypoint as required type into btsgaccuont trait
pub fn execute_update_owner(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    action: cw_ownable::Action,
) -> Result<Response, ContractError> {
    let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
    Ok(Response::default().add_attributes(ownership.into_attributes()))
}
