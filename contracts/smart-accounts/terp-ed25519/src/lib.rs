//! # terp-ed25519
//!
//! CosmWasm authenticator verifying Ed25519 signatures via **host VM imports**:
//!
//! | Host API | Use |
//! |----------|-----|
//! | [`Api::ed25519_verify`](cosmwasm_std::Api::ed25519_verify) | Single signer |
//! | [`Api::ed25519_batch_verify`](cosmwasm_std::Api::ed25519_batch_verify) | Multi-sig / multi-message |
//!
//! ## Message binding
//!
//! Default signed message is `AuthenticationRequest.sign_mode_tx_data.sign_mode_direct`
//! (raw tx bytes as provided by x/smart-account). This matches Tendermint/Cosmos
//! ed25519 usage: **sign the preimage, not a separate digest** (ed25519 internal hash).
//!
//! ## Signature encoding
//!
//! - **Single:** `signature` field = 64 raw bytes  
//! - **Batch / multi-sig:** `signature` = JSON [`Ed25519AuthPayload`]
//!
//! See [`crypto`] for length rules and CosmWasm batch shapes.

mod crypto;
mod error;

pub use crypto::{
    ED25519_PUBKEY_LEN, ED25519_SIGNATURE_LEN, Ed25519AuthPayload, ParsedAuth,
    parse_auth_signature, require_pubkey, verify_batch, verify_single,
};
pub use error::ContractError;

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};
use cw_storage_plus::Item;
use cw2::set_contract_version;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::AuthSudoMsg;

const CONTRACT_NAME: &str = "crates.io:terp-ed25519";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Primary registered verification key (32 bytes).
pub const PUBLIC_KEY: Item<Binary> = Item::new("pk");

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    /// 32-byte ed25519 public key.
    pub pubkey: Binary,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Rotate the registered pubkey (owner only).
    UpdatePubkey { pubkey: Binary },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Binary)]
    Pubkey {},
}

pub type SudoMsg = AuthSudoMsg;

pub struct Ed25519Authenticator;
#[derive(Clone, Debug)]
pub struct Ed25519AuthStructs {}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    crypto::require_pubkey(msg.pubkey.as_slice())?;
    let owner = match msg.owner {
        Some(o) => deps.api.addr_validate(&o)?,
        None => info.sender,
    };
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(owner.as_str()))?;
    PUBLIC_KEY.save(deps.storage, &msg.pubkey)?;
    Ok(Response::new()
        .add_attribute("action", "ed25519_instantiate")
        .add_attribute("mode", "host_ed25519_verify"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdatePubkey { pubkey } => {
            cw_ownable::assert_owner(deps.storage, &info.sender)?;
            crypto::require_pubkey(pubkey.as_slice())?;
            PUBLIC_KEY.save(deps.storage, &pubkey)?;
            let _ = env;
            Ok(Response::new().add_attribute("action", "ed25519_update_pubkey"))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Pubkey {} => cosmwasm_std::to_json_binary(&PUBLIC_KEY.load(deps.storage)?),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    Ed25519Authenticator::process_sudo_auth(deps, env, &msg)
}

impl TerpAccountTrait for Ed25519Authenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = Ed25519AuthStructs;
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
        deps: DepsMut,
        _env: Env,
        req: &terp_auth::OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        let Some(params) = &req.authenticator_params else {
            return Err(ContractError::Unauthorized {});
        };
        crypto::require_pubkey(params.as_slice())?;
        match PUBLIC_KEY.may_load(deps.storage)? {
            Some(pk) if &pk != params => {
                return Err(ContractError::Unauthorized {});
            }
            None => PUBLIC_KEY.save(deps.storage, params)?,
            _ => {}
        }
        Ok(Response::new().add_attribute("action", "ed25519_on_auth_added"))
    }

    fn on_auth_removed(
        deps: DepsMut,
        _env: Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        PUBLIC_KEY.remove(deps.storage);
        Ok(Response::new().add_attribute("action", "ed25519_on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let registered = PUBLIC_KEY.load(deps.storage)?;
        let default_msg = req.sign_mode_tx_data.sign_mode_direct.as_slice();

        match parse_auth_signature(&req.signature)? {
            ParsedAuth::Single { signature } => {
                verify_single(
                    deps.api,
                    default_msg,
                    signature.as_slice(),
                    registered.as_slice(),
                )?;
                Ok(Response::new()
                    .add_attribute("action", "ed25519_authenticate")
                    .add_attribute("verify", "ed25519_verify")
                    .add_attribute("batch_size", "1"))
            }
            ParsedAuth::Batch(payload) => {
                // Build host batch slices — all & references live until verify_batch returns.
                let n = payload.signatures.len();
                let sig_refs: Vec<&[u8]> =
                    payload.signatures.iter().map(|s| s.as_slice()).collect();

                let pk_owned: Vec<Binary> = match &payload.public_keys {
                    Some(pks) => {
                        if pks.len() != n && pks.len() != 1 {
                            return Err(ContractError::BadSignature {
                                reason: format!(
                                    "public_keys len {} incompatible with signatures len {n}",
                                    pks.len()
                                ),
                            });
                        }
                        pks.clone()
                    }
                    None => {
                        // Expand registered key to N copies → host shape (1 msg, N sig, N key)
                        // or (N msg, N sig, N key)
                        vec![registered.clone(); n]
                    }
                };
                let pk_refs: Vec<&[u8]> = if pk_owned.len() == 1 && n > 1 {
                    // Host shape (n,n,1): one key for all — keep single ref
                    vec![pk_owned[0].as_slice()]
                } else if pk_owned.len() == n {
                    pk_owned.iter().map(|p| p.as_slice()).collect()
                } else {
                    return Err(ContractError::BadSignature {
                        reason: "public_keys length invalid".into(),
                    });
                };

                let msg_owned: Vec<Binary> = match &payload.messages {
                    Some(msgs) => msgs.clone(),
                    None => vec![Binary::from(default_msg.to_vec())],
                };
                let msg_refs: Vec<&[u8]> = if msg_owned.len() == 1 {
                    vec![msg_owned[0].as_slice()]
                } else {
                    msg_owned.iter().map(|m| m.as_slice()).collect()
                };

                // When public_keys expanded to N and messages is 1: shape (1,n,n) ✓
                // When both expanded: if messages was None we have len-1 msg → (1,n,n) ✓
                // If user supplied N messages and 1 key: shape (n,n,1) ✓
                let pk_for_batch: Vec<&[u8]> = if pk_refs.len() == 1 {
                    pk_refs
                } else if msg_refs.len() == 1 && pk_refs.len() == n {
                    pk_refs
                } else {
                    pk_refs
                };

                verify_batch(deps.api, &msg_refs, &sig_refs, &pk_for_batch)?;
                Ok(Response::new()
                    .add_attribute("action", "ed25519_authenticate")
                    .add_attribute("verify", "ed25519_batch_verify")
                    .add_attribute("batch_size", n.to_string()))
            }
        }
    }

    fn on_auth_track(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "ed25519_track"))
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "ed25519_confirm"))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new())
    }
}

#[cfg(test)]
mod tests {
    //! Golden vectors from cosmwasm-std MockApi tests (RFC-aligned fixtures).
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{Binary, Coin};

    // Same fixtures as packages/std/src/testing/mock.rs
    const ED25519_MSG_HEX: &str = "72";
    const ED25519_SIG_HEX: &str = "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00";
    const ED25519_PUBKEY_HEX: &str =
        "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";

    fn fixtures() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            hex::decode(ED25519_MSG_HEX).unwrap(),
            hex::decode(ED25519_SIG_HEX).unwrap(),
            hex::decode(ED25519_PUBKEY_HEX).unwrap(),
        )
    }

    #[test]
    fn host_ed25519_verify_golden() {
        let deps = mock_dependencies();
        let (msg, sig, pk) = fixtures();
        verify_single(deps.as_ref().api, &msg, &sig, &pk).unwrap();
    }

    #[test]
    fn host_ed25519_verify_rejects_tampered_msg() {
        let deps = mock_dependencies();
        let (mut msg, sig, pk) = fixtures();
        msg[0] ^= 0x01;
        assert!(verify_single(deps.as_ref().api, &msg, &sig, &pk).is_err());
    }

    #[test]
    fn host_ed25519_batch_golden_single_element() {
        let deps = mock_dependencies();
        let (msg, sig, pk) = fixtures();
        let msgs: &[&[u8]] = &[&msg];
        let sigs: &[&[u8]] = &[&sig];
        let pks: &[&[u8]] = &[&pk];
        verify_batch(deps.as_ref().api, msgs, sigs, pks).unwrap();
    }

    #[test]
    fn host_ed25519_batch_one_msg_two_identical_signers() {
        // Shape (1, n, n): same message, two copies of the same valid signature/key
        let deps = mock_dependencies();
        let (msg, sig, pk) = fixtures();
        let msgs: &[&[u8]] = &[&msg];
        let sigs: &[&[u8]] = &[&sig, &sig];
        let pks: &[&[u8]] = &[&pk, &pk];
        verify_batch(deps.as_ref().api, msgs, sigs, pks).unwrap();
    }

    #[test]
    fn contract_authenticate_single_via_sudo_flow() {
        let mut deps = mock_dependencies();
        let (msg, sig, pk) = fixtures();
        let owner = deps.api.addr_make("owner");
        let info = message_info(&owner, &[] as &[Coin]);
        instantiate(
            deps.as_mut(),
            mock_env(),
            info,
            InstantiateMsg {
                owner: None,
                pubkey: Binary::from(pk.clone()),
            },
        )
        .unwrap();

        let req = terp_auth::AuthenticationRequest {
            authenticator_id: "1".into(),
            account: owner.clone(),
            fee_payer: owner.clone(),
            fee_granter: None,
            fee: vec![],
            msg: terp_auth::Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
                value: Binary::from(b"\x00"),
            },
            msg_index: 0,
            signature: Binary::from(sig),
            sign_mode_tx_data: terp_auth::SignModeTxData {
                sign_mode_direct: Binary::from(msg),
                sign_mode_textual: None,
            },
            tx_data: terp_auth::TxData {
                chain_id: "test".into(),
                account_number: 0,
                sequence: 0,
                timeout_height: 0,
                msgs: vec![],
                memo: String::new(),
            },
            signature_data: terp_auth::SignatureData {
                signers: vec![],
                signatures: vec![],
            },
            simulate: false,
            authenticator_params: None,
        };

        let res = Ed25519Authenticator::on_auth_request(deps.as_mut(), mock_env(), &Box::new(req))
            .unwrap();
        assert!(res.attributes.iter().any(|a| a.value == "ed25519_verify"));
    }
}
