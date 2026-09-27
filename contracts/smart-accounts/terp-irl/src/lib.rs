//! IRL / witness authenticator — trait-complete scaffold.
//!
//! Historical epoch/witness crypto remains future work. This module implements
//! the full `TerpAccountTrait` workflow with structural witness checks so the
//! suite and x/smart-account routing can be exercised end-to-end.

mod error;

pub use error::ContractError;

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::AuthSudoMsg;

const CONTRACT_NAME: &str = "crates.io:terp-irl";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const EPOCH_ROOT: Item<Binary> = Item::new("epoch_root");

#[cw_serde]
pub struct InstantiateMsg {
    /// Current epoch membership root (opaque).
    pub epoch_root: Binary,
}

#[cw_serde]
pub enum ExecuteMsg {
    RotateEpoch { epoch_root: Binary },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Binary)]
    EpochRoot {},
}

/// Witness proof payload (structural until real IRL circuits return).
#[cw_serde]
pub struct IrlWitnessPayload {
    pub epoch_root: Binary,
    pub witness: Binary,
    pub nullifier: Binary,
}

pub type SudoMsg = AuthSudoMsg;

pub struct IrlAuthenticator;
#[derive(Clone, Debug)]
pub struct IrlAuthStructs {}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    EPOCH_ROOT.save(deps.storage, &msg.epoch_root)?;
    Ok(Response::new().add_attribute("action", "irl_instantiate"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::RotateEpoch { epoch_root } => {
            EPOCH_ROOT.save(deps.storage, &epoch_root)?;
            Ok(Response::new().add_attribute("action", "irl_rotate_epoch"))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::EpochRoot {} => cosmwasm_std::to_json_binary(&EPOCH_ROOT.load(deps.storage)?),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    IrlAuthenticator::process_sudo_auth(deps, env, &msg)
}

impl TerpAccountTrait for IrlAuthenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = IrlAuthStructs;
    type AuthProcessResult = Result<Response, ContractError>;

    fn extended_authenticate(
        _deps: DepsMut,
        _auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        Ok(Response::new())
    }

    fn process_sudo_auth(deps: DepsMut, env: Env, req: &Self::SudoMsg) -> Self::AuthProcessResult {
        match req {
            AuthSudoMsg::OnAuthAdded(r) => Self::on_auth_added(deps, env, r),
            AuthSudoMsg::OnAuthRemoved(r) => Self::on_auth_removed(deps, env, r),
            AuthSudoMsg::Authenticate(r) => Self::on_auth_request(deps, env, r),
            AuthSudoMsg::Track(r) => Self::on_auth_track(deps, env, r),
            AuthSudoMsg::ConfirmExecution(r) => Self::on_auth_confirm(deps, env, r),
        }
    }

    fn on_auth_added(
        _deps: DepsMut,
        _env: Env,
        req: &terp_auth::OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        match req.authenticator_params {
            Some(_) => Ok(Response::new().add_attribute("action", "irl_on_auth_added")),
            None => Err(ContractError::MissingAuthenticatorMetadata {}),
        }
    }

    fn on_auth_removed(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "irl_on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let payload: IrlWitnessPayload = cosmwasm_std::from_json(&req.signature).map_err(|e| {
            ContractError::InvalidWitness {
                reason: format!("decode: {e}"),
            }
        })?;
        if payload.witness.is_empty() || payload.nullifier.is_empty() {
            return Err(ContractError::InvalidWitness {
                reason: "empty witness/nullifier".into(),
            });
        }
        let root = EPOCH_ROOT.load(deps.storage)?;
        if payload.epoch_root != root {
            return Err(ContractError::InvalidWitness {
                reason: "epoch_root mismatch".into(),
            });
        }
        // TODO: real IRL membership verify against epoch tree
        Ok(Response::new().add_attribute("action", "irl_authenticate"))
    }

    fn on_auth_track(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "irl_track"))
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "irl_confirm"))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }
}
