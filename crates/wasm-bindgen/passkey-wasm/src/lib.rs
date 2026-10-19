use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::Serialize;
use wasm_bindgen::prelude::*;

// ── Types ────────────────────────────────────────────────────────────

#[derive(Serialize, Debug)]
struct RpEntity {
    name: String,
    id: String,
}

#[derive(Serialize, Debug)]
struct UserEntity {
    id: String,
    name: String,
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Serialize, Debug)]
struct PubKeyCredParam {
    #[serde(rename = "type")]
    cred_type: String,
    alg: i32,
}

#[derive(Serialize, Debug)]
struct AuthenticatorSelection {
    #[serde(rename = "requireResidentKey")]
    require_resident_key: bool,
    #[serde(rename = "residentKey")]
    resident_key: String,
    #[serde(rename = "userVerification")]
    user_verification: String,
}

#[derive(Serialize, Debug)]
struct CreateOptions {
    challenge: String,
    rp: RpEntity,
    user: UserEntity,
    #[serde(rename = "pubKeyCredParams")]
    pub_key_cred_params: Vec<PubKeyCredParam>,
    #[serde(rename = "authenticatorSelection")]
    authenticator_selection: AuthenticatorSelection,
    timeout: u32,
    attestation: String,
}

#[derive(Serialize, Debug)]
struct AllowCredential {
    #[serde(rename = "type")]
    cred_type: String,
    id: String,
}

#[derive(Serialize, Debug)]
struct GetOptions {
    challenge: String,
    #[serde(rename = "rpId")]
    rp_id: String,
    #[serde(rename = "allowCredentials")]
    allow_credentials: Vec<AllowCredential>,
    #[serde(rename = "userVerification")]
    user_verification: String,
    timeout: u32,
}

#[derive(Serialize, Debug)]
struct AttestationEncoded {
    attestation_b64: String,
    client_data_b64: String,
}

#[derive(Serialize, Debug)]
struct AssertionEncoded {
    authenticator_data_b64: String,
    client_data_b64: String,
    signature_b64: String,
}

// ── Internal builders (testable on native) ───────────────────────────

fn make_create_options(
    rp_id: &str,
    rp_name: &str,
    user_id_b64: &str,
    user_name: &str,
    display_name: &str,
    challenge_b64: &str,
) -> Result<CreateOptions, String> {
    B64.decode(challenge_b64)
        .map_err(|e| format!("bad challenge: {e}"))?;
    B64.decode(user_id_b64)
        .map_err(|e| format!("bad user_id: {e}"))?;

    Ok(CreateOptions {
        challenge: challenge_b64.to_string(),
        rp: RpEntity {
            name: rp_name.to_string(),
            id: rp_id.to_string(),
        },
        user: UserEntity {
            id: user_id_b64.to_string(),
            name: user_name.to_string(),
            display_name: display_name.to_string(),
        },
        pub_key_cred_params: vec![PubKeyCredParam {
            cred_type: "public-key".to_string(),
            alg: -7, // ES256.
        }],
        authenticator_selection: AuthenticatorSelection {
            require_resident_key: true,
            resident_key: "required".to_string(),
            user_verification: "required".to_string(),
        },
        timeout: 60000,
        attestation: "direct".to_string(),
    })
}

fn make_get_options(
    rp_id: &str,
    challenge_b64: &str,
    allowed_cred_ids_json: &str,
) -> Result<GetOptions, String> {
    B64.decode(challenge_b64)
        .map_err(|e| format!("bad challenge: {e}"))?;
    let cred_ids: Vec<String> =
        serde_json::from_str(allowed_cred_ids_json).map_err(|e| e.to_string())?;

    Ok(GetOptions {
        challenge: challenge_b64.to_string(),
        rp_id: rp_id.to_string(),
        allow_credentials: cred_ids
            .into_iter()
            .map(|id| AllowCredential {
                cred_type: "public-key".to_string(),
                id,
            })
            .collect(),
        user_verification: "required".to_string(),
        timeout: 60000,
    })
}

fn make_attestation_encoded(
    attestation_object: &[u8],
    client_data_json: &[u8],
) -> AttestationEncoded {
    AttestationEncoded {
        attestation_b64: B64.encode(attestation_object),
        client_data_b64: B64.encode(client_data_json),
    }
}

fn make_assertion_encoded(
    authenticator_data: &[u8],
    client_data_json: &[u8],
    signature: &[u8],
) -> AssertionEncoded {
    AssertionEncoded {
        authenticator_data_b64: B64.encode(authenticator_data),
        client_data_b64: B64.encode(client_data_json),
        signature_b64: B64.encode(signature),
    }
}

// ── WASM Exports ─────────────────────────────────────────────────────

/// Generate a 32-byte cryptographically random challenge, returned as base64.
#[wasm_bindgen]
pub fn generate_challenge() -> Result<String, JsError> {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(B64.encode(buf))
}

/// Build PublicKeyCredentialCreationOptions for navigator.credentials.create().
/// Returns a JS object ready to pass as { publicKey: <result> }.
#[wasm_bindgen]
pub fn build_create_options(
    rp_id: &str,
    rp_name: &str,
    user_id_b64: &str,
    user_name: &str,
    display_name: &str,
    challenge_b64: &str,
) -> Result<JsValue, JsError> {
    let opts = make_create_options(
        rp_id,
        rp_name,
        user_id_b64,
        user_name,
        display_name,
        challenge_b64,
    )
    .map_err(|e| JsError::new(&e))?;
    serde_wasm_bindgen::to_value(&opts).map_err(|e| JsError::new(&e.to_string()))
}

/// Build PublicKeyCredentialRequestOptions for navigator.credentials.get().
/// `allowed_cred_ids_json` is a JSON array of base64 credential IDs: ["abc=", "xyz="]
#[wasm_bindgen]
pub fn build_get_options(
    rp_id: &str,
    challenge_b64: &str,
    allowed_cred_ids_json: &str,
) -> Result<JsValue, JsError> {
    let opts = make_get_options(rp_id, challenge_b64, allowed_cred_ids_json)
        .map_err(|e| JsError::new(&e))?;
    serde_wasm_bindgen::to_value(&opts).map_err(|e| JsError::new(&e.to_string()))
}

/// Encode raw attestation response buffers to base64 for sending to the server.
#[wasm_bindgen]
pub fn encode_attestation(
    attestation_object: &[u8],
    client_data_json: &[u8],
) -> Result<JsValue, JsError> {
    let encoded = make_attestation_encoded(attestation_object, client_data_json);
    serde_wasm_bindgen::to_value(&encoded).map_err(|e| JsError::new(&e.to_string()))
}

/// Encode raw assertion response buffers to base64 for sending to the server.
#[wasm_bindgen]
pub fn encode_assertion(
    authenticator_data: &[u8],
    client_data_json: &[u8],
    signature: &[u8],
) -> Result<JsValue, JsError> {
    let encoded = make_assertion_encoded(authenticator_data, client_data_json, signature);
    serde_wasm_bindgen::to_value(&encoded).map_err(|e| JsError::new(&e.to_string()))
}

// ── Tests (native — no JsValue, tests internal builders) ─────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_base64_32_bytes() {
        let c = generate_challenge().unwrap();
        let decoded = B64.decode(&c).unwrap();
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn create_options_validates_and_serializes() {
        let challenge = B64.encode(b"test-challenge-32-bytes-padding!");
        let user_id = B64.encode(b"user123");
        let opts = make_create_options(
            "localhost",
            "Test RP",
            &user_id,
            "testuser",
            "Test User",
            &challenge,
        )
        .unwrap();
        assert_eq!(opts.rp.id, "localhost");
        assert_eq!(opts.attestation, "direct");
        assert!(opts.authenticator_selection.require_resident_key);
        // Serializes to JSON without error.
        let json = serde_json::to_string(&opts).unwrap();
        assert!(json.contains("pubKeyCredParams"));
    }

    #[test]
    fn create_options_rejects_bad_challenge() {
        let user_id = B64.encode(b"user123");
        let result = make_create_options("localhost", "Test", &user_id, "u", "U", "not-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn get_options_parses_cred_ids() {
        let challenge = B64.encode(b"test-challenge-32-bytes-padding!");
        let creds =
            serde_json::to_string(&vec![B64.encode(b"cred1"), B64.encode(b"cred2")]).unwrap();
        let opts = make_get_options("localhost", &challenge, &creds).unwrap();
        assert_eq!(opts.allow_credentials.len(), 2);
        assert_eq!(opts.user_verification, "required");
    }

    #[test]
    fn attestation_encoding_round_trips() {
        let encoded = make_attestation_encoded(b"attestation-bytes", b"clientdata-bytes");
        let att = B64.decode(&encoded.attestation_b64).unwrap();
        let cd = B64.decode(&encoded.client_data_b64).unwrap();
        assert_eq!(att, b"attestation-bytes");
        assert_eq!(cd, b"clientdata-bytes");
    }

    #[test]
    fn assertion_encoding_round_trips() {
        let encoded = make_assertion_encoded(b"auth-data", b"client-data", b"sig-data");
        assert_eq!(
            B64.decode(&encoded.authenticator_data_b64).unwrap(),
            b"auth-data"
        );
        assert_eq!(
            B64.decode(&encoded.client_data_b64).unwrap(),
            b"client-data"
        );
        assert_eq!(B64.decode(&encoded.signature_b64).unwrap(), b"sig-data");
    }
}
