//! Blossom server — BUD protocol HTTP adapter layer.
//!
//! Maps HTTP requests to core BlobStore/AuthVerifier traits.
//! Requires `blossom` feature (implies `server`).

use super::{
    AuthClaims, AuthScope, AuthVerifier, BlobStore, BlobStoreError, PaymentProof, PaymentVerifier,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub struct BlossomState {
    pub store: Arc<dyn BlobStore>,
    pub auth: Arc<dyn AuthVerifier>,
    pub payment: Option<Arc<dyn PaymentVerifier>>,
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hash() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let hash = parse_hash(hex).unwrap();
        assert_eq!(hash.len(), 32);
        assert_eq!(hash[0], 0x01);
        assert_eq!(hash[31], 0xef);
    }

    #[test]
    fn test_parse_hash_invalid() {
        assert!(parse_hash("not-hex").is_err());
        assert!(parse_hash("0123").is_err()); // too short
    }
}

//  UPLOAD blob schema: https://github.com/hzrd149/blossom/blob/master/buds/06.md

// ── Router builder ────────────────────────────────────────────────────────────

pub fn blossom_router(state: Arc<BlossomState>) -> Router {
    Router::new()
        .route(
            "/blobs/:hash",
            axum::routing::get(get_blob).delete(delete_blob),
        )
        .route("/blobs", axum::routing::post(upload_blob).get(list_blobs))
        .route("/upload", axum::routing::post(upload_blob))
        .route("/health", axum::routing::get(health))
        .with_state(state)
}

pub mod blossom {
    use crate::buds::{BlobDescriptor, BlobStore, BlobStoreError};

    // ── In-memory implementation for testing ───────────────────────────────────────

    use sha2::Digest;
    use sha2::Sha256;
    use std::collections::HashMap;
    use std::sync::RwLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub struct InMemoryBlobStore {
        blobs: RwLock<HashMap<[u8; 32], Vec<u8>>>,
    }

    impl InMemoryBlobStore {
        pub fn new() -> Self {
            Self {
                blobs: RwLock::new(HashMap::new()),
            }
        }
    }

    impl Default for InMemoryBlobStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl BlobStore for InMemoryBlobStore {
        fn get(&self, hash: &[u8; 32]) -> Option<Vec<u8>> {
            self.blobs.read().unwrap().get(hash).cloned()
        }

        fn put(&self, content: Vec<u8>) -> Result<BlobDescriptor, BlobStoreError> {
            let hash: [u8; 32] = Sha256::digest(&content).into();
            let uploaded = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            // let descriptor = BlobDescriptor::build(content).mime_tpye() {
            //     url: format!("/blobs/{}", hex::encode(hash)),
            //     sha256: hash,
            //     size: content.len() as u64,
            //     mime_type: None,
            //     uploaded,
            // };
            let descriptor = BlobDescriptor {
                url: format!("/blobs/{}", hex::encode(hash)),
                sha256: hash,
                size: content.len() as u64,
                mime_type: None,
                uploaded,
            };

            self.blobs.write().unwrap().insert(hash, content);
            Ok(descriptor)
        }

        fn delete(&self, hash: &[u8; 32]) -> Result<(), BlobStoreError> {
            self.blobs
                .write()
                .unwrap()
                .remove(hash)
                .ok_or(BlobStoreError::NotFound)?;
            Ok(())
        }

        fn exists(&self, hash: &[u8; 32]) -> bool {
            self.blobs.read().unwrap().contains_key(hash)
        }

        fn list(&self) -> Vec<[u8; 32]> {
            self.blobs.read().unwrap().keys().cloned().collect()
        }
    }
}

// ── Shared state ─────────────────────────────────────────────────────────────

impl BlossomState {
    pub fn new(store: Arc<dyn BlobStore + 'static>) -> Self {
        //

        Self {
            store,
            auth: Arc::new(crate::buds::bud07::NoopAuthVerifier), //
            payment: None,
        }
    }

    pub fn with_auth(mut self, auth: Arc<dyn AuthVerifier>) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_payment(mut self, payment: Arc<dyn PaymentVerifier>) -> Self {
        self.payment = Some(payment);
        self
    }
}

// ── Handlers ───────────────────────────────────────────────────────────────────

async fn health(_: State<Arc<BlossomState>>) -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "protocol": "blossom" }))
}

/// GET /blobs/{hash} — retrieve a blob by SHA256 hash (hex).
async fn get_blob(
    State(state): State<Arc<BlossomState>>,
    Path(hash_hex): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    // 1. Verify auth
    let claims = state
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // 2. Parse hash from hex
    let hash = parse_hash(&hash_hex)?;

    // 3. Check scope
    check_scope(&claims, &hash)?;

    // 4. Check payment for download (BUD-07)
    if let Some(pv) = &state.payment {
        let proof = extract_payment_proof(&headers)?;
        pv.verify_download(&proof, &hash)
            .map_err(|_| StatusCode::PAYMENT_REQUIRED)?;
    }

    // 5. Fetch blob
    let blob = state.store.get(&hash).ok_or(StatusCode::NOT_FOUND)?;

    // 6. Build response with BUD-02 headers
    let mut resp = Response::new(Body::from(blob));
    if let Ok(hv) = axum::http::HeaderValue::from_str(&hash_hex) {
        resp.headers_mut().insert("X-Content-SHA256", hv);
    }
    resp.headers_mut().insert(
        "Content-Type",
        axum::http::HeaderValue::from_static("application/octet-stream"),
    );
    Ok(resp)
}

/// DELETE /blobs/{hash} — delete a blob.
async fn delete_blob(
    State(state): State<Arc<BlossomState>>,
    Path(hash_hex): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = state
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    check_action(&claims, "delete")?;
    let hash = parse_hash(&hash_hex)?;
    check_scope(&claims, &hash)?;

    state.store.delete(&hash).map_err(|e| match e {
        BlobStoreError::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /blobs or /upload — upload a blob.
async fn upload_blob(
    State(state): State<Arc<BlossomState>>,
    headers: HeaderMap,
    body: Body,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = state
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    check_action(&claims, "upload")?;

    // Read body
    let bytes = axum::body::to_bytes(body, 10 * 1024 * 1024) // 10MB max
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    // Check payment first (BUD-07) — must verify before storing
    if let Some(pv) = &state.payment {
        let proof = extract_payment_proof(&headers)?;
        pv.verify_upload(&proof, bytes.len() as u64)
            .map_err(|_| StatusCode::PAYMENT_REQUIRED)?;
    }

    // Store
    let descriptor = state.store.put(bytes.to_vec()).map_err(|e| match e {
        BlobStoreError::AlreadyExists => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    Ok(Json(serde_json::json!({
        "url": descriptor.url,
        "sha256": hex::encode(&descriptor.sha256),
        "size": descriptor.size,
        "type": descriptor.mime_type,
        "uploaded": descriptor.uploaded,
    })))
}

/// GET /blobs — list blob hashes.
async fn list_blobs(
    State(state): State<Arc<BlossomState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = state
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    check_action(&claims, "list")?;

    let hashes = match &claims.scope {
        AuthScope::BlobHashes(hashes) => hashes.clone(),
        AuthScope::Server => state.store.list(),
    };

    let hex_hashes: Vec<String> = hashes.iter().map(|h| hex::encode(h)).collect();
    Ok(Json(serde_json::json!({ "blobs": hex_hashes })))
}

// ── Auth helpers ───────────────────────────────────────────────────────────────

/// Check if the claims scope includes the given blob hash.
pub fn check_scope(claims: &AuthClaims, hash: &[u8; 32]) -> Result<(), StatusCode> {
    match &claims.scope {
        AuthScope::BlobHashes(hashes) => {
            if !hashes.contains(hash) {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        AuthScope::Server => {}
    }
    Ok(())
}

pub fn check_action(claims: &AuthClaims, action: &str) -> Result<(), StatusCode> {
    if !claims.actions.contains(&action.to_string()) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse a 64-char hex string into a 32-byte hash.
pub fn parse_hash(hex: &str) -> Result<[u8; 32], StatusCode> {
    hex::decode(hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or(StatusCode::BAD_REQUEST)
}

/// Extract payment proof from request headers.
pub fn extract_payment_proof(headers: &HeaderMap) -> Result<PaymentProof, StatusCode> {
    let sig = headers
        .get("X-Payment-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let pubkey = headers
        .get("X-Payment-Pubkey")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let msg = headers
        .get("X-Payment-Message")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(PaymentProof {
        signature: sig.to_string(),
        pubkey: pubkey.to_string(),
        message: msg.to_string(),
    })
}
