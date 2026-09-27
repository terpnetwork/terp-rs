//! Headstash note persistence L0/L1 tests (hash-market SSOT).
//!
//! L0: `HeadstashStore` multi-note, overwrite, envelope validation, PIR padding.
//!     Covered primarily in `store::headstash_note_tests`; this module adds
//!     design-level scenarios (bridge `cm.` addr + season slug) and optional L1 HTTP.
//!
//! L1: axum router PUT/GET with blossom `AuthVerifier` (noop default + reject harness).
//!
//! ```bash
//! cargo test -p hash-market --lib note_persist --features server
//! cargo test -p hash-market --lib headstash_note --features server
//! ```
//!
//! Non-goals: real XChaCha encrypt (Agent A), oracle bags, public `/content` dual-index.

use crate::store::HeadstashStore;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("hash-market-note-persist-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Opaque test envelope — ciphertext is dummy; store never decrypts.
fn opaque_envelope(ct: &str) -> serde_json::Value {
    serde_json::json!({
        "ciphertext": ct,
        "nonce": "00112233445566778899aabb",
        "scheme": "xchacha20poly1305",
        "cleartext_layout": "SEAM-NOTE-OUT-V0",
        "cleartext_len": 382
    })
}

/// Bridge addr convention: `cm.` + hex(commitment-like bytes).
fn bridge_cm_addr(cm32: &[u8; 32]) -> String {
    format!("cm.{}", hex::encode(cm32))
}

#[test]
fn note_persist_bridge_cm_addr_under_season_slug() {
    let dir = tmp_dir("bridge-cm");
    let store = HeadstashStore::new(&dir).unwrap();
    let hs_id = "season-1"; // season slug (also accept contract bech32 in other tests)
    let mut cm = [0u8; 32];
    cm[0] = 0xde;
    cm[1] = 0xad;
    cm[31] = 0xef;
    let addr = bridge_cm_addr(&cm);
    assert!(addr.starts_with("cm."));
    assert!(!addr.contains('/'));
    assert!(!addr.contains(".."));

    let env = opaque_envelope("dummy-bridge-ct");
    store.set_note(hs_id, &addr, &env).unwrap();
    let got = store.get_note(hs_id, &addr).unwrap().expect("stored");
    assert_eq!(got["cleartext_len"], 382);
    assert_eq!(got["cleartext_layout"], "SEAM-NOTE-OUT-V0");
    assert_eq!(got["ciphertext"], "dummy-bridge-ct");

    // Path shape notes/{hs_id}/{addr}.json
    let path = dir.join("notes").join(hs_id).join(format!("{addr}.json"));
    assert!(path.is_file(), "expected file at {}", path.display());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn note_persist_contract_bech32_hs_id() {
    let dir = tmp_dir("bech32-hs");
    let store = HeadstashStore::new(&dir).unwrap();
    let hs_id = "terp1headstashcontractdummy00000000000001";
    let addr = "cm.aabbccdd";
    store
        .set_note(hs_id, addr, &opaque_envelope("ct-bech32-hs"))
        .unwrap();
    assert!(store.get_note(hs_id, addr).unwrap().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn note_persist_private_notes_not_under_content_index() {
    // Design lock: private notes live only under notes/{hs_id}/ — never dual-index /content.
    let dir = tmp_dir("privacy");
    let store = HeadstashStore::new(&dir).unwrap();
    store
        .set_note("hs-priv", "cm.priv01", &opaque_envelope("secret-body"))
        .unwrap();
    assert!(dir
        .join("notes")
        .join("hs-priv")
        .join("cm.priv01.json")
        .is_file());
    // No content-plane spill
    assert!(!dir.join("content").exists());
    assert!(!dir.join("blobs").exists() || store.list_note_keys("hs-priv").unwrap().len() == 1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── L1 HTTP (feature server) ─────────────────────────────────────────────────
//
// Default `BlossomState::new` uses `NoopAuthVerifier` (always OK). For 401 we
// inject a verifier that requires `Authorization: Bearer test-token`.

#[cfg(feature = "server")]
mod l1_http {
    use super::*;
    use crate::server::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use cw721_nips::buds::{AuthClaims, AuthError, AuthResult, AuthScope, AuthVerifier};
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Auth that accepts only `Authorization: Bearer test-token`.
    struct RequireBearerToken;

    impl AuthVerifier for RequireBearerToken {
        fn verify(&self, headers: &axum::http::HeaderMap) -> AuthResult<AuthClaims> {
            let ok = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| s == "Bearer test-token")
                .unwrap_or(false);
            if ok {
                Ok(AuthClaims {
                    subject: "test".into(),
                    expires_at: u64::MAX,
                    actions: vec![
                        "get".into(),
                        "upload".into(),
                        "list".into(),
                        "delete".into(),
                    ],
                    scope: AuthScope::Server,
                })
            } else {
                Err(AuthError::Unauthorized)
            }
        }
    }

    fn app_with_auth(dir: PathBuf) -> (axum::Router, PathBuf) {
        let mut state = AppState::new_merkle_host("test-chain".into(), dir.clone()).unwrap();
        {
            let blossom = Arc::get_mut(&mut state.blossom).expect("unique Arc for test setup");
            blossom.auth = Arc::new(RequireBearerToken);
        }
        (router(Arc::new(state)), dir)
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn note_persist_http_put_get_with_auth() {
        let dir = tmp_dir("http-put-get");
        let (app, dir) = app_with_auth(dir);
        let hs = "season-1";
        let addr = "cm.deadbeef";
        let path = format!("/notes/{hs}/{addr}");
        let body = serde_json::to_vec(&opaque_envelope("http-ct-1")).unwrap();

        let put = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&path)
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        let get = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&path)
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        let json = body_json(get).await;
        assert_eq!(json["ciphertext"], "http-ct-1");
        assert_eq!(json["scheme"], "xchacha20poly1305");
        assert_eq!(json["cleartext_len"], 382);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn note_persist_http_put_without_auth_401() {
        let dir = tmp_dir("http-401");
        let (app, dir) = app_with_auth(dir);
        let path = "/notes/season-1/cm.noauth";
        let body = serde_json::to_vec(&opaque_envelope("should-not-store")).unwrap();

        let put = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(path)
                    .header("content-type", "application/json")
                    // no Authorization
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Default build keeps encrypted notes and does not serve the public-leaf PIR route.
    #[cfg(not(feature = "pir"))]
    #[tokio::test]
    async fn pir_feature_off_is_404_and_note_put_still_works() {
        let dir = tmp_dir("pir-off");
        let (app, dir) = app_with_auth(dir);
        let hs = "community-notes";

        let pir = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/notes/{hs}/pir"))
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token")
                    .body(Body::from(r#"{"selector":[1]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pir.status(), StatusCode::NOT_FOUND);

        let index = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/notes/{hs}/pir-index"))
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(index.status(), StatusCode::NOT_FOUND);

        let put = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/notes/{hs}/cm.aabb"))
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token")
                    .body(Body::from(
                        serde_json::to_vec(&opaque_envelope("still-bud-notes")).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn note_persist_http_get_missing_404() {
        let dir = tmp_dir("http-404");
        let (app, dir) = app_with_auth(dir);

        let get = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/notes/season-1/cm.doesnotexist")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::NOT_FOUND);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
