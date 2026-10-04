//! Passkey authenticator.
//!
//! Authenticate expects a JSON [`PasskeyAuthPayload`] in `signature`.
//! [`webauthn::verify_passkey`] checks ES256 over
//! `authenticatorData || SHA-256(clientDataJSON)` and binds the WebAuthn
//! challenge to `sign_mode_direct`.

mod error;
mod state;
pub mod webauthn;

pub use error::ContractError;
pub use state::{PasskeyRegistration, REGISTRATION};
pub use webauthn::{assertion_message_hash, authenticator_data, client_data_json};

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};
use cw2::set_contract_version;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::AuthSudoMsg;

use crate::state::PasskeyRegistration as Reg;

const CONTRACT_NAME: &str = "crates.io:terp-passkey";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    pub registration: PasskeyRegistration,
}

#[cw_serde]
pub enum ExecuteMsg {
    UpdateOwnership(cw_ownable::Action),
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(PasskeyRegistration)]
    Registration {},
    #[returns(cw_ownable::Ownership<String>)]
    Ownership {},
}

/// Assertion payload in `AuthenticationRequest.signature`.
#[cw_serde]
pub struct PasskeyAuthPayload {
    pub origin: Option<String>,
    pub credential_id: Binary,
    /// ES256 signature: 64-byte `r || s`, or ASN.1 DER.
    pub assertion: Binary,
    pub authenticator_data: Binary,
    pub client_data_json: Binary,
}

pub type SudoMsg = AuthSudoMsg;

pub struct PasskeyAuthenticator;
#[derive(Clone, Debug)]
pub struct PasskeyAuthStructs {}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let owner = match msg.owner {
        Some(o) => deps.api.addr_validate(&o)?,
        None => info.sender,
    };
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(owner.as_str()))?;
    validate_registration(&msg.registration)?;
    REGISTRATION.save(deps.storage, &msg.registration)?;
    Ok(Response::new().add_attribute("action", "passkey_instantiate"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateOwnership(action) => {
            let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
            Ok(Response::new().add_attributes(ownership.into_attributes()))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Registration {} => {
            cosmwasm_std::to_json_binary(&REGISTRATION.load(deps.storage)?)
        }
        QueryMsg::Ownership {} => {
            let o = cw_ownable::get_ownership(deps.storage)?;
            cosmwasm_std::to_json_binary(&o)
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    PasskeyAuthenticator::process_sudo_auth(deps, env, &msg)
}

fn validate_registration(reg: &Reg) -> Result<(), ContractError> {
    if reg.origin.is_empty() || reg.rp_id.is_empty() || reg.credential_id.is_empty() {
        return Err(ContractError::InvalidCredential {
            reason: "origin, rp_id, and credential_id are required".into(),
        });
    }
    if reg.public_key.len() != 33 && reg.public_key.len() != 65 {
        return Err(ContractError::InvalidCredential {
            reason: "public_key must be 33 or 65 bytes".into(),
        });
    }
    Ok(())
}

impl TerpAccountTrait for PasskeyAuthenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = PasskeyAuthStructs;
    type AuthProcessResult = Result<Response, ContractError>;

    fn extended_authenticate(
        _deps: DepsMut,
        _auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "passkey_extended_auth"))
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
            Some(_) => Ok(Response::new().add_attribute("action", "passkey_on_auth_added")),
            None => Err(ContractError::MissingParams {}),
        }
    }

    fn on_auth_removed(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "passkey_on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let payload: PasskeyAuthPayload = cosmwasm_std::from_json(&req.signature).map_err(|e| {
            ContractError::InvalidCredential {
                reason: format!("decode: {e}"),
            }
        })?;
        webauthn::verify_passkey(
            deps.api,
            &REGISTRATION.load(deps.storage)?,
            &payload,
            req.sign_mode_tx_data.sign_mode_direct.as_slice(),
        )?;
        Ok(Response::new().add_attribute("action", "passkey_authenticate"))
    }

    fn on_auth_track(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "passkey_track"))
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "passkey_confirm"))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "passkey_hooks"))
    }
}
