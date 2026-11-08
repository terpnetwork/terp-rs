//! Voice-chain probe. Polytone `Query` hits this instead of a live pool.
//! Shape matches Osmosis `arithmetic_twap` JSON so the registry parser is shared.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, to_json_binary};
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

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
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

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
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

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
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

/// Migrate message. Intentionally empty: migrate is a plain capability entrypoint.
#[cw_serde]
pub struct MigrateMsg {}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(
    _deps: DepsMut,
    _env: Env,
    _msg: MigrateMsg,
    _info: cosmwasm_std::MigrateInfo,
) -> StdResult<Response> {
    Ok(Response::new().add_attribute("action", "migrate"))
}

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::OracleTwapProbe;

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::from_json;
    use cosmwasm_std::testing::{MockApi, message_info, mock_dependencies, mock_env};

    #[test]
    fn canned_twap() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&MockApi::default().addr_make("u"), &[]),
            InstantiateMsg { mid: "1.25".into() },
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

#[cfg(test)]
mod migrate_tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env};

    #[test]
    fn migrate_ok() {
        let mut deps = mock_dependencies();
        let admin = deps.api.addr_make("admin");
        let res = migrate(
            deps.as_mut(),
            mock_env(),
            MigrateMsg {},
            cosmwasm_std::MigrateInfo {
                sender: admin,
                old_migrate_version: None,
            },
        )
        .unwrap();
        assert_eq!(res.attributes[0].value, "migrate");
    }
}
