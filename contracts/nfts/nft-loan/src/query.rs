use cosmwasm_std::{to_json_binary, Binary, Deps, Env, StdResult};

use crate::msg::{CollateralResponse, OfferResponse, QueryMsg};
use crate::state::{load_collateral, load_offer, CONFIG};

pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Collateral { borrower, loan_id } => {
            let borrower = deps.api.addr_validate(&borrower)?;
            let collateral = load_collateral(deps.storage, &borrower, loan_id)
                .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
            to_json_binary(&CollateralResponse {
                borrower: borrower.into_string(),
                loan_id,
                state: collateral.state.clone(),
                collateral,
            })
        }
        QueryMsg::Offer { global_offer_id } => {
            let id: u64 = global_offer_id
                .parse()
                .map_err(|_| cosmwasm_std::StdError::msg("offer not found"))?;
            let offer = load_offer(deps.storage, id)
                .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
            to_json_binary(&OfferResponse {
                global_offer_id,
                offer,
            })
        }
    }
}
