//! Cashu off-chain mesh L0/L1 tests (hash-market).
//!
//! L0: `CashuMeshStore` mint descriptor + wallet envelope round-trip, invalid ids.
//! L1: axum router PUT/GET with blossom `AuthVerifier` (public mint list vs auth writes).
//!
//! ```bash
//! cargo test -p hash-market --lib cashu --features server
//! ```
//!
//! Schema: `docs/plans/cashu/CANONICAL-MINT-REGISTRY.md` (canonical, not “official”).
//! Non-goals: CDK, CosmWasm registry, public dual-index of wallet proofs.

use crate::store::{CashuMeshStore, MintDescriptor, MintStatus};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("hash-market-cashu-mesh-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample_mint(id: &str, url: &str) -> MintDescriptor {
    MintDescriptor {
        mint_id: id.into(),
        url: url.into(),
        name: Some("testnut".into()),
        units: vec!["sat".into()],
        keyset_ids: vec!["00dead".into()],
        nuts: serde_json::json!({}),
        pubkey: None,
        status: MintStatus::Active,
        content_sha256: None,
        registered_at: 0,
        updated_at: 0,
        registrar: None,
        metadata: serde_json::json!({}),
        source: Some("manual".into()),
    }
}

/// Opaque wallet backup envelope — store never decrypts.
fn wallet_envelope(ct: &str) -> serde_json::Value {
    serde_json::json!({
        "ciphertext": ct,
        "nonce": "00112233445566778899aabb",
        "scheme": "xchacha20poly1305",
        "cleartext_layout": "CASHU-TOKEN-BACKUP-V0",
        "cleartext_len": 128
    })
}

#[test]
fn cashu_mesh_mint_descriptor_round_trip() {
    let dir = tmp_dir("mint-l0");
    let store = CashuMeshStore::new(&dir).unwrap();
    let id = "mintsha256hexdemo01";
    store
        .set_mint(id, sample_mint(id, "https://mint.example"))
        .unwrap();
    let got = store.get_mint(id).unwrap().expect("stored");
    assert_eq!(got.mint_id, id);
    assert_eq!(got.url, "https://mint.example");
    assert_eq!(got.status, MintStatus::Active);
    assert_eq!(got.units, vec!["sat".to_string()]);
    assert!(
        dir.join("cashu")
            .join("mints")
            .join(format!("{id}.json"))
            .is_file()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cashu_mesh_wallet_envelope_round_trip() {
    let dir = tmp_dir("wallet-l0");
    let store = CashuMeshStore::new(&dir).unwrap();
    let wid = "wallet-alice";
    let iid = "backup-v0";
    store
        .set_wallet_item(wid, iid, &wallet_envelope("wallet-ct-1"))
        .unwrap();
    let got = store.get_wallet_item(wid, iid).unwrap().expect("stored");
    assert_eq!(got["ciphertext"], "wallet-ct-1");
    assert_eq!(got["cleartext_layout"], "CASHU-TOKEN-BACKUP-V0");
    // Privacy: wallet stash only under cashu/wallets — never content/notes spill
    assert!(!dir.join("content").exists());
    assert!(!dir.join("notes").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cashu_mesh_invalid_id_rejected() {
    let dir = tmp_dir("bad-id");
    let store = CashuMeshStore::new(&dir).unwrap();
    let m = sample_mint("ok", "https://m.example");
    assert!(store.set_mint("../x", m.clone()).is_err());
    assert!(store.set_mint("a/b", m).is_err());
    let env = wallet_envelope("ct");
    assert!(store.set_wallet_item("w", "foo..bar", &env).is_err());
    assert!(store.set_wallet_item("", "item", &env).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── L1 HTTP (feature server) ─────────────────────────────────────────────────

#[cfg(feature = "server")]
mod l1_http {
    use super::*;
    use crate::server::{AppState, router};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use cw721_nips::buds::{AuthClaims, AuthError, AuthResult, AuthScope, AuthVerifier};
    use std::sync::Arc;
    use tower::ServiceExt;

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
        // Defaults: cashu_mesh_enabled + public_mint_list true
        (router(Arc::new(state)), dir)
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn cashu_mesh_http_put_get_mint_with_auth() {
        let dir = tmp_dir("http-mint");
        let (app, dir) = app_with_auth(dir);
        let mint_id = "labmint01";
        let path = format!("/cashu/mints/{mint_id}");
        let body = serde_json::to_vec(&serde_json::json!({
            "url": "https://mint.example",
            "name": "lab",
            "units": ["sat"],
            "status": "active",
            "source": "manual"
        }))
        .unwrap();

        let put = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&path)
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        // Public read (no auth) when public_mint_list default true
        let get = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        let json = body_json(get).await;
        assert_eq!(json["mint_id"], mint_id);
        assert_eq!(json["url"], "https://mint.example");
        assert_eq!(json["status"], "active");

        let list = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/cashu/mints")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let list_json = body_json(list).await;
        assert!(
            list_json["keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k == mint_id)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cashu_mesh_http_put_mint_without_auth_401() {
        let dir = tmp_dir("http-mint-401");
        let (app, dir) = app_with_auth(dir);
        let body = serde_json::to_vec(&serde_json::json!({
            "url": "https://mint.example"
        }))
        .unwrap();

        let put = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/cashu/mints/noauth")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cashu_mesh_http_wallet_put_get_with_auth() {
        let dir = tmp_dir("http-wallet");
        let (app, dir) = app_with_auth(dir);
        let path = "/cashu/wallets/w1/item1";
        let body = serde_json::to_vec(&wallet_envelope("http-wallet-ct")).unwrap();

        let put = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        let get = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        let json = body_json(get).await;
        assert_eq!(json["ciphertext"], "http-wallet-ct");
        assert_eq!(json["scheme"], "xchacha20poly1305");

        // Wallet GET without auth → 401
        let noauth = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(noauth.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cashu_mesh_http_get_mint_missing_404() {
        let dir = tmp_dir("http-mint-404");
        let (app, dir) = app_with_auth(dir);

        let get = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/cashu/mints/doesnotexist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::NOT_FOUND);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
