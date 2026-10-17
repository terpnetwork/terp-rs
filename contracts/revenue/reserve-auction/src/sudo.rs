use cosmwasm_std::{Addr, DepsMut, Env, Order, Response, StdResult};
use cw_storage_plus::Bound;

use crate::helpers::settle_auction;
use crate::msg::SudoMsg;
use crate::state::{auctions, Auction, HaltWindow, CONFIG, HALT_MANAGER, MIN_RESERVE_PRICES};
use crate::ContractError;

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::BeginBlock {} => begin_block(deps, env),
        SudoMsg::EndBlock {} => end_block(deps, env),
        SudoMsg::SetMinReservePrices { min_reserve_prices } => {
            for coin in min_reserve_prices {
                MIN_RESERVE_PRICES.save(deps.storage, coin.denom, &coin.amount)?;
            }
            Ok(Response::new().add_attribute("action", "set-min-reserve-prices"))
        }
        SudoMsg::UnsetMinReservePrices { denoms } => {
            for denom in denoms {
                MIN_RESERVE_PRICES.remove(deps.storage, denom);
            }
            Ok(Response::new().add_attribute("action", "unset-min-reserve-prices"))
        }
    }
}

fn begin_block(deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut halt = HALT_MANAGER.load(deps.storage)?;
    let now = env.block.time.seconds();
    let gap = now.saturating_sub(halt.prev_block_time);
    let mut response = Response::new();
    if halt.prev_block_time > 0 && gap >= config.halt_duration_threshold {
        let window = HaltWindow {
            start_time: halt.prev_block_time,
            end_time: now + config.halt_buffer_duration,
        };
        response = response.add_event(
            cosmwasm_std::Event::new("halt-detected")
                .add_attribute("start_time", window.start_time.to_string())
                .add_attribute("end_time", window.end_time.to_string()),
        );
        halt.halt_windows.push(window);
    }
    halt.prev_block_time = now;
    HALT_MANAGER.save(deps.storage, &halt)?;
    Ok(response)
}

fn end_block(mut deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let halt = HALT_MANAGER.load(deps.storage)?;
    let limit = config.max_auctions_to_settle_per_block as usize;
    let max = Some(Bound::exclusive((
        env.block.time.seconds().saturating_add(1),
        (Addr::unchecked(""), String::new()),
    )));
    let due: Vec<Auction> = auctions()
        .idx
        .end_time
        .range(deps.storage, None, max, Order::Ascending)
        .take(limit)
        .collect::<StdResult<Vec<_>>>()?
        .into_iter()
        .map(|(_, auction)| auction)
        .collect();

    let mut response =
        Response::new().add_event(cosmwasm_std::Event::new("sudo-end-block").add_attribute("action", "settle"));
    for auction in due {
        response = settle_auction(
            deps.branch(),
            env.block.time,
            auction,
            &config,
            &halt,
            response,
        )?;
    }
    Ok(response)
}
