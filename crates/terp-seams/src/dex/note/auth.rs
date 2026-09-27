//! Client auth headers for hash-market /notes (blossom-compatible).
//!
//! Two production modes (match server `NotesAuthConfig`):
//! 1. **Bearer** — `Authorization: Bearer <token>` (service JWT / shared secret)
//! 2. **Snap secp256k1** — `X-Auth-Type` / `X-Pubkey` / `X-Timestamp` / `X-Signature`
//!    over `SHA256("{ts}\n{pubkey_hex}")` via `PrehashVerifier` on the server.
//!
//! Feature `auth` enables snap signing (`k256`). Bearer helpers are always available.

use super::NotePersistError;
use std::time::{SystemTime, UNIX_EPOCH};

/// Build `Authorization: Bearer …` header pair.
pub fn bearer_auth_headers(token: &str) -> Vec<(String, String)> {
    vec![("Authorization".into(), format!("Bearer {token}"))]
}

/// Current unix seconds as string (snap / admin timestamp header).
pub fn now_timestamp_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

/// Canonical snap message: `{timestamp}\n{compressed_pubkey_hex}` (no trailing newline).
pub fn snap_auth_message(timestamp: &str, pubkey_hex: &str) -> String {
    format!("{timestamp}\n{pubkey_hex}")
}

/// Feature-gated: sign snap headers with a 32-byte secp256k1 secret key.
#[cfg(feature = "auth")]
pub fn snap_secp_auth_headers(
    secret_key_hex: &str,
) -> Result<Vec<(String, String)>, NotePersistError> {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};
    use sha2::{Digest, Sha256};

    let sk_bytes = hex::decode(secret_key_hex.trim()).map_err(|_| NotePersistError::BadHex)?;
    if sk_bytes.len() != 32 {
        return Err(NotePersistError::Auth(
            "secp secret must be 32-byte hex".into(),
        ));
    }
    let sk = SigningKey::from_slice(&sk_bytes).map_err(|e| NotePersistError::Auth(e.to_string()))?;
    let pubkey_hex = hex::encode(sk.verifying_key().to_sec1_bytes());
    let timestamp = now_timestamp_secs();
    let msg = snap_auth_message(&timestamp, &pubkey_hex);
    let msg_hash = Sha256::digest(msg.as_bytes());
    let sig: Signature = sk
        .sign_prehash(&msg_hash)
        .map_err(|e| NotePersistError::Auth(e.to_string()))?;
    let sig_hex = hex::encode(sig.to_bytes());

    Ok(vec![
        ("X-Auth-Type".into(), "secp256k1".into()),
        ("X-Pubkey".into(), pubkey_hex),
        ("X-Timestamp".into(), timestamp),
        ("X-Signature".into(), sig_hex),
    ])
}

/// Merge auth sources: bearer and/or pre-built header pairs (snap).
pub fn merge_auth_headers(
    bearer: Option<&str>,
    extra: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(t) = bearer {
        if !t.is_empty() {
            out.extend(bearer_auth_headers(t));
        }
    }
    out.extend(extra.iter().cloned());
    out
}

/// Load auth headers from environment for wallet/CLI.
///
/// | Env | Effect |
/// |-----|--------|
/// | `NOTES_BEARER_TOKEN` | Bearer auth |
/// | `NOTES_SECP_SK` | Snap secp headers (requires feature `auth`) |
pub fn auth_headers_from_env() -> Result<Vec<(String, String)>, NotePersistError> {
    let bearer = std::env::var("NOTES_BEARER_TOKEN").ok();
    let mut headers = merge_auth_headers(bearer.as_deref(), &[]);

    #[cfg(feature = "auth")]
    {
        if let Ok(sk) = std::env::var("NOTES_SECP_SK") {
            if !sk.is_empty() {
                headers.extend(snap_secp_auth_headers(&sk)?);
            }
        }
    }

    #[cfg(not(feature = "auth"))]
    {
        let _ = &mut headers;
        if std::env::var("NOTES_SECP_SK").is_ok() {
            return Err(NotePersistError::Auth(
                "NOTES_SECP_SK set but feature `auth` disabled (enable k256 snap signing)".into(),
            ));
        }
    }

    Ok(headers)
}

#[cfg(all(test, feature = "auth"))]
mod tests {
    use super::*;
    use k256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, SigningKey, VerifyingKey};
    use sha2::{Digest, Sha256};

    #[test]
    fn snap_headers_verify_with_prehash() {
        let sk = SigningKey::random(&mut rand_core::OsRng);
        let sk_hex = hex::encode(sk.to_bytes());
        let headers = snap_secp_auth_headers(&sk_hex).unwrap();
        let map: std::collections::HashMap<_, _> = headers.into_iter().collect();
        assert_eq!(map.get("X-Auth-Type").unwrap(), "secp256k1");
        let pk = map.get("X-Pubkey").unwrap();
        let ts = map.get("X-Timestamp").unwrap();
        let sig_hex = map.get("X-Signature").unwrap();

        let msg = snap_auth_message(ts, pk);
        let hash = Sha256::digest(msg.as_bytes());
        let vk = VerifyingKey::from_sec1_bytes(&hex::decode(pk).unwrap()).unwrap();
        let sig = Signature::from_slice(&hex::decode(sig_hex).unwrap()).unwrap();
        vk.verify_prehash(&hash, &sig).expect("prehash ok");
    }
}
