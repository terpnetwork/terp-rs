//! cw-orch interfaces for account, ticketing, and IBC dispatcher contracts.
//!
//! Same shape as `terp-authenticator-suite`: the interface lives with the
//! suite so guest wasm builds do not depend on cw-orch.

use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use polytone_any_proxy::{execute as poly_execute, instantiate as poly_instantiate, query as poly_query};
use terp_dao::msg::{ExecuteMsg as DaoExecute, InstantiateMsg as DaoInstantiate, QueryMsg as DaoQuery};
use terp_dao::{execute as dao_execute, instantiate as dao_instantiate, query as dao_query};
use terp_dregg::{execute as dregg_execute, instantiate as dregg_instantiate, query as dregg_query};
use terp_frost::{execute as frost_execute, instantiate as frost_instantiate, query as frost_query};
use terp_bls::contract::{execute as bls_execute, instantiate as bls_instantiate, query as bls_query};
use terp_bls::msg::{ExecuteMsg as BlsExecute, InstantiateMsg as BlsInstantiate, QueryMsg as BlsQuery};
use terp_zktls::{execute as tls_execute, instantiate as tls_instantiate, query as tls_query};

use super::wasm_path::wasm;

#[interface(DaoInstantiate, DaoExecute, DaoQuery, Empty, id = "terp-dao")]
pub struct TerpDao;

impl<Chain: TxHandler> Uploadable for TerpDao<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("terp_dao")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            dao_execute,
            dao_instantiate,
            dao_query,
        ))
    }
}

#[interface(
    terp_dregg::InstantiateMsg,
    terp_dregg::ExecuteMsg,
    terp_dregg::QueryMsg,
    Empty,
    id = "terp-dregg"
)]
pub struct TerpDregg;

impl<Chain: TxHandler> Uploadable for TerpDregg<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("terp_dregg")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            dregg_execute,
            dregg_instantiate,
            dregg_query,
        ))
    }
}

#[interface(
    terp_frost::InstantiateMsg,
    terp_frost::ExecuteMsg,
    terp_frost::QueryMsg,
    Empty,
    id = "terp-frost"
)]
pub struct TerpFrost;

impl<Chain: TxHandler> Uploadable for TerpFrost<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("terp_frost")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            frost_execute,
            frost_instantiate,
            frost_query,
        ))
    }
}

#[interface(BlsInstantiate, BlsExecute, BlsQuery, Empty, id = "terp-bls")]
pub struct TerpBls;

impl<Chain: TxHandler> Uploadable for TerpBls<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("terp_bls")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            bls_execute,
            bls_instantiate,
            bls_query,
        ))
    }
}

#[interface(
    terp_zktls::InstantiateMsg,
    terp_zktls::ExecuteMsg,
    terp_zktls::QueryMsg,
    Empty,
    id = "terp-zktls"
)]
pub struct TerpZkTls;

impl<Chain: TxHandler> Uploadable for TerpZkTls<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("terp_zktls")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            tls_execute,
            tls_instantiate,
            tls_query,
        ))
    }
}

#[interface(
    polytone_any_proxy::InstantiateMsg,
    polytone_any_proxy::ExecuteMsg,
    polytone_any_proxy::QueryMsg,
    Empty,
    id = "polytone-any-proxy"
)]
pub struct PolytoneAnyProxy;

impl<Chain: TxHandler> Uploadable for PolytoneAnyProxy<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        wasm("polytone_any_proxy")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            poly_execute,
            poly_instantiate,
            poly_query,
        ))
    }
}
