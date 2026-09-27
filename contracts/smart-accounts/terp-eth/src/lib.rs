//! Ethereum personal_sign authenticator.
//!
//! Verifies EIP-191 personal signatures over `sign_mode_direct` using secp256k1
//! recovery and compares the recovered address to the registered signer.

mod error;

pub use error::ContractError;

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use sha3::{Digest, Keccak256};
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::AuthSudoMsg;

const CONTRACT_NAME: &str = "crates.io:terp-eth";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Lowercase hex address without 0x, 40 chars — or full 0x-prefixed stored as-is normalized.
pub const SIGNER: Item<String> = Item::new("signer");

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    /// Ethereum address (0x…) that may authorize.
    pub signer: String,
}

#[cw_serde]
pub enum ExecuteMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(String)]
    Signer {},
}

pub type SudoMsg = AuthSudoMsg;

pub struct EthAuthenticator;
#[derive(Clone, Debug)]
pub struct EthAuthStructs {}

fn normalize_addr(s: &str) -> Result<String, ContractError> {
    let t = s.trim();
    let hex = t.strip_prefix("0x").unwrap_or(t).to_lowercase();
    if hex.len() != 40 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ContractError::InvalidAddress {});
    }
    Ok(format!("0x{hex}"))
}

fn eth_personal_hash(message: &[u8]) -> [u8; 32] {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

fn recover_eth_address(deps: Deps, message: &[u8], signature: &[u8]) -> Result<String, ContractError> {
    if signature.len() != 65 {
        return Err(ContractError::BadSignature {});
    }
    let hash = eth_personal_hash(message);
    // CosmWasm expects 64-byte compact sig + recovery byte separate in some APIs.
    // secp256k1_recover_pubkey(hash, sig, recovery)
    let recovery = signature[64];
    // eth signatures often use v = 27/28
    let rec = if recovery >= 27 { recovery - 27 } else { recovery };
    let pubkey = deps
        .api
        .secp256k1_recover_pubkey(&hash, &signature[..64], rec)
        .map_err(|_| ContractError::BadSignature {})?;
    // address = last 20 bytes of keccak(uncompressed pubkey without 0x04)
    let pk = if pubkey.len() == 65 && pubkey[0] == 0x04 {
        &pubkey[1..]
    } else {
        pubkey.as_slice()
    };
    let mut hasher = Keccak256::new();
    hasher.update(pk);
    let dig = hasher.finalize();
    let addr = hex::encode(&dig[12..]);
    Ok(format!("0x{addr}"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let signer = normalize_addr(&msg.signer)?;
    let owner = match msg.owner {
        Some(o) => deps.api.addr_validate(&o)?,
        None => info.sender,
    };
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(owner.as_str()))?;
    SIGNER.save(deps.storage, &signer)?;
    Ok(Response::new().add_attribute("action", "eth_instantiate"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Signer {} => cosmwasm_std::to_json_binary(&SIGNER.load(deps.storage)?),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    EthAuthenticator::process_sudo_auth(deps, env, &msg)
}

impl TerpAccountTrait for EthAuthenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = EthAuthStructs;
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
        Ok(Response::new().add_attribute("action", "eth_on_auth_added"))
    }

    fn on_auth_removed(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "eth_on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let expected = SIGNER.load(deps.storage)?;
        let recovered = recover_eth_address(
            deps.as_ref(),
            req.sign_mode_tx_data.sign_mode_direct.as_slice(),
            req.signature.as_slice(),
        )?;
        if recovered != expected {
            return Err(ContractError::Unauthorized {});
        }
        Ok(Response::new().add_attribute("action", "eth_authenticate"))
    }

    fn on_auth_track(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "eth_track"))
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "eth_confirm"))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }
}
