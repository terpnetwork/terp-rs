//! Read-only Schnorr authorization against the applied `group_vk`.
//!
//! Rotation stays on `ExecuteMsg`. This module never writes. `x/smart-account`
//! discards writes made from `Authenticate`, so a rotate inside sudo would not
//! land. A pending proposal does not authorize; only the stored `group_vk` does.
//!
//! osst's `Signature::to_bytes` is 64 bytes and, on secp256k1, keeps only the
//! x coordinate. `decompress` then assumes y parity `0x02`, so an odd-y `R`
//! does not round-trip. The wire signature is therefore 33-byte SEC1 `R`
//! followed by the 32-byte canonical scalar `z`. Verification still calls
//! `osst::frost::verify_signature`.

#[cfg(test)]
use cosmwasm_std::Binary;
use cosmwasm_std::{DepsMut, Env, Response};
use osst::Secp256k1Curve;
use osst::curve::{OsstCurve, OsstPoint, OsstScalar};
use osst::frost::Signature;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::{
    AuthSudoMsg, AuthenticationRequest, ConfirmExecutionRequest, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};

use crate::CONFIG;
use crate::error::ContractError;

pub const FROST_R_LEN: usize = 33;
pub const FROST_Z_LEN: usize = 32;
pub const FROST_SIG_LEN: usize = FROST_R_LEN + FROST_Z_LEN;

pub type Point = <Secp256k1Curve as OsstCurve>::Point;
pub type Scalar = <Secp256k1Curve as OsstCurve>::Scalar;

pub struct FrostAuthenticator;

pub fn process_sudo(deps: DepsMut, env: Env, msg: &AuthSudoMsg) -> Result<Response, ContractError> {
    FrostAuthenticator::process_sudo_auth(deps, env, msg)
}

pub fn encode_group_vk(point: &Point) -> String {
    encode_hex(&point.compress_vec())
}

pub fn encode_signature(signature: &Signature<Point>) -> [u8; FROST_SIG_LEN] {
    let r = signature.r.compress_vec();
    assert!(
        r.len() == FROST_R_LEN,
        "secp256k1 compressed R must be {FROST_R_LEN} bytes, got {}",
        r.len()
    );
    let mut out = [0u8; FROST_SIG_LEN];
    out[..FROST_R_LEN].copy_from_slice(&r);
    out[FROST_R_LEN..].copy_from_slice(&signature.z.to_bytes());
    out
}

pub fn verify_group_signature(
    group_vk: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<(), ContractError> {
    if message.is_empty() {
        return Err(ContractError::EmptySignDoc);
    }
    let group_bytes = decode_point_hex(group_vk).ok_or_else(|| ContractError::GroupVkNotPoint {
        group_vk: group_vk.to_string(),
    })?;
    let group =
        Point::decompress_slice(&group_bytes).ok_or_else(|| ContractError::GroupVkNotPoint {
            group_vk: group_vk.to_string(),
        })?;
    if signature.len() != FROST_SIG_LEN {
        return Err(ContractError::BadSignatureLength {
            got: signature.len(),
            need: FROST_SIG_LEN,
        });
    }
    let r = Point::decompress_slice(&signature[..FROST_R_LEN])
        .ok_or(ContractError::SignatureRNotPoint)?;
    let mut z_bytes = [0u8; FROST_Z_LEN];
    z_bytes.copy_from_slice(&signature[FROST_R_LEN..]);
    let z = Scalar::from_canonical_bytes(&z_bytes).ok_or(ContractError::SignatureZNotScalar)?;
    let parsed = Signature { r, z };
    if osst::frost::verify_signature(&group, message, &parsed) {
        Ok(())
    } else {
        Err(ContractError::SignatureRejected {
            group_vk: group_vk.to_string(),
            message_len: message.len(),
        })
    }
}

fn authenticate(deps: DepsMut, req: &AuthenticationRequest) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if req.simulate {
        return Ok(Response::new()
            .add_attribute("action", "frost_authenticate")
            .add_attribute("simulate", "true")
            .add_attribute("group_vk", cfg.group_vk));
    }
    verify_group_signature(
        &cfg.group_vk,
        req.sign_mode_tx_data.sign_mode_direct.as_slice(),
        req.signature.as_slice(),
    )?;
    Ok(Response::new()
        .add_attribute("action", "frost_authenticate")
        .add_attribute("group_vk", &cfg.group_vk))
}

fn require_point(deps: DepsMut) -> Result<crate::Config, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let bytes = decode_point_hex(&cfg.group_vk).ok_or_else(|| ContractError::GroupVkNotPoint {
        group_vk: cfg.group_vk.clone(),
    })?;
    if Point::decompress_slice(&bytes).is_none() {
        return Err(ContractError::GroupVkNotPoint {
            group_vk: cfg.group_vk.clone(),
        });
    }
    Ok(cfg)
}

fn decode_point_hex(group_vk: &str) -> Option<Vec<u8>> {
    if group_vk.len() != FROST_R_LEN * 2 {
        return None;
    }
    let mut out = Vec::with_capacity(FROST_R_LEN);
    let bytes = group_vk.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const H: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0xf) as usize] as char);
    }
    s
}

impl TerpAccountTrait for FrostAuthenticator {
    type InstantiateMsg = crate::InstantiateMsg;
    type ExecuteMsg = crate::ExecuteMsg;
    type QueryMsg = crate::QueryMsg;
    type SudoMsg = AuthSudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = ();
    type AuthProcessResult = Result<Response, ContractError>;

    fn extended_authenticate(
        _deps: DepsMut,
        _auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "frost_extended_authenticate"))
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
        req: &OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        let cfg = require_point(deps)?;
        if let Some(params) = &req.authenticator_params {
            let bytes =
                decode_point_hex(&cfg.group_vk).ok_or_else(|| ContractError::GroupVkNotPoint {
                    group_vk: cfg.group_vk.clone(),
                })?;
            if params.as_slice() != bytes.as_slice() {
                return Err(ContractError::AuthenticatorParamsMismatch {
                    group_vk: cfg.group_vk,
                });
            }
        }
        Ok(Response::new()
            .add_attribute("action", "frost_on_auth_added")
            .add_attribute("group_vk", cfg.group_vk))
    }

    fn on_auth_removed(
        deps: DepsMut,
        _env: Env,
        _req: &OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        let cfg = CONFIG.load(deps.storage)?;
        Ok(Response::new()
            .add_attribute("action", "frost_on_auth_removed")
            .add_attribute("group_vk", cfg.group_vk))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        authenticate(deps, req)
    }

    fn on_auth_track(deps: DepsMut, _env: Env, _req: &TrackRequest) -> Self::AuthProcessResult {
        let cfg = CONFIG.load(deps.storage)?;
        Ok(Response::new()
            .add_attribute("action", "frost_track")
            .add_attribute("group_vk", cfg.group_vk))
    }

    fn on_auth_confirm(
        deps: DepsMut,
        _env: Env,
        _req: &ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        let cfg = CONFIG.load(deps.storage)?;
        Ok(Response::new()
            .add_attribute("action", "frost_confirm")
            .add_attribute("group_vk", cfg.group_vk))
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "frost_hooks"))
    }
}

#[cfg(test)]
pub fn auth_request(
    account: &cosmwasm_std::Addr,
    message: &[u8],
    signature: &[u8],
    simulate: bool,
) -> AuthenticationRequest {
    use terp_auth::{Any, SignModeTxData, SignatureData, TxData};
    AuthenticationRequest {
        authenticator_id: "frost".into(),
        account: account.clone(),
        fee_payer: account.clone(),
        fee_granter: None,
        fee: vec![],
        msg: Any {
            type_url: "/terp.frost.Auth".into(),
            value: Binary::from(message),
        },
        msg_index: 0,
        signature: Binary::from(signature),
        sign_mode_tx_data: SignModeTxData {
            sign_mode_direct: Binary::from(message),
            sign_mode_textual: None,
        },
        tx_data: TxData {
            chain_id: "terp-local".into(),
            account_number: 1,
            sequence: 1,
            timeout_height: 0,
            msgs: vec![],
            memo: String::new(),
        },
        signature_data: SignatureData {
            signers: vec![account.clone()],
            signatures: vec![Binary::from(signature)],
        },
        simulate,
        authenticator_params: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExecuteMsg, InstantiateMsg, QueryMsg, StatusResponse, execute, instantiate, query, sudo,
        vk_commit,
    };
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{Coin, Order, Storage, from_json};
    use osst::frost::{Signature, SigningPackage};
    use osst::{OsstScalar, SecretShare};
    use rand::rngs::OsRng;

    type TestDeps = cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >;

    fn storage_dump(storage: &dyn Storage) -> Vec<(Vec<u8>, Vec<u8>)> {
        storage.range(None, None, Order::Ascending).collect()
    }

    fn status(deps: &TestDeps) -> StatusResponse {
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::Status {}).unwrap()).unwrap()
    }

    fn fresh_scalar(rng: &mut OsRng) -> Scalar {
        Scalar::random(rng)
    }

    fn share_at(secret: &Scalar, coefficient: Option<&Scalar>, index: u32) -> SecretShare<Scalar> {
        let y = match coefficient {
            None => secret.clone(),
            Some(c) => secret.add(&c.mul(&Scalar::from_u32(index))),
        };
        SecretShare::new(index, y)
    }

    fn group_of(secret: &Scalar) -> Point {
        <Point as OsstPoint>::generator().mul_scalar(secret)
    }

    fn sign_with(
        shares: Vec<SecretShare<Scalar>>,
        group: &Point,
        message: &[u8],
    ) -> Signature<Point> {
        let mut rng = OsRng;
        let mut nonces = Vec::with_capacity(shares.len());
        let mut commits = Vec::with_capacity(shares.len());
        for share in &shares {
            let (nonce, commit) = osst::frost::commit::<Point, _>(share.index, &mut rng);
            nonces.push(nonce);
            commits.push(commit);
        }
        let package = SigningPackage::new(message.to_vec(), commits).expect("signing package");
        let parts: Vec<_> = shares
            .iter()
            .zip(nonces)
            .map(|(share, nonce)| {
                osst::frost::sign(&package, nonce, share, group).expect("signature share")
            })
            .collect();
        osst::frost::aggregate(&package, &parts, group, None).expect("aggregate")
    }

    struct Key {
        group_vk: String,
        shares: Vec<SecretShare<Scalar>>,
        group: Point,
    }

    fn key_1_of_1() -> Key {
        let mut rng = OsRng;
        let secret = fresh_scalar(&mut rng);
        let group = group_of(&secret);
        Key {
            group_vk: encode_group_vk(&group),
            shares: vec![share_at(&secret, None, 1)],
            group,
        }
    }

    fn key_2_of_3() -> Key {
        let mut rng = OsRng;
        let secret = fresh_scalar(&mut rng);
        let coefficient = fresh_scalar(&mut rng);
        let group = group_of(&secret);
        Key {
            group_vk: encode_group_vk(&group),
            shares: vec![
                share_at(&secret, Some(&coefficient), 1),
                share_at(&secret, Some(&coefficient), 2),
                share_at(&secret, Some(&coefficient), 3),
            ],
            group,
        }
    }

    fn boot(group_vk: &str, t: u16, n: u16, seats: Vec<String>) -> (TestDeps, cosmwasm_std::Addr) {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[] as &[Coin]),
            InstantiateMsg {
                group_vk: group_vk.into(),
                t,
                n,
                seats,
            },
        )
        .unwrap();
        (deps, owner)
    }

    fn auth(
        deps: &mut TestDeps,
        account: &cosmwasm_std::Addr,
        message: &[u8],
        signature: &[u8],
        simulate: bool,
    ) -> Result<Response, ContractError> {
        sudo(
            deps.as_mut(),
            mock_env(),
            AuthSudoMsg::Authenticate(Box::new(auth_request(
                account, message, signature, simulate,
            ))),
        )
    }

    #[test]
    fn one_of_one_signature_authorizes_exact_preimage() {
        let key = key_1_of_1();
        let message = b"sign-doc-direct-bytes";
        let signature = encode_signature(&sign_with(key.shares, &key.group, message));
        let (mut deps, owner) = boot(&key.group_vk, 1, 1, vec!["desk".into()]);
        let before = storage_dump(&deps.storage);
        let res = auth(&mut deps, &owner, message, &signature, false).unwrap();
        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "action")
                .map(|a| a.value.as_str()),
            Some("frost_authenticate")
        );
        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "group_vk")
                .map(|a| a.value.as_str()),
            Some(key.group_vk.as_str())
        );
        assert_eq!(storage_dump(&deps.storage), before);
    }

    #[test]
    fn two_of_three_authorizes_and_one_share_does_not() {
        let key = key_2_of_3();
        let message = b"quorum-sign-doc";
        let good = encode_signature(&sign_with(
            vec![key.shares[0].clone(), key.shares[1].clone()],
            &key.group,
            message,
        ));
        let short = encode_signature(&sign_with(vec![key.shares[0].clone()], &key.group, message));
        let (mut deps, owner) = boot(
            &key.group_vk,
            2,
            3,
            vec!["desk".into(), "steward".into(), "spare".into()],
        );
        auth(&mut deps, &owner, message, &good, false).unwrap();
        let err = auth(&mut deps, &owner, message, &short, false).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "frost signature rejected for group_vk {} message_len {}",
                key.group_vk,
                message.len()
            )
        );
    }

    #[test]
    fn wrong_preimage_wrong_length_and_bad_point_are_specific() {
        let key = key_1_of_1();
        let message = b"sign-doc-direct-bytes";
        let signature = encode_signature(&sign_with(key.shares, &key.group, message));
        let (mut deps, owner) = boot(&key.group_vk, 1, 1, vec!["desk".into()]);
        let err = auth(&mut deps, &owner, b"other-sign-doc", &signature, false).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "frost signature rejected for group_vk {} message_len {}",
                key.group_vk,
                "other-sign-doc".len()
            )
        );
        let err = auth(&mut deps, &owner, message, &signature[..64], false).unwrap_err();
        assert_eq!(err.to_string(), "frost signature length 64, need 65");
        let mut bad_r = signature;
        bad_r[0] = 0x00;
        let err = auth(&mut deps, &owner, message, &bad_r, false).unwrap_err();
        assert_eq!(
            err.to_string(),
            "frost signature R is not a secp256k1 point"
        );
        let mut bad_z = signature;
        bad_z[FROST_R_LEN..].copy_from_slice(&[0xff; FROST_Z_LEN]);
        let err = auth(&mut deps, &owner, message, &bad_z, false).unwrap_err();
        assert_eq!(
            err.to_string(),
            "frost signature z is not a canonical scalar"
        );
        let err = auth(&mut deps, &owner, b"", &signature, false).unwrap_err();
        assert_eq!(err.to_string(), "empty sign doc");
    }

    #[test]
    fn opaque_group_vk_fails_auth_and_still_rotates() {
        let (mut deps, owner) = boot(
            "vk-old",
            2,
            3,
            vec!["desk".into(), "steward".into(), "spare".into()],
        );
        let info = message_info(&owner, &[] as &[Coin]);
        let err = auth(&mut deps, &owner, b"msg", &[0u8; 65], false).unwrap_err();
        assert_eq!(err.to_string(), "group_vk is not a secp256k1 point: vk-old");
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-new"),
                hkdf_commit: None,
                next_seats: None,
                next_t: None,
                next_n: None,
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: None,
                blob: "S:0:a".into(),
                admin_id: Some("desk".into()),
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: None,
                blob: "S:0:b".into(),
                admin_id: Some("steward".into()),
            },
        )
        .unwrap();
        assert_eq!(status(&deps).group_vk, "vk-new");
        assert_eq!(status(&deps).old_vk.as_deref(), Some("vk-old"));
    }

    #[test]
    fn rotation_switches_the_authorizing_key_and_pending_does_not() {
        let current = key_1_of_1();
        let next = key_1_of_1();
        let message = b"rotate-then-sign";
        let sig_current = encode_signature(&sign_with(current.shares, &current.group, message));
        let sig_next = encode_signature(&sign_with(next.shares, &next.group, message));
        let (mut deps, owner) = boot(&current.group_vk, 1, 1, vec!["desk".into()]);
        auth(&mut deps, &owner, message, &sig_current, false).unwrap();

        let info = message_info(&owner, &[] as &[Coin]);
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit(&next.group_vk),
                hkdf_commit: None,
                next_seats: None,
                next_t: None,
                next_n: None,
            },
        )
        .unwrap();
        assert!(status(&deps).pending);
        auth(&mut deps, &owner, message, &sig_current, false).unwrap();
        let err = auth(&mut deps, &owner, message, &sig_next, false).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("frost signature rejected for group_vk ")
        );
        assert!(err.to_string().contains(&current.group_vk));

        let before_apply_auth = storage_dump(&deps.storage);
        execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: next.group_vk.clone(),
                hkdf_commit: None,
                blob: "S:0:a".into(),
                admin_id: Some("desk".into()),
            },
        )
        .unwrap();
        assert_eq!(status(&deps).group_vk, next.group_vk);
        assert_eq!(
            status(&deps).old_vk.as_deref(),
            Some(current.group_vk.as_str())
        );
        assert!(!status(&deps).pending);

        let err = auth(&mut deps, &owner, message, &sig_current, false).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "frost signature rejected for group_vk {} message_len {}",
                next.group_vk,
                message.len()
            )
        );
        let before = storage_dump(&deps.storage);
        auth(&mut deps, &owner, message, &sig_next, false).unwrap();
        assert_eq!(storage_dump(&deps.storage), before);
        assert_ne!(storage_dump(&deps.storage), before_apply_auth);
    }

    #[test]
    fn simulate_add_and_remove_do_not_write() {
        let key = key_1_of_1();
        let message = b"sign-doc-direct-bytes";
        let signature = encode_signature(&sign_with(key.shares, &key.group, message));
        let (mut deps, owner) = boot(&key.group_vk, 1, 1, vec!["desk".into()]);
        let before = storage_dump(&deps.storage);
        let res = auth(&mut deps, &owner, message, &[], true).unwrap();
        assert!(
            res.attributes
                .iter()
                .any(|a| a.key == "simulate" && a.value == "true")
        );
        assert_eq!(storage_dump(&deps.storage), before);

        sudo(
            deps.as_mut(),
            mock_env(),
            AuthSudoMsg::OnAuthAdded(OnAuthenticatorAddedRequest {
                account: owner.clone(),
                authenticator_id: "frost".into(),
                authenticator_params: Some(Binary::from(decode_point_hex(&key.group_vk).unwrap())),
            }),
        )
        .unwrap();
        assert_eq!(storage_dump(&deps.storage), before);

        let err = sudo(
            deps.as_mut(),
            mock_env(),
            AuthSudoMsg::OnAuthAdded(OnAuthenticatorAddedRequest {
                account: owner.clone(),
                authenticator_id: "frost".into(),
                authenticator_params: Some(Binary::from(vec![0x02; 33])),
            }),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "authenticator params are not the current group point for group_vk {}",
                key.group_vk
            )
        );
        assert_eq!(storage_dump(&deps.storage), before);

        sudo(
            deps.as_mut(),
            mock_env(),
            AuthSudoMsg::OnAuthRemoved(OnAuthenticatorRemovedRequest {
                account: owner.clone(),
                authenticator_id: "frost".into(),
                authenticator_params: None,
            }),
        )
        .unwrap();
        assert_eq!(storage_dump(&deps.storage), before);
        auth(&mut deps, &owner, message, &signature, false).unwrap();
        assert_eq!(status(&deps).group_vk, key.group_vk);
    }

    #[test]
    fn uppercase_group_vk_is_rejected() {
        let key = key_1_of_1();
        let upper = key.group_vk.to_ascii_uppercase();
        let (mut deps, owner) = boot(&upper, 1, 1, vec!["desk".into()]);
        let err = auth(&mut deps, &owner, b"msg", &[0u8; 65], false).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("group_vk is not a secp256k1 point: {upper}")
        );
    }
}
