//! FROST inbox nested in hash-market (HashMerchant = inbox + broadcast adapter).
//! Not FilePV / custody/frost.rs. Serve on 127.0.0.1 (HASHMERCHANT_BIND).
//! Session cookie routes live here too — Next export cannot Set-Cookie.

use crate::onboard_authz::frost_http::SharedInbox;
use crate::onboard_authz::itl_session;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

pub fn router(inbox: SharedInbox) -> Router {
    Router::new()
        .route("/itl/frost/jobs", post(dispatch))
        .route("/itl/frost/jobs/:id", get(dispatch))
        .route("/itl/frost/jobs/:id/share", post(dispatch))
        .route("/itl/frost/jobs/:id/broadcast", post(dispatch))
        .route("/itl/door/consume", post(door_dispatch))
        .route("/itl/door/ushers", post(door_dispatch))
        .route("/itl/door/paid", post(door_dispatch))
        .route(
            "/itl/session/challenge",
            post(session_dispatch).options(session_dispatch),
        )
        .route(
            "/itl/session/verify",
            post(session_dispatch).options(session_dispatch),
        )
        .route(
            "/itl/session/me",
            get(session_dispatch).options(session_dispatch),
        )
        .route(
            "/itl/session/logout",
            post(session_dispatch).options(session_dispatch),
        )
        .route(
            "/itl/session/health",
            get(session_dispatch).options(session_dispatch),
        )
        .route(
            "/itl/session",
            get(session_dispatch).options(session_dispatch),
        )
        .with_state(inbox)
}

async fn session_dispatch(req: Request) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let Some(r) =
        itl_session::try_handle(&method, &path, &body, origin.as_deref(), cookie.as_deref())
    else {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"ok": false, "error": "not found"}).to_string(),
        )
            .into_response();
    };
    let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store");
    if let Some(o) = origin.as_deref()
        && itl_session::origin_allowed(o)
    {
        builder = builder
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, o)
            .header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true")
            .header(header::VARY, "Origin")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS")
            .header(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "content-type, x-itl-frost",
            );
    }
    if let Some(c) = r.set_cookie {
        builder = builder.header(header::SET_COOKIE, c);
    }
    builder
        .body(axum::body::Body::from(r.body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn json_resp(status: u16, body: String) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

async fn door_dispatch(req: Request) -> Response {
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    if let Some(r) = crate::onboard_authz::frost_http::authorize(&path, cookie.as_deref()) {
        return json_resp(r.status, r.body);
    }
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    match crate::onboard_authz::itl_door::try_handle(&method, &path, &body, cookie.as_deref()) {
        Some(r) => json_resp(r.status, r.body),
        None => json_resp(404, json!({"ok": false, "error": "not found"}).to_string()),
    }
}

async fn dispatch(State(inbox): State<SharedInbox>, req: Request) -> Response {
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    if let Some(r) = crate::onboard_authz::frost_http::authorize(&path, cookie.as_deref()) {
        return json_resp(r.status, r.body);
    }
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
    use crate::onboard_authz::itl_session::mint_test_cookie;
    use tower::ServiceExt;

    fn inbox() -> SharedInbox {
        std::sync::Arc::new(std::sync::Mutex::new(FrostInbox::default()))
    }

    fn admin_cookie() -> String {
        mint_test_cookie("admin", "desk")
    }

    #[tokio::test]
    async fn frost_nested_jobs_share_broadcast() {
        let app = router(inbox());
        let cookie = admin_cookie();
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/itl/frost/jobs")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(axum::body::Body::from({
                        let rc = crate::onboard_authz::itl_frost::roster_commit(
                            0,
                            2,
                            3,
                            "vk-old",
                            &["desk".into(), "steward".into(), "spare".into()],
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
                    .header("cookie", &cookie)
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
                    .header("cookie", &cookie)
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
                    .header("cookie", &cookie)
                    .body(axum::body::Body::from(r#"{"txid":"txid-20"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn frost_job_refuses_anonymous_and_usher() {
        let app = router(inbox());
        let anon = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/itl/frost/jobs")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"pay_code":"x","roster_commit":"ab"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
        let usher = mint_test_cookie("usher", "ada");
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/itl/frost/jobs")
                    .header("content-type", "application/json")
                    .header("cookie", usher)
                    .body(axum::body::Body::from(
                        r#"{"pay_code":"x","roster_commit":"ab"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn session_me_without_cookie_is_401() {
        let app = router(inbox());
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/itl/session/me")
                    .header("origin", "http://127.0.0.1:3051")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let acao = res
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(acao, "http://127.0.0.1:3051");
        assert_ne!(acao, "*");
    }
}
