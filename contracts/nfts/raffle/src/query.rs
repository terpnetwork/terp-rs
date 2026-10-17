use cosmwasm_std::{to_json_binary, Binary, Deps, Env, StdResult};

use crate::msg::{QueryMsg, RaffleResponse, TicketCountResponse, TicketResponse};
use crate::state::{load_raffle, raffle_state, CONFIG, TICKETS, USER_TICKETS};

pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Raffle { raffle_id } => {
            let info = load_raffle(deps.storage, raffle_id).map_err(|e| {
                cosmwasm_std::StdError::msg(e.to_string())
            })?;
            to_json_binary(&RaffleResponse {
                raffle_id,
                state: raffle_state(&env, &info),
                info,
            })
        }
        QueryMsg::Ticket { raffle_id, index } => {
            let owner = TICKETS.load(deps.storage, (raffle_id, index))?;
            to_json_binary(&TicketResponse {
                owner: owner.into_string(),
            })
        }
        QueryMsg::TicketCount { owner, raffle_id } => {
            let owner = deps.api.addr_validate(&owner)?;
            let count = USER_TICKETS
                .may_load(deps.storage, (owner, raffle_id))?
                .unwrap_or(0);
            to_json_binary(&TicketCountResponse { count })
        }
    }
}
