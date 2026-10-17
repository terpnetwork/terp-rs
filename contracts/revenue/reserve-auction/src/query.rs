use cosmwasm_std::{Deps, Env, StdResult};

use crate::msg::QueryMsg;
use crate::state::{auctions, CONFIG, HALT_MANAGER};

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<cosmwasm_std::Binary> {
    match msg {
        QueryMsg::Config {} => cosmwasm_std::to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::HaltManager {} => cosmwasm_std::to_json_binary(&HALT_MANAGER.load(deps.storage)?),
        QueryMsg::Auction {
            collection,
            token_id,
        } => {
            let collection = deps.api.addr_validate(&collection)?;
            cosmwasm_std::to_json_binary(&auctions().may_load(
                deps.storage,
                (collection, token_id),
            )?)
        }
    }
}
