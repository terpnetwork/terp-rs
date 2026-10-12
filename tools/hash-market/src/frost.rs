//! FROST inbox nested in hash-market (HashMerchant = inbox + broadcast adapter).
//! Not FilePV / custody/frost.rs. Serve on 127.0.0.1 (HASHMERCHANT_BIND).

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::json;
use crate::onboard_authz::frost_http::SharedInbox;

pub fn router(inbox: SharedInbox) -> Router {
    Router::new()
        .route("/itl/frost/jobs", post(dispatch))
        .route("/itl/frost/jobs/:id", get(dispatch))
        .route("/itl/frost/jobs/:id/share", post(dispatch))
        .route("/itl/frost/jobs/:id/broadcast", post(dispatch))
        .with_state(inbox)
}

async fn dispatch(State(inbox): State<SharedInbox>, req: Request) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let mut g = match inbox.lock() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"ok": false, "error": "inbox_lock"}).to_string(),
            )
                .into_response();
        }
    };
    match g.try_handle(&method, &path, &body) {
        Some(r) => {
            let status =
                StatusCode::from_u16(r.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, [(header::CONTENT_TYPE, "application/json")], r.body).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"ok": false, "error": "not found"}).to_string(),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard_authz::frost_http::FrostInbox;
    use tower::ServiceExt;

    fn inbox() -> SharedInbox {
        std::sync::Arc::new(std::sync::Mutex::new(FrostInbox::default()))
    }

    #[tokio::test]
    async fn frost_nested_jobs_share_broadcast() {
        let app = router(inbox());
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/itl/frost/jobs")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from({
                        let rc = crate::onboard_authz::itl_frost::roster_commit(
                            0,
                            2,
                            3,
                            "vk-old",
                            &[
                                "desk".into(),
                                "steward".into(),
                                "spare".into(),
                            ],
                        );
                        format!(r#"{{"pay_code":"pay-20-ink","roster_commit":"{rc}"}}"#)
                    }))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = v["id"].as_str().unwrap().to_string();

        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/itl/frost/jobs/{id}/share"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"admin_id":"desk","blob":"S:0:a"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/itl/frost/jobs/{id}/share"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"admin_id":"steward","blob":"S:0:b"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/itl/frost/jobs/{id}/broadcast"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"txid":"txid-20"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
