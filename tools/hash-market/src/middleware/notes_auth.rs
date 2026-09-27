//! Production auth for private `/notes/*` (and blossom routes via BlossomState.auth).
//!
//! Modes (see [`crate::config::NotesAuthConfig`]):
//! - `noop` — allow all (local/dev; default)
//! - `bearer` — `Authorization: Bearer <token>`
//! - `secp` — snap headers `X-Auth-Type: secp256k1` + PrehashVerifier
//! - `bearer_or_secp` — either path succeeds
//!
//! Snap message (docs/headstash.md): `SHA256("{ts}\n{pubkey_hex}")` signed; server
//! uses `verify_prehash` (client already hashed).

use crate::config::NotesAuthConfig;
use cw721_nips::buds::{
    AuthClaims, AuthError, AuthResult, AuthScope, AuthVerifier, NoopAuthVerifier,
};
use k256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

impl NotesAuthConfig {
    /// Build verifier from config; falls back to env for bearer token.
    pub fn into_verifier(self) -> Arc<dyn AuthVerifier> {
        let mode = self.mode.to_lowercase();
        let bearer = self
            .bearer_token
            .clone()
            .or_else(|| std::env::var("NOTES_BEARER_TOKEN").ok())
            .or_else(|| std::env::var("JWT_SECRET").ok())
            .filter(|s| !s.is_empty());

        match mode.as_str() {
            "bearer" => {
                let token = bearer.unwrap_or_default();
                Arc::new(BearerTokenVerifier { token })
            }
            "secp" => Arc::new(SnapSecpVerifier::new(
                self.allowed_pubkeys,
                self.allow_any_secp,
                self.timestamp_tolerance_secs,
            )),
            "bearer_or_secp" => Arc::new(AnyOfAuthVerifier {
                verifiers: vec![
                    Arc::new(BearerTokenVerifier {
                        token: bearer.unwrap_or_default(),
                    }),
                    Arc::new(SnapSecpVerifier::new(
                        self.allowed_pubkeys,
                        self.allow_any_secp,
                        self.timestamp_tolerance_secs,
                    )),
                ],
            }),
            _ => Arc::new(NoopAuthVerifier),
        }
    }
}

fn full_claims(subject: impl Into<String>) -> AuthClaims {
    AuthClaims {
        subject: subject.into(),
        expires_at: u64::MAX,
        actions: vec![
            "get".into(),
            "upload".into(),
            "list".into(),
            "delete".into(),
        ],
        scope: AuthScope::Server,
    }
}

/// Accept only `Authorization: Bearer <token>`.
pub struct BearerTokenVerifier {
    pub token: String,
}

impl AuthVerifier for BearerTokenVerifier {
    fn verify(&self, headers: &axum::http::HeaderMap) -> AuthResult<AuthClaims> {
        if self.token.is_empty() {
            return Err(AuthError::Unauthorized);
        }
        let ok = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|t| t == self.token)
            .unwrap_or(false);
        if ok {
            Ok(full_claims("bearer"))
        } else {
            Err(AuthError::Unauthorized)
        }
    }
}

/// Snap-style secp256k1 prehash auth.
pub struct SnapSecpVerifier {
    allowed: HashSet<String>,
    allow_any: bool,
    tolerance_s: u64,
}

impl SnapSecpVerifier {
    pub fn new(allowed_pubkeys: Vec<String>, allow_any: bool, tolerance_s: u64) -> Self {
        let allowed = allowed_pubkeys
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        Self {
            allowed,
            allow_any,
            tolerance_s,
        }
    }
}

impl AuthVerifier for SnapSecpVerifier {
    fn verify(&self, headers: &axum::http::HeaderMap) -> AuthResult<AuthClaims> {
        let auth_type = headers
            .get("X-Auth-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth_type != "secp256k1" {
            return Err(AuthError::Unauthorized);
        }
        let pubkey_hex = headers
            .get("X-Pubkey")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AuthError::MissingHeader("X-Pubkey".into()))?;
        let timestamp_str = headers
            .get("X-Timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AuthError::MissingHeader("X-Timestamp".into()))?;
        let sig_hex = headers
            .get("X-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AuthError::MissingHeader("X-Signature".into()))?;

        let pk_norm = pubkey_hex.trim().to_lowercase();
        if !self.allow_any && !self.allowed.is_empty() && !self.allowed.contains(&pk_norm) {
            return Err(AuthError::Forbidden);
        }
        if !self.allow_any && self.allowed.is_empty() {
            return Err(AuthError::Unauthorized);
        }

        let ts: u64 = timestamp_str
            .trim()
            .parse()
            .map_err(|_| AuthError::Unauthorized)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let age = now.abs_diff(ts);
        if age > self.tolerance_s {
            return Err(AuthError::TimestampExpired);
        }

        let msg = format!("{timestamp_str}\n{pubkey_hex}");
        let msg_hash = Sha256::digest(msg.as_bytes());
        let pk_bytes = hex::decode(pubkey_hex).map_err(|_| AuthError::InvalidSignature)?;
        let vk =
            VerifyingKey::from_sec1_bytes(&pk_bytes).map_err(|_| AuthError::InvalidSignature)?;
        let sig_bytes = hex::decode(sig_hex.trim()).map_err(|_| AuthError::InvalidSignature)?;
        let sig =
            Signature::try_from(sig_bytes.as_slice()).map_err(|_| AuthError::InvalidSignature)?;
        vk.verify_prehash(&msg_hash, &sig)
            .map_err(|_| AuthError::InvalidSignature)?;

        Ok(full_claims(pk_norm))
    }
}

/// First successful verifier wins.
pub struct AnyOfAuthVerifier {
    pub verifiers: Vec<Arc<dyn AuthVerifier>>,
}

impl AuthVerifier for AnyOfAuthVerifier {
    fn verify(&self, headers: &axum::http::HeaderMap) -> AuthResult<AuthClaims> {
        let mut last = AuthError::Unauthorized;
        for v in &self.verifiers {
            match v.verify(headers) {
                Ok(c) => return Ok(c),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};

    fn sign_snap(sk: &SigningKey) -> HeaderMap {
        let pubkey_hex = hex::encode(sk.verifying_key().to_sec1_bytes());
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let msg = format!("{timestamp}\n{pubkey_hex}");
        let hash = Sha256::digest(msg.as_bytes());
        let sig: Signature = sk.sign_prehash(&hash).unwrap();
        let mut h = HeaderMap::new();
        h.insert("X-Auth-Type", "secp256k1".parse().unwrap());
        h.insert("X-Pubkey", pubkey_hex.parse().unwrap());
        h.insert("X-Timestamp", timestamp.parse().unwrap());
        h.insert("X-Signature", hex::encode(sig.to_bytes()).parse().unwrap());
        h
    }

    #[test]
    fn bearer_accepts_matching_token() {
        let v = BearerTokenVerifier {
            token: "secret".into(),
        };
        let mut h = HeaderMap::new();
        h.insert("Authorization", "Bearer secret".parse().unwrap());
        assert!(v.verify(&h).is_ok());
        h.insert("Authorization", "Bearer wrong".parse().unwrap());
        assert!(v.verify(&h).is_err());
    }

    #[test]
    fn snap_prehash_roundtrip() {
        let sk = SigningKey::random(&mut rand_core::OsRng);
        let pk = hex::encode(sk.verifying_key().to_sec1_bytes());
        let v = SnapSecpVerifier::new(vec![pk], false, 300);
        let h = sign_snap(&sk);
        assert!(v.verify(&h).is_ok());
    }

    #[test]
    fn bearer_or_secp_either_path() {
        let sk = SigningKey::random(&mut rand_core::OsRng);
        let pk = hex::encode(sk.verifying_key().to_sec1_bytes());
        let v = AnyOfAuthVerifier {
            verifiers: vec![
                Arc::new(BearerTokenVerifier {
                    token: "tok".into(),
                }),
                Arc::new(SnapSecpVerifier::new(vec![pk], false, 300)),
            ],
        };
        let mut h = HeaderMap::new();
        h.insert("Authorization", "Bearer tok".parse().unwrap());
        assert!(v.verify(&h).is_ok());
        assert!(v.verify(&sign_snap(&sk)).is_ok());
    }
}
