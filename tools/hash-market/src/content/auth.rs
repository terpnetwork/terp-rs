//! Ingest auth for distribution plane: shared bearer and/or plane HS256 JWT.
//!
//! Aligns with oline `auth_plane::service_jwt`:
//! - secret: `OLINE_SERVICE_JWT_SECRET` (or config / lab fallback)
//! - audience: `hash-market-distribution`
//! - role: `content.ingest` (optional require)

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_AUD: &str = "hash-market-distribution";
pub const ROLE_CONTENT_INGEST: &str = "content.ingest";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IngestAuthMode {
    /// Shared secret only (local/dev default).
    #[default]
    Bearer,
    /// Plane/service HS256 JWT only.
    JwtPlane,
    /// Accept either bearer secret or valid plane JWT.
    Both,
}

impl IngestAuthMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "jwt" | "jwt-plane" | "jwt_plane" | "plane" => Self::JwtPlane,
            "both" | "any" => Self::Both,
            _ => Self::Bearer,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceJwtClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: u64,
    #[serde(default)]
    pub iat: u64,
    #[serde(default)]
    pub nbf: Option<u64>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub profile: Option<String>,
}

/// Resolve HMAC secret (same env convention as oline service JWT).
pub fn resolve_service_jwt_secret(config_secret: Option<&str>) -> Option<Vec<u8>> {
    if let Some(s) = config_secret {
        if !s.is_empty() {
            return Some(s.as_bytes().to_vec());
        }
    }
    for key in [
        "OLINE_SERVICE_JWT_SECRET",
        "OLINE_ACCOUNT_JWT_SECRET",
        "JWT_SECRET",
    ] {
        if let Ok(s) = std::env::var(key) {
            if !s.is_empty() {
                return Some(s.into_bytes());
            }
        }
    }
    None
}

pub fn verify_service_jwt(
    token: &str,
    secret: &[u8],
    expected_aud: &str,
    required_role: Option<&str>,
) -> Result<ServiceJwtClaims, String> {
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 3 {
        return Err("jwt must have 3 segments".into());
    }
    let (h_b64, p_b64, s_b64) = (parts[0], parts[1], parts[2]);
    let header_raw = URL_SAFE_NO_PAD
        .decode(h_b64)
        .map_err(|e| format!("jwt header: {e}"))?;
    let header: serde_json::Value =
        serde_json::from_slice(&header_raw).map_err(|e| format!("jwt header json: {e}"))?;
    if header.get("alg").and_then(|v| v.as_str()) != Some("HS256") {
        return Err("need HS256".into());
    }
    let payload_raw = URL_SAFE_NO_PAD
        .decode(p_b64)
        .map_err(|e| format!("jwt payload: {e}"))?;
    let claims: ServiceJwtClaims =
        serde_json::from_slice(&payload_raw).map_err(|e| format!("jwt claims: {e}"))?;

    if claims.aud != expected_aud {
        return Err(format!(
            "aud mismatch: got {}, want {expected_aud}",
            claims.aud
        ));
    }

    let signing_input = format!("{h_b64}.{p_b64}");
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| format!("hmac: {e}"))?;
    mac.update(signing_input.as_bytes());
    let expected = mac.finalize().into_bytes();
    let sig = URL_SAFE_NO_PAD
        .decode(s_b64)
        .map_err(|e| format!("jwt sig: {e}"))?;
    if !constant_time_eq(&sig, &expected) {
        return Err("jwt signature invalid".into());
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(nbf) = claims.nbf {
        if now < nbf {
            return Err("jwt nbf".into());
        }
    }
    if now >= claims.exp {
        return Err("jwt expired".into());
    }
    if claims.sub.is_empty() {
        return Err("jwt sub empty".into());
    }
    if let Some(role) = required_role {
        if !claims.roles.iter().any(|r| r == role) {
            return Err(format!("missing role {role}"));
        }
    }
    Ok(claims)
}

/// Sign HS256 (for tests / local mint without oline binary).
pub fn sign_service_jwt(claims: &ServiceJwtClaims, secret: &[u8]) -> Result<String, String> {
    let header = serde_json::json!({"alg":"HS256","typ":"JWT"});
    let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).map_err(|e| e.to_string())?);
    let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).map_err(|e| e.to_string())?);
    let signing_input = format!("{h}.{p}");
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| format!("hmac: {e}"))?;
    mac.update(signing_input.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{signing_input}.{sig}"))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut v = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        v |= x ^ y;
    }
    v == 0
}

/// Check Authorization / X-Minio-Webhook-Auth against distribution auth policy.
pub fn check_ingest_auth(
    auth_header: Option<&str>,
    x_minio: Option<&str>,
    mode: &IngestAuthMode,
    bearer_token: Option<&str>,
    jwt_secret: Option<&[u8]>,
    jwt_audience: &str,
    require_role: Option<&str>,
) -> Result<(), String> {
    let raw = auth_header
        .or(x_minio)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let try_bearer = |token: &str| -> bool {
        let Some(expected) = bearer_token.filter(|s| !s.is_empty()) else {
            return false;
        };
        let candidates = [format!("Bearer {expected}"), expected.to_string()];
        candidates
            .iter()
            .any(|c| constant_time_eq(c.as_bytes(), token.as_bytes()))
            || x_minio.is_some_and(|x| constant_time_eq(x.as_bytes(), expected.as_bytes()))
    };

    let try_jwt = |token: &str| -> Result<(), String> {
        let secret = jwt_secret.ok_or_else(|| "jwt-plane configured but no secret".to_string())?;
        let t = token
            .strip_prefix("Bearer ")
            .or_else(|| token.strip_prefix("bearer "))
            .unwrap_or(token);
        verify_service_jwt(t, secret, jwt_audience, require_role).map(|_| ())
    };

    match mode {
        IngestAuthMode::Bearer => {
            // No bearer configured → open (local e2e)
            if bearer_token.map(|s| s.is_empty()).unwrap_or(true) {
                return Ok(());
            }
            let token = raw.ok_or_else(|| "missing Authorization".to_string())?;
            if try_bearer(token) {
                Ok(())
            } else {
                Err("bearer mismatch".into())
            }
        }
        IngestAuthMode::JwtPlane => {
            let token = raw.ok_or_else(|| "missing Authorization".to_string())?;
            try_jwt(token)
        }
        IngestAuthMode::Both => {
            if bearer_token.map(|s| s.is_empty()).unwrap_or(true) && jwt_secret.is_none() {
                return Ok(()); // fully open lab
            }
            let token = raw.ok_or_else(|| "missing Authorization".to_string())?;
            if try_bearer(token) {
                return Ok(());
            }
            try_jwt(token)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_roundtrip_and_auth() {
        let secret = b"plane-service-secret-lab!!!!";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = ServiceJwtClaims {
            sub: "webhook-collector".into(),
            iss: "webhook-collector".into(),
            aud: DEFAULT_AUD.into(),
            exp: now + 300,
            iat: now,
            nbf: Some(now - 1),
            roles: vec![ROLE_CONTENT_INGEST.into()],
            profile: None,
        };
        let tok = sign_service_jwt(&claims, secret).unwrap();
        let v = verify_service_jwt(&tok, secret, DEFAULT_AUD, Some(ROLE_CONTENT_INGEST)).unwrap();
        assert_eq!(v.sub, "webhook-collector");

        check_ingest_auth(
            Some(&format!("Bearer {tok}")),
            None,
            &IngestAuthMode::JwtPlane,
            None,
            Some(secret.as_slice()),
            DEFAULT_AUD,
            Some(ROLE_CONTENT_INGEST),
        )
        .unwrap();

        check_ingest_auth(
            Some("Bearer shared-dev-token"),
            None,
            &IngestAuthMode::Both,
            Some("shared-dev-token"),
            Some(secret.as_slice()),
            DEFAULT_AUD,
            None,
        )
        .unwrap();
    }
}
