//! Fractal price view: query one registry, compute your own window / method.
//! Instantiate as many times as you want. The registry stays the SSOT.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult,
};
use cw_oracle_registry::msg::{HistoryResponse, QueryMsg as RegistryQuery};
use cw_storage_plus::Item;

const CONTRACT_NAME: &str = "crates.io:cw-oracle-aggregator";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cw_serde]
pub struct InstantiateMsg {
    pub registry: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    SetRegistry { registry: String },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(cosmwasm_std::Addr)]
    Registry {},
    /// Median of the newest `window` observations for `market_id`.
    #[returns(ViewResponse)]
    Median { market_id: String, window: u32 },
}

#[cw_serde]
pub struct ViewResponse {
    pub market_id: String,
    pub method: String,
    pub n: u32,
    pub value: Decimal,
}

pub const REGISTRY: Item<cosmwasm_std::Addr> = Item::new("reg");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let registry = deps.api.addr_validate(&msg.registry)?;
    REGISTRY.save(deps.storage, &registry)?;
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "instantiate_oracle_aggregator"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> StdResult<Response> {
    match msg {
        ExecuteMsg::SetRegistry { registry } => {
            let registry = deps.api.addr_validate(&registry)?;
            REGISTRY.save(deps.storage, &registry)?;
            Ok(Response::new().add_attribute("action", "set_registry"))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Registry {} => to_json_binary(&REGISTRY.load(deps.storage)?),
        QueryMsg::Median { market_id, window } => {
            let registry = REGISTRY.load(deps.storage)?;
            let limit = window.max(1).min(128);
            let hist: HistoryResponse = deps.querier.query_wasm_smart(
                registry.to_string(),
                &RegistryQuery::History {
                    market_id: market_id.clone(),
                    limit: Some(limit),
                },
            )?;
            if hist.observations.is_empty() {
                return Err(StdError::msg("no observations"));
            }
            let mut mids: Vec<Decimal> = hist.observations.iter().map(|o| o.mid).collect();
            mids.sort();
            let n = mids.len();
            let value = if n % 2 == 1 {
                mids[n / 2]
            } else {
                (mids[n / 2 - 1] + mids[n / 2]) / Decimal::from_ratio(2u128, 1u128)
            };
            to_json_binary(&ViewResponse {
                market_id,
                method: "median".into(),
                n: n as u32,
                value,
            })
        }
    }
}

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::OracleAggregator;

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi};
    use cosmwasm_std::{from_json, ContractResult, SystemResult, WasmQuery};
    use cw_oracle_registry::msg::ObservationResponse;
    use cw_oracle_registry::state::SourceKind;

    #[test]
    fn median_of_mocked_history() {
        let registry = MockApi::default().addr_make("registry");
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&MockApi::default().addr_make("u"), &[]),
            InstantiateMsg {
                registry: registry.to_string(),
            },
        )
        .unwrap();

        deps.querier.update_wasm(move |q| match q {
            WasmQuery::Smart { .. } => {
                let hist = HistoryResponse {
                    observations: vec![
                        ObservationResponse {
                            market_id: "OSMO/USD".into(),
                            source: SourceKind::Relayer,
                            source_chain: "osmosis-test-1".into(),
                            pool_id: None,
                            height: 3,
                            mid: Decimal::percent(300),
                            recorder: registry.clone(),
                        },
                        ObservationResponse {
                            market_id: "OSMO/USD".into(),
                            source: SourceKind::Relayer,
                            source_chain: "osmosis-test-1".into(),
                            pool_id: None,
                            height: 2,
                            mid: Decimal::percent(100),
                            recorder: registry.clone(),
                        },
                        ObservationResponse {
                            market_id: "OSMO/USD".into(),
                            source: SourceKind::Relayer,
                            source_chain: "osmosis-test-1".into(),
                            pool_id: None,
                            height: 1,
                            mid: Decimal::percent(200),
                            recorder: registry.clone(),
                        },
                    ],
                };
                SystemResult::Ok(ContractResult::Ok(to_json_binary(&hist).unwrap()))
            }
            _ => SystemResult::Err(cosmwasm_std::SystemError::Unknown {}),
        });

        let bin = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Median {
                market_id: "OSMO/USD".into(),
                window: 3,
            },
        )
        .unwrap();
        let v: ViewResponse = from_json(bin).unwrap();
        assert_eq!(v.n, 3);
        assert_eq!(v.value, Decimal::percent(200));
    }
}
