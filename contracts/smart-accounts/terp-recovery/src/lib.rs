//! # terp-recovery — break-glass authenticator
//!
//! ## Flow
//! 1. Instantiate with guardians, threshold, **hash algorithm**, and rehash rounds.
//! 2. On Authenticate, compute a **domain-separated challenge digest**:
//!    ```text
//!    digest = rehash_N(hash_alg, domain || chain_id || account || auth_id || sign_mode_direct)
//!    ```
//! 3. Each guardian approval must either:
//!    - **Ed25519**: 64-byte signature over `digest` (when `pubkey` registered), via host
//!      `ed25519_verify`, or
//!    - **Digest attestation** (address-only MVP): `approval.attestation == rehash(alg,
//!      guardian || 0x00 || digest)` so clients prove knowledge of the same break-glass
//!      hashing path without storing guardian keys on-chain.
//! 4. Distinct valid guardian approvals ≥ threshold → allow.
//!
//! See [`hash`] for algorithm catalog and domain separation.

mod error;
pub mod hash;

pub use error::ContractError;
pub use hash::{
    break_glass_challenge, challenge_preimage, poseidon_pallas_hash_once, rehash, RecoveryHashAlg,
    DEFAULT_DOMAIN,
};

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::AuthSudoMsg;

const CONTRACT_NAME: &str = "crates.io:terp-recovery";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const CONFIG: Item<RecoveryConfig> = Item::new("cfg");

const ED25519_SIG_LEN: usize = 64;
const ED25519_PK_LEN: usize = 32;

/// One registered guardian.
#[cw_serde]
pub struct GuardianConfig {
    /// Bech32 account address (canonical guardian id).
    pub address: String,
    /// Optional 32-byte ed25519 public key for signature break-glass.
    pub pubkey: Option<Binary>,
    /// Optional commitment `rehash(addr||salt||pk)` for stronger registration.
    pub commitment: Option<Binary>,
}

/// On-chain recovery policy.
#[cw_serde]
pub struct RecoveryConfig {
    pub guardians: Vec<GuardianConfig>,
    pub threshold: u32,
    /// Digest algorithm used for break-glass challenge rehashing.
    pub hash_alg: RecoveryHashAlg,
    /// Sequential rehash rounds (1..=64). Default should be 1 unless matching an external scheme.
    pub rehash_rounds: u32,
    /// Domain tag (default `terp-recovery/break-glass/v1`).
    pub domain: String,
    /// If true, guardians without pubkey may approve via digest attestation.
    pub allow_address_only_approvals: bool,
}

impl RecoveryConfig {
    pub fn validate(&self, api: &dyn cosmwasm_std::Api) -> Result<(), ContractError> {
        if self.guardians.is_empty() || self.threshold == 0 {
            return Err(ContractError::MissingConfig {});
        }
        if self.threshold as usize > self.guardians.len() {
            return Err(ContractError::ThresholdNotMet {
                got: 0,
                need: self.threshold,
            });
        }
        if !(1..=64).contains(&self.rehash_rounds) {
            return Err(ContractError::InvalidRounds {
                got: self.rehash_rounds,
            });
        }
        if self.domain.is_empty() {
            return Err(ContractError::HashAlg {
                reason: "empty domain".into(),
            });
        }
        for g in &self.guardians {
            api.addr_validate(&g.address).map_err(|e| {
                ContractError::InvalidGuardian {
                    reason: format!("{}: {e}", g.address),
                }
            })?;
            if let Some(pk) = &g.pubkey {
                if pk.len() != ED25519_PK_LEN {
                    return Err(ContractError::InvalidGuardian {
                        reason: format!(
                            "{}: pubkey must be {ED25519_PK_LEN} bytes",
                            g.address
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

#[cw_serde]
pub struct InstantiateMsg {
    pub config: RecoveryConfig,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Replace full recovery config (should be gated by owner / governance in production).
    UpdateConfig(RecoveryConfig),
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(RecoveryConfig)]
    Config {},
    /// Compute challenge digest for the given auth context (client helper).
    #[returns(ChallengeResponse)]
    Challenge {
        chain_id: String,
        account: String,
        authenticator_id: String,
        sign_mode_direct: Binary,
    },
}

#[cw_serde]
pub struct ChallengeResponse {
    pub hash_alg: RecoveryHashAlg,
    pub rehash_rounds: u32,
    pub domain: String,
    pub digest: Binary,
}

/// One guardian's break-glass approval in `AuthenticationRequest.signature`.
#[cw_serde]
pub struct GuardianApproval {
    pub guardian: String,
    /// Ed25519 signature over **challenge digest bytes as message** (64 bytes),
    /// OR digest attestation when address-only (see module docs).
    pub signature: Binary,
}

/// Authenticate payload (JSON in `signature` field).
#[cw_serde]
pub struct RecoveryAuthPayload {
    /// Must match on-chain `config.hash_alg` (explicit for client/server agreement).
    pub hash_alg: RecoveryHashAlg,
    /// Must match on-chain `config.rehash_rounds`.
    pub rehash_rounds: u32,
    pub approvals: Vec<GuardianApproval>,
}

pub type SudoMsg = AuthSudoMsg;

pub struct RecoveryAuthenticator;
#[derive(Clone, Debug)]
pub struct RecoveryAuthStructs {}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let mut cfg = msg.config;
    if cfg.domain.is_empty() {
        cfg.domain = DEFAULT_DOMAIN.into();
    }
    if cfg.rehash_rounds == 0 {
        cfg.rehash_rounds = 1;
    }
    cfg.validate(deps.api)?;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "recovery_instantiate")
        .add_attribute("hash_alg", cfg.hash_alg.as_str())
        .add_attribute("rehash_rounds", cfg.rehash_rounds.to_string()))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateConfig(mut cfg) => {
            if cfg.domain.is_empty() {
                cfg.domain = DEFAULT_DOMAIN.into();
            }
            if cfg.rehash_rounds == 0 {
                cfg.rehash_rounds = 1;
            }
            cfg.validate(deps.api)?;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new()
                .add_attribute("action", "recovery_update_config")
                .add_attribute("hash_alg", cfg.hash_alg.as_str()))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => cosmwasm_std::to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Challenge {
            chain_id,
            account,
            authenticator_id,
            sign_mode_direct,
        } => {
            let cfg = CONFIG.load(deps.storage)?;
            let digest = break_glass_challenge(
                cfg.hash_alg,
                cfg.rehash_rounds,
                &cfg.domain,
                &chain_id,
                &account,
                &authenticator_id,
                sign_mode_direct.as_slice(),
            )
            .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
            cosmwasm_std::to_json_binary(&ChallengeResponse {
                hash_alg: cfg.hash_alg,
                rehash_rounds: cfg.rehash_rounds,
                domain: cfg.domain,
                digest: Binary::from(digest),
            })
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    RecoveryAuthenticator::process_sudo_auth(deps, env, &msg)
}

fn verify_approval(
    api: &dyn cosmwasm_std::Api,
    cfg: &RecoveryConfig,
    challenge: &[u8],
    approval: &GuardianApproval,
) -> Result<(), ContractError> {
    let gcfg = cfg
        .guardians
        .iter()
        .find(|g| g.address == approval.guardian)
        .ok_or_else(|| ContractError::ApprovalFailed {
            guardian: approval.guardian.clone(),
            reason: "not a registered guardian".into(),
        })?;

    if let Some(pk) = &gcfg.pubkey {
        // Ed25519 over challenge digest bytes (host import)
        if approval.signature.len() != ED25519_SIG_LEN {
            return Err(ContractError::ApprovalFailed {
                guardian: approval.guardian.clone(),
                reason: format!("ed25519 sig must be {ED25519_SIG_LEN} bytes"),
            });
        }
        match api.ed25519_verify(challenge, approval.signature.as_slice(), pk.as_slice()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(ContractError::ApprovalFailed {
                guardian: approval.guardian.clone(),
                reason: "ed25519_verify false".into(),
            }),
            Err(e) => Err(ContractError::ApprovalFailed {
                guardian: approval.guardian.clone(),
                reason: format!("ed25519_verify: {e}"),
            }),
        }
    } else if cfg.allow_address_only_approvals {
        // Digest attestation: H(guardian || 0x00 || challenge) must equal signature bytes
        let mut att_pre = approval.guardian.as_bytes().to_vec();
        att_pre.push(0);
        att_pre.extend_from_slice(challenge);
        let expected = rehash(cfg.hash_alg, &att_pre, cfg.rehash_rounds)?;
        if approval.signature.as_slice() != expected.as_slice() {
            return Err(ContractError::ApprovalFailed {
                guardian: approval.guardian.clone(),
                reason: "address-only attestation digest mismatch".into(),
            });
        }
        Ok(())
    } else {
        Err(ContractError::ApprovalFailed {
            guardian: approval.guardian.clone(),
            reason: "guardian has no pubkey and address-only approvals disabled".into(),
        })
    }
}

impl TerpAccountTrait for RecoveryAuthenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = RecoveryAuthStructs;
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
            return Err(ContractError::Unauthorized {});
        }
        Ok(Response::new().add_attribute("action", "recovery_on_auth_added"))
    }

    fn on_auth_removed(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "recovery_on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let cfg = CONFIG.load(deps.storage)?;
        let payload: RecoveryAuthPayload = cosmwasm_std::from_json(&req.signature).map_err(|e| {
            ContractError::HashAlg {
                reason: format!("payload decode: {e}"),
            }
        })?;

        // Client must echo the configured algorithm (prevents silent alg confusion).
        if payload.hash_alg != cfg.hash_alg {
            return Err(ContractError::HashAlg {
                reason: format!(
                    "payload.hash_alg {:?} != config {:?}",
                    payload.hash_alg, cfg.hash_alg
                ),
            });
        }
        if payload.rehash_rounds != cfg.rehash_rounds {
            return Err(ContractError::HashAlg {
                reason: format!(
                    "payload.rehash_rounds {} != config {}",
                    payload.rehash_rounds, cfg.rehash_rounds
                ),
            });
        }

        let challenge = break_glass_challenge(
            cfg.hash_alg,
            cfg.rehash_rounds,
            &cfg.domain,
            &req.tx_data.chain_id,
            req.account.as_str(),
            &req.authenticator_id,
            req.sign_mode_tx_data.sign_mode_direct.as_slice(),
        )?;

        let mut seen = std::collections::BTreeSet::<String>::new();
        let mut valid = 0u32;
        for ap in &payload.approvals {
            if !seen.insert(ap.guardian.clone()) {
                return Err(ContractError::DuplicateApproval {
                    guardian: ap.guardian.clone(),
                });
            }
            verify_approval(deps.api, &cfg, &challenge, ap)?;
            valid += 1;
        }

        if valid < cfg.threshold {
            return Err(ContractError::ThresholdNotMet {
                got: valid,
                need: cfg.threshold,
            });
        }

        Ok(Response::new()
            .add_attribute("action", "recovery_authenticate")
            .add_attribute("hash_alg", cfg.hash_alg.as_str())
            .add_attribute("rehash_rounds", cfg.rehash_rounds.to_string())
            .add_attribute("approvals", valid.to_string()))
    }

    fn on_auth_track(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "recovery_track"))
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "recovery_confirm"))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }
}
