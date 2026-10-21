use crate::error::ContractError;
mod error;
pub mod msg;
pub mod state;

use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::DAO;

use terp_account::traits::default::TerpAccountTrait;

use cosmwasm_std::entry_point;
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, to_json_binary};
use cw2::set_contract_version;
use serde::{Deserialize, Serialize};

// Constants

const CONTRACT_NAME: &str = "crates.io:terp-dao";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerpAccountDaoStructs {}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerpAccountDao {}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    InstantiateMsg { dao_addr: _ }: InstantiateMsg,
) -> Result<Response, ContractError> {
    let submsg = vec![];
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    Ok(Response::new()
        .add_submessages(submsg)
        .add_attribute("method", "instantiate")
        .add_attribute("smart_account", info.sender))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {}
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetDao {} => to_json_binary(&DAO.load(deps.storage)?),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(
    deps: DepsMut,
    env: Env,
    req: <TerpAccountDao as TerpAccountTrait>::SudoMsg,
) -> Result<Response, ContractError> {
    TerpAccountDao::process_sudo_auth(deps, env, &req)
}

impl terp_account::traits::default::TerpAccountTrait for TerpAccountDao {
    type InstantiateMsg = crate::msg::InstantiateMsg;
    type ExecuteMsg = crate::msg::ExecuteMsg;
    type QueryMsg = crate::msg::QueryMsg;
    type SudoMsg = terp_auth::AuthSudoMsg;
    type ContractError = crate::error::ContractError;
    type AuthMethodStructs = TerpAccountDaoStructs;
    type AuthProcessResult = Result<Response, ContractError>;

    fn process_sudo_auth(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &Self::SudoMsg,
    ) -> Self::AuthProcessResult {
        match req {
            terp_auth::AuthSudoMsg::OnAuthAdded(req) => Self::on_auth_added(deps, env, req),
            terp_auth::AuthSudoMsg::OnAuthRemoved(req) => Self::on_auth_removed(deps, env, req),
            terp_auth::AuthSudoMsg::Authenticate(req) => Self::on_auth_request(deps, env, req),
            terp_auth::AuthSudoMsg::Track(req) => Self::on_auth_track(deps, env, req),
            terp_auth::AuthSudoMsg::ConfirmExecution(req) => Self::on_auth_confirm(deps, env, req),
        }
    }

    fn extended_authenticate(
        _deps: cosmwasm_std::DepsMut,
        _auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        todo!()
    }

    fn on_auth_added(
        _deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &terp_auth::OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        /// check if dao member
        ///
        /// - save auth by addr prefix to binary object for params (to be typed-defined later for things like filters/rate-limits)
        todo!()
    }

    fn on_auth_removed(
        _deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        todo!()
    }

    fn on_auth_request(
        _deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        // ensure still dao-member (raw-request)

        // check msg involves dao-goodie bag?

        todo!()
    }

    fn on_auth_track(
        _deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        // ?
        todo!()
    }

    fn on_auth_confirm(
        _deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        // ?
        todo!()
    }

    fn on_hooks(_deps: cosmwasm_std::DepsMut, _env: cosmwasm_std::Env) -> Self::AuthProcessResult {
        todo!()
    }
}
