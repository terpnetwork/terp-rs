//! Voice-chain probe. Polytone `Query` hits this instead of a live pool.
//! Shape matches Osmosis `arithmetic_twap` JSON so the registry parser is shared.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw_storage_plus::Item;

const CONTRACT_NAME: &str = "crates.io:cw-oracle-twap-probe";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cw_serde]
pub struct InstantiateMsg {
    pub mid: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    SetMid { mid: String },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(TwapResponse)]
    ArithmeticTwap { pool_id: u64 },
}

#[cw_serde]
pub struct TwapResponse {
    pub arithmetic_twap: String,
    pub pool_id: u64,
}

pub const MID: Item<String> = Item::new("mid");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    MID.save(deps.storage, &msg.mid)?;
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "instantiate_twap_probe"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> StdResult<Response> {
    match msg {
        ExecuteMsg::SetMid { mid } => {
            MID.save(deps.storage, &mid)?;
            Ok(Response::new().add_attribute("action", "set_mid"))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::ArithmeticTwap { pool_id } => {
            let mid = MID.load(deps.storage)?;
            to_json_binary(&TwapResponse {
                arithmetic_twap: mid,
                pool_id,
            })
        }
    }
}

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::OracleTwapProbe;

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi};
    use cosmwasm_std::from_json;

    #[test]
    fn canned_twap() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&MockApi::default().addr_make("u"), &[]),
            InstantiateMsg {
                mid: "1.25".into(),
            },
        )
        .unwrap();
        let bin = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::ArithmeticTwap { pool_id: 1 },
        )
        .unwrap();
        let r: TwapResponse = from_json(bin).unwrap();
        assert_eq!(r.arithmetic_twap, "1.25");
        assert_eq!(r.pool_id, 1);
    }
}
