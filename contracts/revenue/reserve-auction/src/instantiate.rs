use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use cw2::set_contract_version;

use crate::msg::InstantiateMsg;
use crate::state::{Config, HaltManager, HALT_MANAGER, MIN_RESERVE_PRICES};
use crate::ContractError;
use crate::{CONTRACT_NAME, CONTRACT_VERSION};

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let config = Config {
        fee_manager: deps.api.addr_validate(&msg.fee_manager)?,
        trading_fee_percent: msg.trading_fee_percent,
        min_bid_increment_percent: msg.min_bid_increment_percent,
        min_duration: msg.min_duration,
        max_duration: msg.max_duration,
        extend_duration: msg.extend_duration,
        create_auction_fee: msg.create_auction_fee,
        max_auctions_to_settle_per_block: msg.max_auctions_to_settle_per_block,
        halt_duration_threshold: msg.halt_duration_threshold,
        halt_buffer_duration: msg.halt_buffer_duration,
        halt_postpone_duration: msg.halt_postpone_duration,
        burn_bps: msg.burn_bps,
    };
    config.save(deps.storage)?;
    for coin in msg.min_reserve_prices {
        MIN_RESERVE_PRICES.save(deps.storage, coin.denom, &coin.amount)?;
    }
    // Seed with the instantiate block so the first BeginBlock is not a false halt.
    HALT_MANAGER.save(
        deps.storage,
        &HaltManager {
            prev_block_time: env.block.time.seconds(),
            halt_windows: vec![],
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}
