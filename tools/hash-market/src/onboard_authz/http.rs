//! Tiny HTTP surface so a host binary can mount onboard without a second process.
//!
//! Recognized paths (optional `/onboard` prefix):
//! - `GET  /status` | `/v1/status`
//! - `POST /v1/register-grant`  `{"address":"terp1…"}`
//! - `POST /v1/mint-grant`      `{"address":"terp1…"}`

use super::{GrantPurpose, OnboardRuntime};
use serde::Deserialize;
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    pub fn write_to(&self, s: &mut impl Write) -> std::io::Result<()> {
        let reason = if self.status == 200 { "OK" } else { "ERR" };
        let cors = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type\r\n";
        let resp = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
            self.status,
            reason,
            self.body.len(),
            cors,
            self.body
        );
        s.write_all(resp.as_bytes())
    }
}

#[derive(Deserialize)]
struct GrantBody {
    address: String,
    /// Blinded Pedersen C from DREGG pay-targets memo. Required unless
    /// `ONBOARD_SKIP_PAID=1` (lab).
    #[serde(default)]
    c: Option<String>,
}

/// Strip a trailing query string and an optional `/onboard` prefix.
pub fn normalize_path(path: &str) -> &str {
    let path = path.split('?').next().unwrap_or(path);
    path.strip_prefix("/onboard")
        .filter(|p| p.is_empty() || p.starts_with('/'))
        .unwrap_or(path)
}

impl OnboardRuntime {
    /// `Some` if this request belongs to the onboard surface; `None` so the
    /// host can keep serving its own routes.
    pub fn try_handle(&self, method: &str, path: &str, body: &str) -> Option<HttpResponse> {
        let path = normalize_path(path);
        if method == "OPTIONS"
            && (path == "/status"
                || path == "/v1/status"
                || path == "/v1/register-grant"
                || path == "/v1/mint-grant"
                || path == "/v1/passkey-grant"
                || path == "/health"
                || path == "/v1/health")
        {
            return Some(HttpResponse {
                status: 200,
                body: String::new(),
            });
        }
        if method == "GET"
            && (path == "/status"
                || path == "/v1/status"
                || path == "/health"
                || path == "/v1/health")
        {
            let mut v = self.status_json();
            if let Some(obj) = v.as_object_mut() {
                let ok = obj.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
                obj.insert("ok".into(), serde_json::json!(ok));
            }
            return Some(HttpResponse {
                status: if v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
                    200
                } else {
                    503
                },
                body: v.to_string(),
            });
        }
        if method == "POST"
            && (path == "/v1/register-grant"
                || path == "/v1/mint-grant"
                || path == "/v1/passkey-grant")
        {
            let parsed: GrantBody = match serde_json::from_str(body) {
                Ok(p) => p,
                Err(e) => {
                    return Some(HttpResponse {
                        status: 400,
                        body: json!({"error": e.to_string()}).to_string(),
                    });
                }
            };
            if path.ends_with("mint-grant")
                && std::env::var("ONBOARD_ALLOW_MINT_GRANT").ok().as_deref() != Some("1")
            {
                return Some(HttpResponse {
                    status: 404,
                    body: json!({
                        "ok": false,
                        "error": "HashMerchant does not grant mint; DREGG mints. passkey-grant is AddAuthenticator only.",
                    })
                    .to_string(),
                });
            }
            let purpose = if path.ends_with("mint-grant") {
                GrantPurpose::Mint
            } else if path.ends_with("passkey-grant") {
                GrantPurpose::Onboard
            } else {
                GrantPurpose::Register
            };
            if purpose != GrantPurpose::Mint {
                if let Err(e) =
                    OnboardRuntime::require_paid_commit(&parsed.address, parsed.c.as_deref())
                {
                    return Some(HttpResponse {
                        status: 402,
                        body: json!({"ok": false, "error": e}).to_string(),
                    });
                }
            }
            if path.ends_with("passkey-grant") && !self.broadcast {
                return Some(HttpResponse {
                    status: 503,
                    body: json!({
                        "ok": false,
                        "error": "broadcast disabled; not chain login. Device unlocks a key in this tab only.",
                    })
                    .to_string(),
                });
            }
            return Some(match self.allocate(purpose, &parsed.address) {
                Ok(v) => {
                    if path.ends_with("passkey-grant") {
                        let hash = v.get("txhash").and_then(|x| x.as_str()).unwrap_or("");
                        let granter = v.get("granter").and_then(|x| x.as_str()).unwrap_or("");
                        let runway_on =
                            !self.runway_amount.is_empty() && self.runway_amount != "0";
                        let rhash = v.get("runway_txhash").and_then(|x| x.as_str()).unwrap_or("");
                        let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
                        if !ok
                            || hash.is_empty()
                            || granter.is_empty()
                            || (runway_on && rhash.is_empty())
                        {
                            return Some(HttpResponse {
                                status: 502,
                                body: json!({
                                    "ok": false,
                                    "error": "passkey-grant broadcast produced no txhash",
                                    "detail": v,
                                })
                                .to_string(),
                            });
                        }
                    }
                    HttpResponse {
                        status: 200,
                        body: v.to_string(),
                    }
                }
                Err(e) => HttpResponse {
                    status: 400,
                    body: json!({"error": e}).to_string(),
                },
            });
        }
        if path == "/status"
            || path.starts_with("/v1/")
            || path == "/onboard"
            || path.starts_with("/onboard/")
        {
            return Some(HttpResponse {
                status: 404,
                body: json!({"error":"not found"}).to_string(),
            });
        }
        None
    }
}

fn handle_conn(mut s: TcpStream, runtime: &OnboardRuntime) {
    let mut buf = [0u8; 8192];
    let n = match s.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let req = String::from_utf8_lossy(&buf[..n]);
    let mut lines = req.split("\r\n");
    let start = lines.next().unwrap_or("");
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");
    let body = req.split("\r\n\r\n").nth(1).unwrap_or("").trim_end_matches('\0');
    let resp = runtime.try_handle(method, path, body).unwrap_or(HttpResponse {
        status: 404,
        body: json!({"error":"not found"}).to_string(),
    });
    let _ = resp.write_to(&mut s);
}

/// Blocking loop. One OS thread per connection so 100 checkouts don't serialize
/// on accept; grant broadcast still serializes inside `GrantBroadcaster`.
pub fn serve(bind: &str, runtime: Arc<OnboardRuntime>) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind)?;
    listener.set_nonblocking(false)?;
    eprintln!(
        "{} → http://{bind}  GET /status  POST /v1/register-grant|/v1/mint-grant|/v1/passkey-grant (threaded)",
        runtime.service_name
    );
    for stream in listener.incoming() {
        let s = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rt = runtime.clone();
        std::thread::spawn(move || handle_conn(s, rt.as_ref()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard_authz::OnboardRoles;

    fn rt() -> OnboardRuntime {
        let _ = std::env::set_var("ONBOARD_SKIP_PAID", "1");
        OnboardRuntime {
            roles: OnboardRoles {
                runtime: "terp1dao".into(),
                hashmerchant: "terp1rt".into(),
                chain_id: "120u-1".into(),
            },
            service_name: "test".into(),
            register_limit: "25000".into(),
            mint_limit: "50000".into(),
            runway_amount: crate::onboard_authz::RUNWAY_UTHOL.into(),
            broadcast: false,
            broadcaster: None,
            grant_tx_mu: std::sync::Mutex::new(()),
        }
    }

    #[test]
    fn host_prefix_and_bare_paths() {
        let rt = rt();
        assert!(rt.try_handle("GET", "/status", "").is_some());
        let h = rt.try_handle("GET", "/health", "").unwrap();
        assert_eq!(h.status, 200);
        let hv: serde_json::Value = serde_json::from_str(&h.body).unwrap();
        assert_eq!(hv["ok"], true);
        assert_eq!(hv["fee_granter"], "terp1rt");
        assert_ne!(hv["fee_granter"], hv["runtime"]);
        assert!(rt.try_handle("GET", "/onboard/status", "").is_some());
        let mint = rt
            .try_handle(
                "POST",
                "/v1/mint-grant",
                r#"{"address":"terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0"}"#,
            )
            .unwrap();
        assert_eq!(mint.status, 404);
        assert!(rt.try_handle("GET", "/ve/health", "").is_none());
        let g = rt
            .try_handle(
                "POST",
                "/onboard/v1/register-grant",
                r#"{"address":"terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0"}"#,
            )
            .unwrap();
        assert_eq!(g.status, 200, "{}", g.body);
        assert!(g.body.contains("AllowedMsgAllowance"));
        let gv: serde_json::Value = serde_json::from_str(&g.body).unwrap();
        assert_eq!(gv["granter"], "terp1rt");
        assert_ne!(gv["granter"], "terp1dao");
        let v: serde_json::Value = serde_json::from_str(&g.body).unwrap();
        let allowed = v["msg"]["allowance"]["allowed_messages"].as_array().unwrap();
        assert!(allowed.iter().all(|x| x.as_str() != Some("/cosmos.bank.v1beta1.MsgSend")));
        assert_eq!(v["runway"]["amount"], "1000000");
        assert_eq!(v["runway"]["msg"]["@type"], "/cosmos.bank.v1beta1.MsgSend");
        let pk = rt
            .try_handle(
                "POST",
                "/onboard/v1/passkey-grant",
                r#"{"address":"terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0"}"#,
            )
            .unwrap();
        assert_eq!(pk.status, 503);
    }
}
