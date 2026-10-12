//! Onboard feegrant surface **embedded** in hash-market.
//!
//! Canonical public paths (nest under `/onboard`):
//!   GET  /onboard/status
//!   GET  /onboard/v1/status
//!   POST /onboard/v1/register-grant   {"address":"terp1…"}
//!   POST /onboard/v1/mint-grant       {"address":"terp1…"}
//!
//! Not mounted at process root — `/health` stays HashMerchant liveness.
//! Do not run a second onboard daemon.

use crate::onboard_authz::{GrantPurpose, OnboardRuntime};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn router(rt: Arc<OnboardRuntime>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/v1/status", get(status))
        .route("/v1/register-grant", post(register_grant))
        .route("/v1/mint-grant", post(mint_grant))
        .with_state(rt)
}

async fn status(State(rt): State<Arc<OnboardRuntime>>) -> Json<Value> {
    Json(rt.status_json())
}

#[derive(Deserialize)]
struct GrantBody {
    address: String,
}

async fn register_grant(
    State(rt): State<Arc<OnboardRuntime>>,
    Json(body): Json<GrantBody>,
) -> (StatusCode, Json<Value>) {
    grant(rt, GrantPurpose::Register, &body.address)
}

async fn mint_grant(
    State(rt): State<Arc<OnboardRuntime>>,
    Json(body): Json<GrantBody>,
) -> (StatusCode, Json<Value>) {
    grant(rt, GrantPurpose::Mint, &body.address)
}

fn grant(
    rt: Arc<OnboardRuntime>,
    purpose: GrantPurpose,
    address: &str,
) -> (StatusCode, Json<Value>) {
    match rt.allocate(purpose, address) {
        Ok(v) => (StatusCode::OK, Json(v)),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard_authz::OnboardRoles;
    use tower::ServiceExt;

    fn rt() -> Arc<OnboardRuntime> {
        let mut r = OnboardRuntime::from_env();
        r.roles = OnboardRoles {
            runtime: "terp1daoplaceholderxxxxxxxxxxxxxxxxx".into(),
            hashmerchant: "terp1runtimeplaceholderxxxxxxxxxxxxxxxx".into(),
            chain_id: "120u-1".into(),
        };
        r.service_name = "hash-market-onboard".into();
        r.broadcast = false;
        r.broadcaster = None;
        Arc::new(r)
    }

    #[tokio::test]
    async fn onboard_nested_not_root_status() {
        let app = router(rt());
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["chain_id"], "120u-1");
        assert!(!v["allowed_messages"].as_array().unwrap().is_empty());
    }
}
