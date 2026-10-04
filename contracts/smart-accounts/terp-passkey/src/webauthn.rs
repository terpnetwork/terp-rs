//! WebAuthn ES256 checks for the passkey authenticator.
//!
//! The signed message is `authenticatorData || SHA-256(clientDataJSON)`.
//! The host checks that digest with `Api::secp256r1_verify` against the P-256
//! public key stored at registration. `clientDataJSON.challenge` must be the
//! base64url encoding of `sign_mode_direct`, and `origin` / `rpIdHash` must
//! match the registration.

use cosmwasm_std::{Api, Binary};
use sha2::{Digest, Sha256};

use crate::error::ContractError;
use crate::state::PasskeyRegistration;
use crate::PasskeyAuthPayload;

const AUTH_DATA_MIN: usize = 37;

pub fn authenticator_data(rp_id: &str) -> Vec<u8> {
    let mut out = Sha256::digest(rp_id.as_bytes()).to_vec();
    out.push(0x01); // user present
    out.extend_from_slice(&0u32.to_be_bytes());
    out
}

pub fn client_data_json(origin: &str, challenge: &[u8]) -> Vec<u8> {
    format!(
        r#"{{"type":"webauthn.get","challenge":"{}","origin":"{}"}}"#,
        b64url(challenge),
        origin
    )
    .into_bytes()
}

pub fn assertion_message_hash(authenticator_data: &[u8], client_data_json: &[u8]) -> [u8; 32] {
    let client_hash = Sha256::digest(client_data_json);
    let mut msg = Vec::with_capacity(authenticator_data.len() + 32);
    msg.extend_from_slice(authenticator_data);
    msg.extend_from_slice(&client_hash);
    Sha256::digest(&msg).into()
}

pub fn verify_passkey(
    api: &dyn Api,
    reg: &PasskeyRegistration,
    payload: &PasskeyAuthPayload,
    sign_mode_direct: &[u8],
) -> Result<(), ContractError> {
    if payload.assertion.is_empty()
        || payload.authenticator_data.is_empty()
        || payload.client_data_json.is_empty()
    {
        return Err(ContractError::InvalidCredential {
            reason: "empty assertion fields".into(),
        });
    }
    if payload.credential_id != reg.credential_id {
        return Err(ContractError::InvalidCredential {
            reason: "credential_id mismatch".into(),
        });
    }
    if reg.origin.is_empty() || reg.rp_id.is_empty() {
        return Err(ContractError::InvalidCredential {
            reason: "registration missing origin or rp_id".into(),
        });
    }
    match &payload.origin {
        Some(origin) if origin == &reg.origin => {}
        _ => return Err(ContractError::Unauthorized {}),
    }
    if reg.public_key.len() != 33 && reg.public_key.len() != 65 {
        return Err(ContractError::InvalidCredential {
            reason: "public_key must be 33 or 65 bytes".into(),
        });
    }

    let client: ClientData = cosmwasm_std::from_json(&payload.client_data_json).map_err(|e| {
        ContractError::InvalidCredential {
            reason: format!("clientDataJSON: {e}"),
        }
    })?;
    if client.ty != "webauthn.get" {
        return Err(ContractError::InvalidCredential {
            reason: "clientData type must be webauthn.get".into(),
        });
    }
    if client.origin != reg.origin {
        return Err(ContractError::Unauthorized {});
    }
    let challenge = b64url_decode(&client.challenge).map_err(|_| {
        ContractError::InvalidCredential {
            reason: "challenge is not base64url".into(),
        }
    })?;
    if challenge.as_slice() != sign_mode_direct {
        return Err(ContractError::InvalidCredential {
            reason: "challenge does not match sign_mode_direct".into(),
        });
    }
    if sign_mode_direct.is_empty() {
        return Err(ContractError::InvalidCredential {
            reason: "empty sign_mode_direct".into(),
        });
    }

    let ad = payload.authenticator_data.as_slice();
    if ad.len() < AUTH_DATA_MIN {
        return Err(ContractError::InvalidCredential {
            reason: "authenticatorData shorter than 37 bytes".into(),
        });
    }
    let rp_hash = Sha256::digest(reg.rp_id.as_bytes());
    if ad[..32] != rp_hash[..] {
        return Err(ContractError::InvalidCredential {
            reason: "rpIdHash mismatch".into(),
        });
    }
    if ad[32] & 0x01 == 0 {
        return Err(ContractError::InvalidCredential {
            reason: "user presence flag clear".into(),
        });
    }

    let hash = assertion_message_hash(ad, payload.client_data_json.as_slice());
    let sig = signature_rs(&payload.assertion)?;
    match api.secp256r1_verify(&hash, &sig, reg.public_key.as_slice()) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ContractError::InvalidCredential {
            reason: "secp256r1_verify false".into(),
        }),
        Err(e) => Err(ContractError::InvalidCredential {
            reason: format!("secp256r1_verify: {e}"),
        }),
    }
}

#[derive(serde::Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    ty: String,
    challenge: String,
    origin: String,
}

fn signature_rs(sig: &Binary) -> Result<Vec<u8>, ContractError> {
    if sig.len() == 64 {
        return Ok(sig.to_vec());
    }
    der_to_rs(sig.as_slice()).map_err(|reason| ContractError::InvalidCredential {
        reason: reason.into(),
    })
}

fn der_to_rs(sig: &[u8]) -> Result<Vec<u8>, &'static str> {
    if sig.len() < 8 || sig[0] != 0x30 {
        return Err("assertion is neither 64-byte r||s nor DER");
    }
    let mut i = 2usize;
    if sig[1] & 0x80 != 0 {
        return Err("long-form DER length");
    }
    let r = read_der_int(sig, &mut i)?;
    let s = read_der_int(sig, &mut i)?;
    if i != sig.len() {
        return Err("trailing DER bytes");
    }
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&r);
    out.extend_from_slice(&s);
    Ok(out)
}

fn read_der_int(sig: &[u8], i: &mut usize) -> Result<[u8; 32], &'static str> {
    if *i >= sig.len() || sig[*i] != 0x02 {
        return Err("DER integer tag");
    }
    *i += 1;
    if *i >= sig.len() {
        return Err("DER integer length");
    }
    let len = sig[*i] as usize;
    *i += 1;
    if len == 0 || *i + len > sig.len() {
        return Err("DER integer truncated");
    }
    let mut bytes = &sig[*i..*i + len];
    *i += len;
    if bytes.first() == Some(&0x00) {
        bytes = &bytes[1..];
    }
    if bytes.is_empty() || bytes.len() > 32 {
        return Err("DER integer width");
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(out)
}

fn b64url(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    let rest = data.len() - i;
    if rest == 1 {
        let n = (data[i] as u32) << 16;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
    } else if rest == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
    }
    out
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, ()> {
    fn val(c: u8) -> Result<u8, ()> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' | b'+' => Ok(62),
            b'_' | b'/' => Ok(63),
            _ => Err(()),
        }
    }
    let bytes = s.as_bytes();
    if bytes.iter().any(|c| *c == b'=') {
        return Err(());
    }
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut nbits = 0;
    for &c in bytes {
        buf = (buf << 6) | val(c)? as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((buf >> nbits) as u8);
            buf &= (1 << nbits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::MockApi;
    use p256::ecdsa::signature::hazmat::PrehashSigner;
    use p256::ecdsa::SigningKey;

    fn registration(pk: &[u8]) -> PasskeyRegistration {
        PasskeyRegistration {
            origin: "https://app.terp.network".into(),
            rp_id: "app.terp.network".into(),
            credential_id: Binary::from(b"cred-id-1"),
            public_key: Binary::from(pk.to_vec()),
        }
    }

    fn signed(msg: &[u8]) -> (PasskeyRegistration, PasskeyAuthPayload) {
        let sk = SigningKey::from_bytes(&p256::FieldBytes::from([0x11u8; 32])).unwrap();
        let pk = sk.verifying_key().to_encoded_point(false);
        let reg = registration(pk.as_bytes());
        let client = client_data_json(&reg.origin, msg);
        let ad = authenticator_data(&reg.rp_id);
        let hash = assertion_message_hash(&ad, &client);
        let sig: p256::ecdsa::Signature = sk.sign_prehash(&hash).unwrap();
        let payload = PasskeyAuthPayload {
            origin: Some(reg.origin.clone()),
            credential_id: reg.credential_id.clone(),
            assertion: Binary::from(sig.to_bytes().to_vec()),
            authenticator_data: Binary::from(ad),
            client_data_json: Binary::from(client),
        };
        (reg, payload)
    }

    #[test]
    fn es256_accepts_bound_challenge() {
        let api = MockApi::default();
        let (reg, payload) = signed(b"sign-mode-direct");
        verify_passkey(&api, &reg, &payload, b"sign-mode-direct").unwrap();
    }

    #[test]
    fn es256_rejects_tampered_message() {
        let api = MockApi::default();
        let (reg, payload) = signed(b"sign-mode-direct");
        let err = verify_passkey(&api, &reg, &payload, b"sign-mode-direcu").unwrap_err();
        assert!(matches!(err, ContractError::InvalidCredential { .. }));
    }
}
