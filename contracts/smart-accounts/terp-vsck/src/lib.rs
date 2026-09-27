//! # terp-vsck — Shielded Vote (VSCK) authenticator for DAOs
//!
//! Lets a DAO register a CosmWasm authenticator that gates transactions with
//! **private voting proofs** from the vote-sdk / Shielded Vote workflow:
//!
//! | Role | Circuit (vote-sdk) | Purpose |
//! |------|--------------------|---------|
//! | Delegator | `delegation` | Prove note / voting power |
//! | Voter | `vote_proof` | Cast ballot privately |
//! | Tallier | `share_reveal` | Ceremony share open |
//!
//! ## Integration sketch
//! 1. DAO governance instantiates `terp-vsck` with circuit VKs + dao address.
//! 2. DAO registers this contract as a CosmWasm authenticator on a smart account
//!    or as the voting module's auth gate for proposal execution messages.
//! 3. `OpenSession` binds a proposal to a note tree root from the vote chain / IBC.
//! 4. Voters attach [`VsckAuthPayload`] proofs to authorize ballot-related msgs.
//!
//! See `README.md` for architecture and relation to vote-sdk.

mod error;
mod msg;
mod state;
pub mod verify;
#[cfg(test)]
mod sudo_matrix;

pub use error::ContractError;
pub use msg::*;
pub use verify::{DefaultVsckVerifier, VsckVerifier};

use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw2::set_contract_version;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::AuthSudoMsg;

use crate::state::{Config, Session, CONFIG, NULLIFIERS, SESSIONS};

const CONTRACT_NAME: &str = "crates.io:terp-vsck";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub type SudoMsg = AuthSudoMsg;

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let admin = match msg.admin {
        Some(a) => deps.api.addr_validate(&a)?,
        None => info.sender.clone(),
    };
    let dao = deps.api.addr_validate(&msg.dao)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin: admin.clone(),
            dao: dao.clone(),
            circuit_vks: msg.circuit_vks,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "vsck_instantiate")
        .add_attribute("dao", dao)
        .add_attribute("admin", admin))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    match msg {
        ExecuteMsg::OpenSession {
            session_id,
            note_tree_root,
            proposal_ref,
        } => {
            if info.sender != cfg.admin && info.sender != cfg.dao {
                return Err(ContractError::Unauthorized {});
            }
            SESSIONS.save(
                deps.storage,
                &session_id,
                &Session {
                    session_id: session_id.clone(),
                    note_tree_root,
                    proposal_ref,
                    open: true,
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "vsck_open_session")
                .add_attribute("session_id", session_id))
        }
        ExecuteMsg::CloseSession { session_id } => {
            if info.sender != cfg.admin && info.sender != cfg.dao {
                return Err(ContractError::Unauthorized {});
            }
            let mut s = SESSIONS.load(deps.storage, &session_id)?;
            s.open = false;
            SESSIONS.save(deps.storage, &session_id, &s)?;
            Ok(Response::new().add_attribute("action", "vsck_close_session"))
        }
        ExecuteMsg::UpdateCircuitVks(vks) => {
            if info.sender != cfg.admin {
                return Err(ContractError::Unauthorized {});
            }
            let mut c = CONFIG.load(deps.storage)?;
            c.circuit_vks = vks;
            CONFIG.save(deps.storage, &c)?;
            Ok(Response::new().add_attribute("action", "vsck_update_vks"))
        }
        ExecuteMsg::UpdateAdmin { admin } => {
            if info.sender != cfg.admin {
                return Err(ContractError::Unauthorized {});
            }
            let mut c = CONFIG.load(deps.storage)?;
            c.admin = deps.api.addr_validate(&admin)?;
            CONFIG.save(deps.storage, &c)?;
            Ok(Response::new().add_attribute("action", "vsck_update_admin"))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let c = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: c.admin.to_string(),
                dao: c.dao.to_string(),
            })
        }
        QueryMsg::Session { session_id } => {
            let s = SESSIONS.load(deps.storage, &session_id)?;
            to_json_binary(&SessionResponse {
                session_id: s.session_id,
                note_tree_root: s.note_tree_root,
                proposal_ref: s.proposal_ref,
                open: s.open,
            })
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    VsckAuthenticator::process_sudo_auth(deps, env, &msg)
}

#[derive(Debug, Clone)]
pub struct VsckAuthStructs {}

pub struct VsckAuthenticator;

impl TerpAccountTrait for VsckAuthenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = VsckAuthStructs;
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
        if req.authenticator_params.is_none() {
            return Err(ContractError::MissingParams {});
        }
        Ok(Response::new().add_attribute("action", "vsck_on_auth_added"))
    }

    fn on_auth_removed(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "vsck_on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let payload: VsckAuthPayload = cosmwasm_std::from_json(&req.signature).map_err(|e| {
            ContractError::InvalidProof {
                reason: format!("payload decode: {e}"),
            }
        })?;

        let session = SESSIONS
            .may_load(deps.storage, &payload.session_id)?
            .ok_or_else(|| ContractError::UnknownSession {
                session_id: payload.session_id.clone(),
            })?;

        let cfg = CONFIG.load(deps.storage)?;
        DefaultVsckVerifier.verify(deps.as_ref(), &cfg.circuit_vks, &session, &payload)?;

        Ok(Response::new()
            .add_attribute("action", "vsck_authenticate")
            .add_attribute("session_id", payload.session_id)
            .add_attribute("role", format!("{:?}", payload.role)))
    }

    fn on_auth_track(
        deps: DepsMut,
        _env: Env,
        req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        if let Some(params) = &req.authenticator_params {
            if let Ok(payload) = cosmwasm_std::from_json::<VsckAuthPayload>(params) {
                let mut key = payload.session_id.as_bytes().to_vec();
                key.push(0);
                key.extend_from_slice(payload.nullifier.as_slice());
                if NULLIFIERS.has(deps.storage, &key) {
                    return Err(ContractError::NullifierReplay {});
                }
                NULLIFIERS.save(deps.storage, &key, &true)?;
            }
        }
        Ok(Response::new().add_attribute("action", "vsck_track"))
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "vsck_confirm"))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }
}
