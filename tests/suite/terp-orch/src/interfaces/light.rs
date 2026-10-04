//! cw-orch interfaces for the 08-wasm light clients.
//!
//! Crosslink's own `interface` feature wires entry points that take a custom
//! query. This suite calls `instantiate::client` (storage only) so the Mock
//! path runs the same genesis checks as the guest, and uploads
//! `cw_ics08_wasm_crosslink.wasm` on a live chain.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult};
use cw_ics08_wasm_crosslink::msg::{
    ExecuteMsg as CrosslinkExecute, InstantiateMsg as CrosslinkInstantiate, QueryMsg as CrosslinkQuery,
};
use cw_orch::{interface, prelude::*};

use super::wasm_path::wasm;

fn crosslink_instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: CrosslinkInstantiate,
) -> Result<Response, cw_ics08_wasm_crosslink::ContractError> {
    cw_ics08_wasm_crosslink::instantiate::client(deps.storage, msg)?;
    Ok(Response::new())
}

fn crosslink_execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: CrosslinkExecute,
) -> Result<Response, cw_ics08_wasm_crosslink::ContractError> {
    match msg {}
}

fn crosslink_query(
    _deps: Deps,
    _env: Env,
    _msg: CrosslinkQuery,
) -> Result<Binary, cw_ics08_wasm_crosslink::ContractError> {
    Err(cw_ics08_wasm_crosslink::ContractError::InvalidClientMessage)
}

#[interface(
    CrosslinkInstantiate,
    CrosslinkExecute,
    CrosslinkQuery,
    Empty,
    id = "cw-ics08-wasm-crosslink"
)]
pub struct CwIcs08WasmCrosslink;

impl<Chain: TxHandler> Uploadable for CwIcs08WasmCrosslink<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("cw_ics08_wasm_crosslink")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            crosslink_execute,
            crosslink_instantiate,
            crosslink_query,
        ))
    }
}

#[interface(
    cw_ics08_wasm_dregg::msg::InstantiateMsg,
    cw_ics08_wasm_dregg::msg::ExecuteMsg,
    cw_ics08_wasm_dregg::msg::QueryMsg,
    Empty,
    id = "cw-ics08-wasm-dregg"
)]
pub struct CwIcs08WasmDregg;

impl<Chain: TxHandler> Uploadable for CwIcs08WasmDregg<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("cw_ics08_wasm_dregg")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            cw_ics08_wasm_dregg::contract::execute,
            cw_ics08_wasm_dregg::contract::instantiate,
            cw_ics08_wasm_dregg::contract::query,
        ))
    }
}

#[interface(
    cw_ics08_wasm_terp::msg::InstantiateMsg,
    cw_ics08_wasm_terp::msg::ExecuteMsg,
    cw_ics08_wasm_terp::msg::QueryMsg,
    Empty,
    id = "cw-ics08-wasm-terp"
)]
pub struct CwIcs08WasmTerp;

impl<Chain: TxHandler> Uploadable for CwIcs08WasmTerp<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("cw_ics08_wasm_terp")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            cw_ics08_wasm_terp::execute,
            cw_ics08_wasm_terp::instantiate,
            cw_ics08_wasm_terp::query,
        ))
    }
}

/// JSON `{}`. The published `cw_ics08_wasm_zap1.wasm` accepts an empty
/// instantiate message. That crate is still pinned to crates.io cosmwasm-std
/// 1.5, so this suite does not call its Rust entry points.
#[cw_serde]
pub struct Zap1Instantiate {}

#[cw_serde]
pub enum Zap1Execute {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum Zap1Query {
    #[returns(String)]
    Unused {},
}

fn zap1_instantiate(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: Zap1Instantiate,
) -> StdResult<Response> {
    Ok(Response::new())
}

fn zap1_execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: Zap1Execute,
) -> StdResult<Response> {
    match msg {}
}

fn zap1_query(_deps: Deps, _env: Env, msg: Zap1Query) -> StdResult<Binary> {
    match msg {
        Zap1Query::Unused {} => Ok(Binary::default()),
    }
}

#[interface(Zap1Instantiate, Zap1Execute, Zap1Query, Empty, id = "cw-ics08-wasm-zap1")]
pub struct CwIcs08WasmZap1;

impl<Chain: TxHandler> Uploadable for CwIcs08WasmZap1<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("cw_ics08_wasm_zap1")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            zap1_execute,
            zap1_instantiate,
            zap1_query,
        ))
    }
}
