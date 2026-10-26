use cosmwasm_std::{Binary, Deps, Env, StdResult, to_json_binary};

use crate::msg::{AltPotResponse, QueryMsg, RaffleResponse, TicketCountResponse, TicketResponse};
use crate::state::{
    ALT_POT, ALT_TICKETS, CONFIG, ORACLE_PRICING, TICKETS, USER_TICKETS, load_raffle, raffle_state,
};

pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let msg = match msg.into_oracle_query() {
        Ok(oracle) => {
            let pricing = ORACLE_PRICING.may_load(deps.storage)?;
            return terp_oracle::handle_oracle_query(deps, &env, pricing.as_ref(), oracle);
        }
        Err(msg) => msg,
    };
    match msg {
        QueryMsg::AltPot { raffle_id } => to_json_binary(&AltPotResponse {
            alt_tickets: ALT_TICKETS.may_load(deps.storage, raffle_id)?.unwrap_or(0),
            pot: ALT_POT.may_load(deps.storage, raffle_id)?.unwrap_or_default(),
        }),
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Raffle { raffle_id } => {
            let info = load_raffle(deps.storage, raffle_id)
                .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
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
        _ => Err(cosmwasm_std::StdError::msg("unknown query")),
    }
}
