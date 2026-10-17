use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use cw2::set_contract_version;

use crate::msg::InstantiateMsg;
use crate::state::{Config, NONCE};
use crate::{ContractError, CONTRACT_NAME, CONTRACT_VERSION};

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let config = Config {
        fee_manager: deps.api.addr_validate(&msg.fee_manager)?,
        royalty_registry: msg
            .royalty_registry
            .as_ref()
            .map(|addr| deps.api.addr_validate(addr))
            .transpose()?,
        protocol_fee_bps: msg.protocol_fee_bps,
        max_royalty_fee_bps: msg.max_royalty_fee_bps,
        maker_reward_bps: msg.maker_reward_bps,
        taker_reward_bps: msg.taker_reward_bps,
        burn_bps: msg.burn_bps,
        default_denom: msg.default_denom,
    };
    config.save(deps.storage)?;
    NONCE.save(deps.storage, &0)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("contract_name", CONTRACT_NAME)
        .add_attribute("contract_version", CONTRACT_VERSION))
}
