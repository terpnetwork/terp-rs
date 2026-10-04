//! Deterministic P-256 passkey used by suite instantiate and authenticate tests.

use cosmwasm_std::Binary;
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::SigningKey;
use terp_passkey::{
    authenticator_data, client_data_json, PasskeyAuthPayload, PasskeyRegistration,
};

pub const PASSKEY_ORIGIN: &str = "https://app.terp.network";
pub const PASSKEY_RP_ID: &str = "app.terp.network";
pub const PASSKEY_CREDENTIAL_ID: &[u8] = b"cred-id-1";

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&p256::FieldBytes::from([0x11u8; 32])).expect("p256 seed")
}

pub fn passkey_registration() -> PasskeyRegistration {
    let sk = signing_key();
    let pk = sk.verifying_key().to_encoded_point(false);
    PasskeyRegistration {
        origin: PASSKEY_ORIGIN.into(),
        rp_id: PASSKEY_RP_ID.into(),
        credential_id: Binary::from(PASSKEY_CREDENTIAL_ID.to_vec()),
        public_key: Binary::from(pk.as_bytes().to_vec()),
    }
}

/// ES256 assertion whose WebAuthn challenge is `sign_mode_direct`.
pub fn passkey_auth_payload(sign_mode_direct: &[u8]) -> PasskeyAuthPayload {
    let sk = signing_key();
    let client = client_data_json(PASSKEY_ORIGIN, sign_mode_direct);
    let ad = authenticator_data(PASSKEY_RP_ID);
    let hash = terp_passkey::assertion_message_hash(&ad, &client);
    let sig: p256::ecdsa::Signature = sk.sign_prehash(&hash).expect("sign");
    PasskeyAuthPayload {
        origin: Some(PASSKEY_ORIGIN.into()),
        credential_id: Binary::from(PASSKEY_CREDENTIAL_ID.to_vec()),
        assertion: Binary::from(sig.to_bytes().to_vec()),
        authenticator_data: Binary::from(ad),
        client_data_json: Binary::from(client),
    }
}
