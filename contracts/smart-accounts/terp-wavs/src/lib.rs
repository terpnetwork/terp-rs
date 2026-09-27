pub mod contract;
mod error;
pub mod msg;
mod state;

use cosmwasm_std::{to_json_binary, Env, HashFunction, Response, BLS12_381_G1_GENERATOR};
use serde::{Deserialize, Serialize};

pub use crate::error::ContractError;
use crate::state::WAVS_PUBKEY;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerpAccountWavsAuthStruct {}

#[derive(Debug, Clone, Serialize, Default, Deserialize)]
pub struct TerpAccountWavs {}
impl terp_account::traits::default::TerpAccountTrait for TerpAccountWavs {
    type InstantiateMsg = crate::msg::InstantiateMsg;
    type ExecuteMsg = crate::msg::ExecuteMsg;
    type QueryMsg = crate::msg::QueryMsg;
    type SudoMsg = terp_auth::AuthSudoMsg;
    type ContractError = crate::error::ContractError;
    type AuthMethodStructs = TerpAccountWavsAuthStruct;
    type AuthProcessResult = Result<Response, ContractError>;

    fn process_sudo_auth(
        deps: cosmwasm_std::DepsMut,
        env: Env,
        msg: &Self::SudoMsg,
    ) -> Self::AuthProcessResult {
        match msg {
            Self::SudoMsg::OnAuthAdded(auth_add) => Self::on_auth_added(deps, env, &auth_add),
            Self::SudoMsg::OnAuthRemoved(auth_remove) => {
                Self::on_auth_removed(deps, env, &auth_remove)
            }
            Self::SudoMsg::Authenticate(auth_req) => Self::on_auth_request(deps, env, &auth_req),
            Self::SudoMsg::Track(track_req) => Self::on_auth_track(deps, env, &track_req),
            Self::SudoMsg::ConfirmExecution(conf_exec_req) => {
                Self::on_auth_confirm(deps, env, &conf_exec_req)
            }
        }
    }

    fn on_auth_added(
        deps: cosmwasm_std::DepsMut,
        env: Env,
        req: &terp_auth::OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        //TODO: check member is a part of all DAOS registering for membership check
        //TODO: register RBAM json filters for specific DAOs, if any: https://github.com/DA0-DA0/dao-contracts/blob/development/packages/cw-jsonfilter/README.md
        // small storage writes, for example global contract entropy or count of registered accounts
        match req.authenticator_params {
            Some(_) => Ok(Response::new().add_attribute("action", "auth_added_req")),
            None => Err(ContractError::MissingAuthenticatorMetadata {}),
        }
    }

    fn on_auth_removed(
        deps: cosmwasm_std::DepsMut,
        env: Env,
        req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        //TODO: remove data set
        Ok(Response::new().add_attribute("action", "auth_removed_req"))
    }

    fn on_auth_request(
        deps: cosmwasm_std::DepsMut,
        env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let pubkeys = WAVS_PUBKEY.load(deps.storage)?;
        // assert the wavs operator signature length
        let a = req.signature_data.signers.len();
        let b = pubkeys.threshold;
        if a < b {
            return Err(ContractError::InvalidPubkeyCount { a, b });
        }

        // EXAMPLE IMPLEMENTATION FOR BLS12_381 VERIFICATION COMMONWARE-CRYPTO -> COSMWASM_STD
        // TODO: improve functionality/integrate described workflow here: https://gist.github.com/liangping/3809ca1f1c13bc250217041a7d343fcf
        if !deps.api.bls12_381_pairing_equality(
            &BLS12_381_G1_GENERATOR,
            &deps.api.bls12_381_aggregate_g2(
                &req.clone()
                    .signature_data
                    .signatures
                    .into_iter()
                    .map(|a| a.clone().to_vec())
                    .collect::<Vec<_>>()
                    .concat(),
            )?,
            &deps.api.bls12_381_aggregate_g1(
                &req.signature_data
                    .signatures
                    .iter()
                    .map(|a| a.to_vec())
                    .collect::<Vec<_>>()
                    .concat(),
            )?,
            &deps.api.bls12_381_hash_to_g2(
                HashFunction::Sha256,
                &to_json_binary(&req.tx_data.msgs)?,
                b"QUUX-V01-CS02-with-BLS12381G1_XMD:SHA-256_SSWU_RO_",
            )?,
        )? {
            return Err(ContractError::VerificationError(
                cosmwasm_std::VerificationError::GenericErr,
            ));
        }

        Ok(Response::new().add_attribute("action", "auth_req"))
    }

    fn on_auth_track(
        deps: cosmwasm_std::DepsMut,
        env: Env,
        req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "track_req"))
    }

    fn on_auth_confirm(
        deps: cosmwasm_std::DepsMut,
        env: Env,
        req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "conf_exec_req"))
    }

    fn on_hooks(deps: cosmwasm_std::DepsMut, env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }

    fn extended_authenticate(
        deps: cosmwasm_std::DepsMut,
        auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        todo!()
    }
}
