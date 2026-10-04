//! # terp-zkjwt — JWT ownership proofs via CosmWasm host ZK verify
//!
//! **Full flow:** see [`JWT_AUTH_FLOW.md`](../JWT_AUTH_FLOW.md).
//!
//! ## Cryptographic path (feature `zk-host`)
//!
//! ```text
//! Authenticate:
//!   1. Decode ZkJwtAuthPayload from signature
//!   2. Load IssuerConfig → zkid
//!   3. Parse public_inputs = nullifier || claim_commitment || [msg_bind] || rest
//!   4. deps.api.proof_instance_verify(zkid, proof, public_inputs)
//!   5. Optional: enforce msg_bind == compute_msg_bind(...)
//!   6. Bind claim to account if required; reject spent nullifier; store nullifier
//! ```
//!
//! Without `zk-host`, structural envelope checks only (Mock CI).

pub mod circom_codec;
mod error;
pub mod fixtures;
mod instances;
pub mod msg;
mod state;
pub mod verify;

pub use circom_codec::{
    claim_is_sound, decode_circom_publics_be, decode_host_instances_bytes, encode_host_instances,
    parse_public_json_array, CircomJwtPolicyClaim, CIRCOM_JWT_CODEC_V1, CIRCOM_JWT_MIN_POLICY_PUBLICS,
    CIRCOM_JWT_N_PUBLIC, CLAIM_EVENT_SCHEMA_V1, MIN_SOUND_CLAIM_RICHNESS,
};
pub use error::ContractError;
pub use instances::{
    build_public_inputs, compute_msg_bind, parse_public_instances, JwtPublicInstances,
    MIN_INSTANCES_LEN, WITH_INCLUSION_ROOT_LEN, WITH_ROOT_AND_MSG_BIND_LEN,
};
pub use msg::*;
pub use state::Config;
pub use verify::{circuit_ids, default_verifier, DefaultZkJwtVerifier, ZkJwtVerifier};
#[cfg(feature = "zk-host")]
pub use verify::HostZkJwtVerifier;

use cosmwasm_std::{
    to_json_binary, Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw2::set_contract_version;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::AuthSudoMsg;

use crate::state::{Config as Cfg, CLAIMS, CONFIG, ISSUERS, NULLIFIERS};

const CONTRACT_NAME: &str = "crates.io:terp-zkjwt";
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
    CONFIG.save(
        deps.storage,
        &Cfg {
            admin: admin.clone(),
            require_registered_claim: msg.require_registered_claim,
        },
    )?;
    for issuer in msg.issuers {
        ISSUERS.save(deps.storage, &issuer.issuer, &issuer)?;
    }
    Ok(Response::new()
        .add_attribute("action", "zkjwt_instantiate")
        .add_attribute("admin", admin)
        .add_attribute(
            "verify_mode",
            if cfg!(feature = "zk-host") {
                "proof_instance_verify"
            } else {
                "structural"
            },
        ))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpsertIssuer(cfg) => {
            only_admin(deps.as_ref(), &info)?;
            let id = cfg.issuer.clone();
            ISSUERS.save(deps.storage, &id, &cfg)?;
            Ok(Response::new().add_attribute("action", "zkjwt_upsert_issuer"))
        }
        ExecuteMsg::RemoveIssuer { issuer } => {
            only_admin(deps.as_ref(), &info)?;
            ISSUERS.remove(deps.storage, &issuer);
            Ok(Response::new().add_attribute("action", "zkjwt_remove_issuer"))
        }
        ExecuteMsg::RotateInclusionRoot {
            issuer,
            inclusion_set_root,
        } => {
            only_admin(deps.as_ref(), &info)?;
            if inclusion_set_root.len() != 32 {
                return Err(ContractError::InvalidProof {
                    reason: "inclusion_set_root must be 32 bytes".into(),
                });
            }
            let mut cfg = ISSUERS.load(deps.storage, &issuer)?;
            cfg.inclusion_set_root = Some(inclusion_set_root);
            ISSUERS.save(deps.storage, &issuer, &cfg)?;
            Ok(Response::new()
                .add_attribute("action", "zkjwt_rotate_inclusion_root")
                .add_attribute("issuer", issuer))
        }
        ExecuteMsg::RegisterClaim {
            claim_commitment,
            issuer,
        } => {
            if !ISSUERS.has(deps.storage, &issuer) {
                return Err(ContractError::UnknownIssuer { issuer });
            }
            CLAIMS.save(deps.storage, claim_commitment.as_slice(), &info.sender)?;
            Ok(Response::new()
                .add_attribute("action", "zkjwt_register_claim")
                .add_attribute("account", info.sender))
        }
        ExecuteMsg::UpdateAdmin { admin } => {
            only_admin(deps.as_ref(), &info)?;
            let mut c = CONFIG.load(deps.storage)?;
            c.admin = deps.api.addr_validate(&admin)?;
            CONFIG.save(deps.storage, &c)?;
            Ok(Response::new().add_attribute("action", "zkjwt_update_admin"))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Issuer { issuer } => to_json_binary(&ISSUERS.load(deps.storage, &issuer)?),
        QueryMsg::ListIssuers {} => {
            let keys: StdResult<Vec<_>> = ISSUERS
                .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
                .collect();
            to_json_binary(&keys?)
        }
        QueryMsg::IsClaimRegistered { claim_commitment } => {
            to_json_binary(&CLAIMS.has(deps.storage, claim_commitment.as_slice()))
        }
        QueryMsg::ClaimOwner { claim_commitment } => {
            let owner = CLAIMS.may_load(deps.storage, claim_commitment.as_slice())?;
            to_json_binary(&owner.map(|a| a.to_string()))
        }
        QueryMsg::Config {} => {
            let c = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: c.admin.to_string(),
                require_registered_claim: c.require_registered_claim,
            })
        }
        QueryMsg::InclusionSetRoot { issuer } => {
            let cfg = ISSUERS.load(deps.storage, &issuer)?;
            to_json_binary(&cfg.inclusion_set_root)
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    ZkJwtAuthenticator::process_sudo_auth(deps, env, &msg)
}

fn only_admin(deps: Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let c = CONFIG.load(deps.storage)?;
    if info.sender != c.admin {
        return Err(ContractError::Unauthorized {});
    }
    Ok(())
}

/// Bind context for sudo `AuthenticationRequest` (msg_bind). Execute path has none.
struct MsgBindCtx<'a> {
    chain_id: &'a str,
    account: &'a str,
    authenticator_id: &'a str,
    msg_index: u64,
    sign_mode_direct: &'a [u8],
}

fn authenticate_payload(
    deps: DepsMut,
    env: Env,
    payload: ZkJwtAuthPayload,
    account: Option<&Addr>,
    bind: Option<MsgBindCtx<'_>>,
) -> Result<Response, ContractError> {
    let issuer = ISSUERS
        .may_load(deps.storage, &payload.issuer)?
        .ok_or_else(|| ContractError::UnknownIssuer {
            issuer: payload.issuer.clone(),
        })?;

    default_verifier().verify(deps.as_ref(), &issuer, &payload)?;

    let (nf, claim_bytes, codec_attr): (Vec<u8>, Vec<u8>, &str) =
        if crate::circom_codec::is_circom_host_instances(payload.public_inputs.as_slice()) {
            let claim = crate::circom_codec::decode_host_instances_bytes(
                payload.public_inputs.as_slice(),
            )?;
            if payload.claim_commitment.as_slice() != claim.claim_commitment.as_slice() {
                return Err(ContractError::InvalidProof {
                    reason: "claim_commitment does not match circom accountSalt".into(),
                });
            }
            if issuer.inclusion_set_root.is_some() {
                return Err(ContractError::InvalidProof {
                    reason: "inclusion_set_root issuer incompatible with circom-jwt-v1 PI"
                        .into(),
                });
            }
            (
                claim.nullifier.to_vec(),
                claim.claim_commitment.to_vec(),
                crate::circom_codec::CIRCOM_JWT_CODEC_V1,
            )
        } else {
            let inst = crate::instances::parse_public_instances(&payload.public_inputs)?;

            if let Some(registered_root) = &issuer.inclusion_set_root {
                if registered_root.len() != 32 {
                    return Err(ContractError::InvalidProof {
                        reason: "issuer inclusion_set_root must be 32 bytes".into(),
                    });
                }
                let Some(pi_root) = inst.inclusion_set_root else {
                    return Err(ContractError::InvalidProof {
                        reason:
                            "public_inputs missing inclusion_set_root [64..96); issuer requires root"
                                .into(),
                    });
                };
                if pi_root != registered_root.as_slice() {
                    return Err(ContractError::InvalidProof {
                        reason:
                            "inclusion_set_root mismatch (public instance != IssuerConfig root)"
                                .into(),
                    });
                }
            }

            if let (Some(mb), Some(b)) = (inst.msg_bind, bind) {
                let expected = crate::instances::compute_msg_bind(
                    b.chain_id,
                    b.account,
                    b.authenticator_id,
                    b.msg_index,
                    b.sign_mode_direct,
                );
                if !crate::instances::msg_bind_matches(mb, &expected) {
                    return Err(ContractError::InvalidProof {
                        reason: "msg_bind mismatch (public instance != host compute_msg_bind)"
                            .into(),
                    });
                }
            }

            (
                inst.nullifier.to_vec(),
                inst.claim_commitment.to_vec(),
                "policy-v1",
            )
        };

    let cfg = CONFIG.load(deps.storage)?;
    if cfg.require_registered_claim {
        match CLAIMS.may_load(deps.storage, payload.claim_commitment.as_slice())? {
            Some(owner) if account.map(|a| owner == *a).unwrap_or(false) => {}
            Some(_) => {
                return Err(ContractError::InvalidProof {
                    reason: "claim_commitment registered to different account".into(),
                });
            }
            None => return Err(ContractError::UnregisteredClaim {}),
        }
    }
    let _ = claim_bytes;

    if NULLIFIERS.has(deps.storage, &nf) {
        return Err(ContractError::NullifierReplay {});
    }
    NULLIFIERS.save(deps.storage, &nf, &env.block.height)?;

    Ok(Response::new()
        .add_attribute("action", "zkjwt_authenticate")
        .add_attribute("issuer", payload.issuer)
        .add_attribute("circuit_id", payload.circuit_id)
        .add_attribute("codec", codec_attr)
        .add_attribute(
            "verify",
            if cfg!(feature = "zk-host") {
                "proof_instance_verify"
            } else {
                "structural"
            },
        )
        .add_attribute(
            "zkid",
            issuer
                .zkid
                .map(|z| z.to_string())
                .unwrap_or_else(|| "none".into()),
        ))
}

#[derive(Debug, Clone)]
pub struct ZkJwtAuthStructs {}

pub struct ZkJwtAuthenticator;

impl TerpAccountTrait for ZkJwtAuthenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = ZkJwtAuthStructs;
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
        Ok(Response::new().add_attribute("action", "zkjwt_on_auth_added"))
    }

    fn on_auth_removed(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "zkjwt_on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let payload: ZkJwtAuthPayload = cosmwasm_std::from_json(&req.signature).map_err(|e| {
            ContractError::InvalidProof {
                reason: format!("payload decode: {e}"),
            }
        })?;
        authenticate_payload(
            deps,
            env,
            payload,
            Some(&req.account),
            Some(MsgBindCtx {
                chain_id: &req.tx_data.chain_id,
                account: req.account.as_str(),
                authenticator_id: &req.authenticator_id,
                msg_index: req.msg_index,
                sign_mode_direct: req.sign_mode_tx_data.sign_mode_direct.as_slice(),
            }),
        )
    }

    fn on_auth_track(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        // Nullifier already spent in Authenticate (module may not pass proof on Track).
        Ok(Response::new().add_attribute("action", "zkjwt_track"))
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "zkjwt_confirm"))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }
}
