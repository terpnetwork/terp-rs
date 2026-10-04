pub mod contract;
mod error;
pub mod msg;
mod state;
pub mod verify;

use cosmwasm_std::{Env, Response};

pub use crate::error::ContractError;
pub use crate::msg::{BlsAuthPayload, BlsVariant};
pub use crate::state::BLS_CONFIG;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TerpAccountBlsAuthStruct {}

#[derive(Debug, Clone, serde::Serialize, Default, serde::Deserialize)]
pub struct TerpAccountBls {}

impl terp_account::traits::default::TerpAccountTrait for TerpAccountBls {
    type InstantiateMsg = crate::msg::InstantiateMsg;
    type ExecuteMsg = crate::msg::ExecuteMsg;
    type QueryMsg = crate::msg::QueryMsg;
    type SudoMsg = terp_auth::AuthSudoMsg;
    type ContractError = crate::error::ContractError;
    type AuthMethodStructs = TerpAccountBlsAuthStruct;
    type AuthProcessResult = Result<Response, ContractError>;

    fn process_sudo_auth(
        deps: cosmwasm_std::DepsMut,
        env: Env,
        msg: &Self::SudoMsg,
    ) -> Self::AuthProcessResult {
        match msg {
            Self::SudoMsg::OnAuthAdded(auth_add) => Self::on_auth_added(deps, env, auth_add),
            Self::SudoMsg::OnAuthRemoved(auth_remove) => {
                Self::on_auth_removed(deps, env, auth_remove)
            }
            Self::SudoMsg::Authenticate(auth_req) => Self::on_auth_request(deps, env, auth_req),
            Self::SudoMsg::Track(track_req) => Self::on_auth_track(deps, env, track_req),
            Self::SudoMsg::ConfirmExecution(conf_exec_req) => {
                Self::on_auth_confirm(deps, env, conf_exec_req)
            }
        }
    }

    fn on_auth_added(
        _deps: cosmwasm_std::DepsMut,
        _env: Env,
        req: &terp_auth::OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        match req.authenticator_params {
            Some(_) => Ok(Response::new().add_attribute("action", "auth_added_req")),
            None => Err(ContractError::MissingAuthenticatorMetadata {}),
        }
    }

    fn on_auth_removed(
        _deps: cosmwasm_std::DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "auth_removed_req"))
    }

    fn on_auth_request(
        deps: cosmwasm_std::DepsMut,
        _env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let cfg = BLS_CONFIG.load(deps.storage)?;
        let payload: BlsAuthPayload =
            cosmwasm_std::from_json(&req.signature).map_err(|e| ContractError::InvalidSignature {
                reason: format!("payload: {e}"),
            })?;
        let sigs = &req.signature_data.signatures;
        if payload.key_indices.len() != sigs.len() || sigs.is_empty() {
            return Err(ContractError::InvalidSignature {
                reason: "key_indices and signatures must be the same non-empty length".into(),
            });
        }
        let message = req.sign_mode_tx_data.sign_mode_direct.as_slice();
        let mut seen = std::collections::BTreeSet::<u32>::new();
        for (index, sig) in payload.key_indices.iter().zip(sigs.iter()) {
            if !seen.insert(*index) {
                return Err(ContractError::InvalidSignature {
                    reason: format!("duplicate key index {index}"),
                });
            }
            let key = cfg.operator_keys.get(*index as usize).ok_or_else(|| {
                ContractError::InvalidSignature {
                    reason: format!("key index {index} out of range"),
                }
            })?;
            verify::verify_one(deps.api, &cfg.variant, key.as_slice(), sig.as_slice(), message)?;
        }
        let got = seen.len() as u32;
        if got < cfg.threshold {
            return Err(ContractError::ThresholdNotMet {
                got,
                need: cfg.threshold,
            });
        }
        Ok(Response::new()
            .add_attribute("action", "auth_req")
            .add_attribute("variant", cfg.variant.as_str())
            .add_attribute("signers", got.to_string()))
    }

    fn on_auth_track(
        _deps: cosmwasm_std::DepsMut,
        _env: Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "track_req"))
    }

    fn on_auth_confirm(
        _deps: cosmwasm_std::DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "conf_exec_req"))
    }

    fn on_hooks(_deps: cosmwasm_std::DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }

    fn extended_authenticate(
        _deps: cosmwasm_std::DepsMut,
        _auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "extended_auth"))
    }
}
