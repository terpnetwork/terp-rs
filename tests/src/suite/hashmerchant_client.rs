//! Composable HashMerchant HTTP client.
//!
//! Same methods against:
//! - an **in-process** Axum router (parallel cargo tests, no port, no binary)
//! - a **live URL** from [`super::hashmerchant::HashMerchantSuite`] / [`SidecarFleet`]
//!
//! Feature planes are methods, not extra crates. Curate grant *msgs* stay in
//! `terp_onboard_authz::curate`; this type only speaks HTTP.

use anyhow::{Context, Result};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

use hash_market::server::{router, AppState};

/// Talks to hash-market-server planes (`/health`, `/blobs`, `/trees`, `/onboard`).
#[derive(Clone)]
pub struct HashMerchantClient {
    mode: Mode,
    /// Keeps the merkle/blob data dir alive for in-process hosts.
    _data: Option<Arc<tempfile::TempDir>>,
}

#[derive(Clone)]
enum Mode {
    Http(String),
    InProcess(Router),
}

pub struct InProcessOpts {
    pub chain_id: String,
    pub onboard_dao: Option<String>,
    pub onboard_runtime: Option<String>,
}

impl Default for InProcessOpts {
    fn default() -> Self {
        Self {
            chain_id: "terp-test-1".into(),
            onboard_dao: None,
            onboard_runtime: None,
        }
    }
}

impl HashMerchantClient {
    pub fn from_url(base: impl Into<String>) -> Self {
        Self {
            mode: Mode::Http(base.into().trim_end_matches('/').to_string()),
            _data: None,
        }
    }

    /// Merkle/BUD host, optional embedded `/onboard/*`. No VE, no second process.
    pub fn in_process(opts: InProcessOpts) -> Result<Self> {
        let dir = tempfile::tempdir().context("hashmerchant data dir")?;
        let mut state = AppState::new_merkle_host(opts.chain_id.clone(), dir.path().to_path_buf())?;
        if let (Some(dao), Some(rt_key)) = (opts.onboard_dao.clone(), opts.onboard_runtime.clone()) {
            state.enable_onboard_roles(dao, rt_key, opts.chain_id.clone());
        }
        let app = router(Arc::new(state));
        Ok(Self {
            mode: Mode::InProcess(app),
            _data: Some(Arc::new(dir)),
        })
    }

    pub fn in_process_merkle(chain_id: &str) -> Result<Self> {
        Self::in_process(InProcessOpts {
            chain_id: chain_id.into(),
            ..Default::default()
        })
    }

    pub fn in_process_with_onboard(chain_id: &str, dao: &str, runtime_key: &str) -> Result<Self> {
        Self::in_process(InProcessOpts {
            chain_id: chain_id.into(),
            onboard_dao: Some(dao.into()),
            onboard_runtime: Some(runtime_key.into()),
        })
    }

    pub async fn health(&self) -> Result<Value> {
        let (st, v) = self.json("GET", "/health", None).await?;
        anyhow::ensure!(st.is_success(), "health {st}");
        Ok(v)
    }

    pub async fn onboard_status(&self) -> Result<Value> {
        let (st, v) = self.json("GET", "/onboard/status", None).await?;
        anyhow::ensure!(st.is_success(), "onboard status {st}: {v}");
        Ok(v)
    }

    pub async fn request_register_grant(&self, address: &str) -> Result<(StatusCode, Value)> {
        self.json(
            "POST",
            "/onboard/v1/register-grant",
            Some(json!({ "address": address })),
        )
        .await
    }

    pub async fn request_mint_grant(&self, address: &str) -> Result<(StatusCode, Value)> {
        self.json(
            "POST",
            "/onboard/v1/mint-grant",
            Some(json!({ "address": address })),
        )
        .await
    }

    pub async fn upload_blob(&self, bytes: &[u8]) -> Result<Value> {
        let (st, body) = self
            .raw("POST", "/blobs", bytes, "application/octet-stream")
            .await?;
        anyhow::ensure!(st.is_success(), "blob upload {st}");
        serde_json::from_slice(&body).context("blob upload json")
    }

    pub async fn get_blob(&self, sha256_hex: &str) -> Result<(StatusCode, Vec<u8>)> {
        self.raw("GET", &format!("/blobs/{sha256_hex}"), &[], "")
            .await
    }

    pub async fn list_trees(&self) -> Result<Value> {
        let (st, v) = self.json("GET", "/trees", None).await?;
        anyhow::ensure!(st.is_success(), "list trees {st}");
        Ok(v)
    }

    pub async fn put_tree(&self, id: &str, body: Value) -> Result<StatusCode> {
        let (st, _) = self
            .json("POST", &format!("/trees/{id}"), Some(body))
            .await?;
        Ok(st)
    }

    pub async fn get_tree(&self, id: &str) -> Result<(StatusCode, Value)> {
        self.json("GET", &format!("/trees/{id}"), None).await
    }

    /// Probe a path (insanity cases / 404).
    pub async fn probe(&self, method: &str, path: &str) -> Result<StatusCode> {
        let (st, _) = self.raw(method, path, &[], "").await?;
        Ok(st)
    }


    pub async fn delete_blob(&self, sha256_hex: &str) -> Result<StatusCode> {
        self.probe("DELETE", &format!("/blobs/{sha256_hex}")).await
    }

    pub async fn delete_tree(&self, id: &str) -> Result<StatusCode> {
        self.probe("DELETE", &format!("/trees/{id}")).await
    }

    pub async fn member_proof(&self, tree_id: &str, addr: &str) -> Result<(StatusCode, Value)> {
        self.json("GET", &format!("/trees/{tree_id}/members/{addr}"), None)
            .await
    }

    pub async fn ve_health(&self) -> Result<(StatusCode, Value)> {
        self.json("GET", "/ve/health", None).await
    }

    pub async fn cashu_mints(&self) -> Result<(StatusCode, Value)> {
        self.json("GET", "/cashu/mints", None).await
    }

    pub async fn oracle_bounds(&self) -> Result<(StatusCode, Value)> {
        self.json("GET", "/oracle/bounds", None).await
    }

    pub async fn put_headstash(&self, id: &str, body: Value) -> Result<StatusCode> {
        let (st, _) = self
            .json("POST", &format!("/headstash/{id}"), Some(body))
            .await?;
        Ok(st)
    }

    pub async fn get_headstash(&self, id: &str) -> Result<(StatusCode, Value)> {
        self.json("GET", &format!("/headstash/{id}"), None).await
    }

    async fn json(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        let bytes = match &body {
            Some(v) => serde_json::to_vec(v)?,
            None => Vec::new(),
        };
        let ctype = if body.is_some() {
            "application/json"
        } else {
            ""
        };
        let (st, raw) = self.raw(method, path, &bytes, ctype).await?;
        if raw.is_empty() {
            return Ok((st, Value::Null));
        }
        let v = serde_json::from_slice(&raw).unwrap_or(Value::String(
            String::from_utf8_lossy(&raw).into_owned(),
        ));
        Ok((st, v))
    }

    async fn raw(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        content_type: &str,
    ) -> Result<(StatusCode, Vec<u8>)> {
        match &self.mode {
            Mode::InProcess(app) => {
                let mut b = Request::builder().method(method).uri(path);
                if !content_type.is_empty() {
                    b = b.header("content-type", content_type);
                }
                let req = b.body(Body::from(body.to_vec()))?;
                let res = app.clone().oneshot(req).await?;
                let st = res.status();
                let bytes = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024)
                    .await
                    .context("read body")?;
                Ok((st, bytes.to_vec()))
            }
            Mode::Http(base) => {
                let url = format!("{base}{path}");
                let client = reqwest::Client::new();
                let mut req = match method {
                    "GET" => client.get(&url),
                    "POST" => client.post(&url),
                    "DELETE" => client.delete(&url),
                    "PUT" => client.put(&url),
                    _ => anyhow::bail!("method {method}"),
                };
                if !content_type.is_empty() {
                    req = req.header("content-type", content_type);
                }
                if !body.is_empty() {
                    req = req.body(body.to_vec());
                }
                let res = req.send().await?;
                let st = StatusCode::from_u16(res.status().as_u16())?;
                let bytes = res.bytes().await?;
                Ok((st, bytes.to_vec()))
            }
        }
    }
}
