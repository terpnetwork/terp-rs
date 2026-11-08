//! Ordered LCD endpoints.
//!
//! `TERP_NODE_REST` is a comma-separated list. The default is the house node.
//! An endpoint is refused when its chain id is not `morocco-1`, and skipped
//! when it refuses connections, returns 5xx, or lags the healthiest height
//! by more than 20 blocks. Each request waits at most 3 s. One failure retries
//! the next endpoint. The whole call shares a 6 s budget.
//!
//! `node_info` may be read up to the existing 256 KB ceiling because morocco-1's
//! `build_deps` list pushes `cosmos_sdk_version` past 32 KB. Every other body
//! still stops at 32 KB. Facts only keep the projected fields.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::Value;

use super::cache::Cache;
use super::cite::Cited;
use crate::terp::node;

const DEFAULT_REST: &str = "http://192.168.10.101:1317";
const CHAIN_ID: &str = "morocco-1";
pub(crate) const NODE_INFO_PATH: &str = "/cosmos/base/tendermint/v1beta1/node_info";
const BLOCKS_LATEST_PATH: &str = "/cosmos/base/tendermint/v1beta1/blocks/latest";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const TOTAL_BUDGET: Duration = Duration::from_secs(6);
const HEALTH_TTL: Duration = Duration::from_secs(30);
const HEIGHT_LAG_MAX: i64 = 20;
const RESPONSE_CAP: usize = 32 * 1024;
const NODE_INFO_CAP: usize = 256 * 1024;
const BLOCK_SCAN: usize = 8 * 1024;
const MAX_CALLS: u32 = 24;

/// Shared deadline for every LCD call inside one `facts_for` (or one tool).
pub struct Budget {
    deadline: Instant,
    calls: u32,
}

impl Budget {
    pub fn new() -> Self {
        Self {
            deadline: Instant::now() + TOTAL_BUDGET,
            calls: 0,
        }
    }

    fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    pub fn exhausted(&self) -> bool {
        self.expired() || self.calls >= MAX_CALLS
    }

    fn begin_call(&mut self) -> Result<()> {
        if self.calls >= MAX_CALLS {
            anyhow::bail!("LCD call cap spent");
        }
        if self.expired() {
            anyhow::bail!("chain query budget spent (6s)");
        }
        self.calls += 1;
        Ok(())
    }

    fn timeout(&self) -> Duration {
        let remain = self.deadline.saturating_duration_since(Instant::now());
        remain.min(REQUEST_TIMEOUT)
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct Ep {
    base: String,
    host: String,
    /// `Some(false)` is a permanent chain-id refusal.
    chain_ok: Option<bool>,
    height: Option<i64>,
    down_until: Option<Instant>,
    probed_at: Option<Instant>,
}

enum ReadMode {
    Json {
        limit: usize,
    },
    /// `blocks/latest` is a full block. Scan the first 8 KB for the header height.
    BlockHeight,
}

struct RawHttp {
    status: reqwest::StatusCode,
    body: Vec<u8>,
    height: Option<i64>,
}

enum Attempt {
    Retry(anyhow::Error),
    Stop(anyhow::Error),
}

pub struct Pool {
    eps: Mutex<Vec<Ep>>,
    cache: Cache,
    client: reqwest::Client,
}

impl Pool {
    pub fn from_env() -> Self {
        let raw = std::env::var("TERP_NODE_REST").unwrap_or_default();
        let bases = if raw.trim().is_empty() {
            vec![DEFAULT_REST.to_string()]
        } else {
            split_rest(&raw)
        };
        Self::from_bases(bases)
    }

    pub fn from_bases(bases: Vec<String>) -> Self {
        let eps = bases
            .into_iter()
            .filter_map(|base| {
                let base = normalize_base(&base)?;
                let host = host_only(&base);
                Some(Ep {
                    base,
                    host,
                    chain_ok: None,
                    height: None,
                    down_until: None,
                    probed_at: None,
                })
            })
            .collect();
        let client = reqwest::Client::builder()
            .connect_timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("reqwest client");
        Self {
            eps: Mutex::new(eps),
            cache: Cache::new(),
            client,
        }
    }

    pub async fn get_json(&self, path: &str, budget: &mut Budget) -> Result<Cited<Value>> {
        let clean = node::validate(path).map_err(|r| anyhow!("refused: {r}"))?;
        if let Some(hit) = self.cache.get(&clean) {
            return Ok(hit);
        }
        self.prepare(budget).await?;
        let order = self.candidates();
        if order.is_empty() {
            anyhow::bail!("no healthy morocco-1 endpoint");
        }
        let mut tries = 0u8;
        let mut last = anyhow!("no healthy morocco-1 endpoint");
        for idx in order {
            if tries >= 2 || budget.exhausted() {
                break;
            }
            match self.fetch_parsed(idx, &clean, budget).await {
                Ok(cited) => {
                    self.cache.put(&clean, &cited);
                    return Ok(cited);
                }
                Err(Attempt::Stop(e)) => return Err(e),
                Err(Attempt::Retry(e)) => {
                    tries += 1;
                    self.mark_down(idx);
                    last = e;
                }
            }
        }
        Err(last)
    }

    async fn prepare(&self, budget: &mut Budget) -> Result<()> {
        let n = lock(&self.eps).len();
        let mut errors: Vec<anyhow::Error> = Vec::new();
        for idx in 0..n {
            if self.skip_probe(idx) {
                continue;
            }
            if budget.exhausted() {
                break;
            }
            if let Err(e) = self.probe_one(idx, budget).await {
                errors.push(e);
            }
        }
        if self.candidates().is_empty() {
            if let Some(e) = errors.pop() {
                return Err(e);
            }
            anyhow::bail!("no healthy morocco-1 endpoint");
        }
        Ok(())
    }

    fn skip_probe(&self, idx: usize) -> bool {
        let eps = lock(&self.eps);
        let Some(ep) = eps.get(idx) else {
            return true;
        };
        let now = Instant::now();
        if ep.chain_ok == Some(false) || is_down(ep, now) {
            return true;
        }
        fresh_ok(ep, now)
    }

    async fn probe_one(&self, idx: usize, budget: &mut Budget) -> Result<()> {
        let base = self.base_host(idx).map(|(b, _)| b);
        let Some(base) = base else {
            anyhow::bail!("missing endpoint");
        };
        let raw = match self
            .http(&base, NODE_INFO_PATH, budget, json_mode(NODE_INFO_PATH))
            .await
        {
            Ok(raw) => raw,
            Err(Attempt::Retry(e)) | Err(Attempt::Stop(e)) => {
                self.mark_down(idx);
                return Err(e);
            }
        };
        if raw.status.is_server_error() || raw.status.is_redirection() || !raw.status.is_success() {
            self.mark_down(idx);
            anyhow::bail!("{} node_info HTTP {}", self.host_of(idx), raw.status);
        }
        let value: Value = serde_json::from_slice(&raw.body).map_err(|e| {
            self.mark_down(idx);
            anyhow!("{} node_info: {e}", self.host_of(idx))
        })?;
        let network = chain_id_of(&value).unwrap_or("");
        if network != CHAIN_ID {
            self.mark_refused(idx);
            anyhow::bail!("endpoint refused: chain-id {network} is not {CHAIN_ID}");
        }
        let mut height = raw.height;
        if height.is_none() {
            height = self.block_height(&base, budget).await;
        }
        self.mark_ok(idx, height);
        Ok(())
    }

    async fn fetch_parsed(
        &self,
        idx: usize,
        path: &str,
        budget: &mut Budget,
    ) -> Result<Cited<Value>, Attempt> {
        let Some((base, host)) = self.base_host(idx) else {
            return Err(Attempt::Stop(anyhow!("missing endpoint")));
        };
        let raw = self.http(&base, path, budget, json_mode(path)).await?;
        if raw.status.is_server_error() || raw.status.is_redirection() {
            return Err(Attempt::Retry(anyhow!("{host} {path} HTTP {}", raw.status)));
        }
        if !raw.status.is_success() {
            return Err(Attempt::Stop(anyhow!("{host} {path} HTTP {}", raw.status)));
        }
        let value: Value = serde_json::from_slice(&raw.body)
            .map_err(|e| Attempt::Stop(anyhow!("{host} {path}: {e}")))?;
        let mut height = raw.height;
        if height.is_none() {
            height = self.block_height(&base, budget).await;
        }
        if height.is_none() {
            height = lock(&self.eps).get(idx).and_then(|e| e.height);
        }
        if let Some(h) = height {
            self.note_height(idx, h);
        }
        Ok(Cited {
            data: value,
            height: height.unwrap_or(0),
            endpoint: host,
            path: path.to_string(),
            fetched_at: Utc::now(),
            cached: false,
        })
    }

    async fn block_height(&self, base: &str, budget: &mut Budget) -> Option<i64> {
        if budget.exhausted() {
            return None;
        }
        let raw = self
            .http(base, BLOCKS_LATEST_PATH, budget, ReadMode::BlockHeight)
            .await
            .ok()?;
        if !raw.status.is_success() {
            return raw.height;
        }
        raw.height
            .or_else(|| height_in_block_body(&String::from_utf8_lossy(&raw.body)))
    }

    async fn http(
        &self,
        base: &str,
        path: &str,
        budget: &mut Budget,
        mode: ReadMode,
    ) -> Result<RawHttp, Attempt> {
        budget.begin_call().map_err(Attempt::Stop)?;
        let timeout = budget.timeout();
        if timeout < Duration::from_millis(50) {
            return Err(Attempt::Stop(anyhow!("chain query budget spent (6s)")));
        }
        let url = format!("{}{path}", base.trim_end_matches('/'));
        let resp = self
            .client
            .get(&url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| Attempt::Retry(anyhow!("{url}: {e}")))?;
        let status = resp.status();
        let height = header_height(&resp);
        let body = read_body(resp, mode).await?;
        Ok(RawHttp {
            status,
            body,
            height,
        })
    }

    fn candidates(&self) -> Vec<usize> {
        let eps = lock(&self.eps);
        let now = Instant::now();
        let heights: Vec<Option<i64>> = eps
            .iter()
            .map(|e| {
                if e.chain_ok == Some(true) {
                    e.height
                } else {
                    None
                }
            })
            .collect();
        let lag = lagging(&heights);
        eps.iter()
            .enumerate()
            .filter(|(i, e)| e.chain_ok == Some(true) && !is_down(e, now) && !lag[*i])
            .map(|(i, _)| i)
            .collect()
    }

    fn base_host(&self, idx: usize) -> Option<(String, String)> {
        lock(&self.eps)
            .get(idx)
            .map(|e| (e.base.clone(), e.host.clone()))
    }

    fn host_of(&self, idx: usize) -> String {
        lock(&self.eps)
            .get(idx)
            .map(|e| e.host.clone())
            .unwrap_or_default()
    }

    fn mark_down(&self, idx: usize) {
        if let Some(ep) = lock(&self.eps).get_mut(idx) {
            ep.down_until = Some(Instant::now() + HEALTH_TTL);
            ep.probed_at = Some(Instant::now());
        }
    }

    fn mark_refused(&self, idx: usize) {
        if let Some(ep) = lock(&self.eps).get_mut(idx) {
            ep.chain_ok = Some(false);
            ep.down_until = None;
            ep.probed_at = Some(Instant::now());
        }
    }

    fn mark_ok(&self, idx: usize, height: Option<i64>) {
        if let Some(ep) = lock(&self.eps).get_mut(idx) {
            ep.chain_ok = Some(true);
            ep.height = height.or(ep.height);
            ep.down_until = None;
            ep.probed_at = Some(Instant::now());
        }
    }

    fn note_height(&self, idx: usize, height: i64) {
        if let Some(ep) = lock(&self.eps).get_mut(idx) {
            ep.height = Some(height);
        }
    }
}

pub fn global() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Pool::from_env)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn fresh_ok(ep: &Ep, now: Instant) -> bool {
    ep.chain_ok == Some(true)
        && ep
            .probed_at
            .is_some_and(|t| now.saturating_duration_since(t) < HEALTH_TTL)
}

fn is_down(ep: &Ep, now: Instant) -> bool {
    ep.down_until.is_some_and(|t| t > now)
}

fn json_mode(path: &str) -> ReadMode {
    let path = path.split('?').next().unwrap_or(path);
    let limit = if path == NODE_INFO_PATH {
        NODE_INFO_CAP
    } else {
        RESPONSE_CAP
    };
    ReadMode::Json { limit }
}

fn split_rest(raw: &str) -> Vec<String> {
    raw.split(',').filter_map(normalize_base).collect()
}

fn normalize_base(raw: &str) -> Option<String> {
    let s = raw.trim().trim_end_matches('/');
    if s.starts_with("http://") || s.starts_with("https://") {
        Some(s.to_string())
    } else {
        None
    }
}

pub(crate) fn host_only(base: &str) -> String {
    let rest = base.split("://").nth(1).unwrap_or(base);
    rest.split('/').next().unwrap_or(rest).to_string()
}

fn chain_id_of(v: &Value) -> Option<&str> {
    v.pointer("/default_node_info/network")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            v.pointer("/node_info/network")
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
        })
}

fn header_height(resp: &reqwest::Response) -> Option<i64> {
    resp.headers()
        .get("x-cosmos-block-height")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}

fn height_in_block_body(body: &str) -> Option<i64> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(h) = block_header_height(&v) {
            return Some(h);
        }
    }
    let header_at = body.find("\"header\"")?;
    let slice = &body[header_at..];
    let key_at = slice.find("\"height\"")?;
    let rest = slice[key_at + "\"height\"".len()..].trim_start();
    let rest = rest.trim_start_matches(':').trim_start();
    let rest = rest.trim_start_matches('"');
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if num.is_empty() {
        None
    } else {
        num.parse().ok()
    }
}

fn block_header_height(v: &Value) -> Option<i64> {
    let h = v.pointer("/block/header/height")?;
    h.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| h.as_i64())
        .or_else(|| h.as_u64().and_then(|n| i64::try_from(n).ok()))
}

/// `true` when `height` is more than 20 behind the max. A gap of exactly 20 stays healthy.
fn lagging(heights: &[Option<i64>]) -> Vec<bool> {
    let max = heights.iter().flatten().copied().max();
    heights
        .iter()
        .map(|h| match (h, max) {
            (Some(h), Some(max)) => max - h > HEIGHT_LAG_MAX,
            _ => false,
        })
        .collect()
}

async fn read_body(mut resp: reqwest::Response, mode: ReadMode) -> Result<Vec<u8>, Attempt> {
    let (limit, allow_short) = match mode {
        ReadMode::Json { limit } => (limit, false),
        ReadMode::BlockHeight => (BLOCK_SCAN, true),
    };
    if !allow_short {
        if let Some(len) = resp.content_length() {
            if len > limit as u64 {
                return Err(Attempt::Stop(anyhow!("response too large ({len} bytes)")));
            }
        }
    }
    let mut buf = Vec::new();
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| Attempt::Retry(anyhow!("reading body: {e}")))?;
        let Some(chunk) = chunk else {
            break;
        };
        if buf.len() + chunk.len() > limit {
            if allow_short {
                let room = limit.saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..room.min(chunk.len())]);
                break;
            }
            return Err(Attempt::Stop(anyhow!("response exceeds {limit} bytes")));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[cfg(test)]
pub mod harness {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Clone)]
    pub struct Route {
        pub status: u16,
        pub body: String,
        pub height: Option<i64>,
    }

    pub struct Server {
        pub url: String,
        pub hits: Arc<Mutex<Vec<String>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    pub fn route(status: u16, body: impl Into<String>, height: Option<i64>) -> Route {
        Route {
            status,
            body: body.into(),
            height,
        }
    }

    pub async fn serve(routes: HashMap<String, Route>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let hits_task = hits.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    break;
                };
                let routes = routes.clone();
                let hits = hits_task.clone();
                tokio::spawn(async move {
                    let _ = handle(sock, routes, hits).await;
                });
            }
        });
        tokio::task::yield_now().await;
        Server {
            url: format!("http://127.0.0.1:{port}"),
            hits,
            task,
        }
    }

    pub async fn closed_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        port
    }

    async fn handle(
        mut sock: tokio::net::TcpStream,
        routes: HashMap<String, Route>,
        hits: Arc<Mutex<Vec<String>>>,
    ) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let n = sock.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let text = String::from_utf8_lossy(&buf);
        let line = text.lines().next().unwrap_or("");
        let path = line.split_whitespace().nth(1).unwrap_or("/");
        lock(&hits).push(path.to_string());
        let route = routes.get(path).cloned().unwrap_or(Route {
            status: 404,
            body: "{}".into(),
            height: None,
        });
        let reason = if route.status == 200 { "OK" } else { "ERR" };
        let mut head = format!(
            "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            route.status,
            route.body.len()
        );
        if let Some(h) = route.height {
            head.push_str(&format!("x-cosmos-block-height: {h}\r\n"));
        }
        head.push_str("\r\n");
        sock.write_all(head.as_bytes()).await?;
        sock.write_all(route.body.as_bytes()).await?;
        let _ = sock.shutdown().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::harness::{closed_port, route, serve};
    use super::*;

    fn node_info(network: &str) -> String {
        format!(
            r#"{{"default_node_info":{{"network":"{network}"}},"application_version":{{"version":"6.2.1","cosmos_sdk_version":"v0.55.0","build_deps":[]}}}}"#
        )
    }

    fn supply_body() -> &'static str {
        r#"{"supply":[{"denom":"uterp","amount":"10"}]}"#
    }

    #[test]
    fn splits_rest_and_host() {
        assert_eq!(
            split_rest(" http://192.168.10.101:1317/, https://terp.api.nodex.one "),
            vec![
                "http://192.168.10.101:1317".to_string(),
                "https://terp.api.nodex.one".to_string()
            ]
        );
        assert!(split_rest("ftp://nope").is_empty());
        assert_eq!(
            host_only("https://terp.api.nodex.one/lcd"),
            "terp.api.nodex.one"
        );
        assert_eq!(
            host_only("http://192.168.10.101:1317"),
            "192.168.10.101:1317"
        );
    }

    #[test]
    fn height_lag_boundary_is_exclusive() {
        assert_eq!(lagging(&[Some(100), Some(120)]), vec![false, false]);
        assert_eq!(lagging(&[Some(100), Some(121)]), vec![true, false]);
        assert_eq!(lagging(&[Some(130), Some(100)]), vec![false, true]);
        assert_eq!(lagging(&[None, Some(50)]), vec![false, false]);
    }

    #[test]
    fn reads_height_from_a_truncated_block() {
        let body = r#"{"block_id":{"hash":"aa"},"block":{"header":{"height":"42","time":"x""#;
        assert_eq!(height_in_block_body(body), Some(42));
        let whole = r#"{"block":{"header":{"height":"7"}}}"#;
        assert_eq!(height_in_block_body(whole), Some(7));
    }

    #[tokio::test]
    async fn primary_refused_uses_secondary_once() {
        let dead = closed_port().await;
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), Some(200)),
        );
        routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(200, supply_body(), Some(200)),
        );
        let secondary = serve(routes).await;
        let pool = Pool::from_bases(vec![
            format!("http://127.0.0.1:{dead}"),
            secondary.url.clone(),
        ]);
        let mut budget = Budget::new();
        let cited = pool
            .get_json("/cosmos/bank/v1beta1/supply", &mut budget)
            .await
            .expect("secondary");
        assert_eq!(cited.data["supply"][0]["denom"], "uterp");
        assert!(!cited.cached);
        let hits = lock(&secondary.hits);
        assert_eq!(
            hits.iter()
                .filter(|p| p.starts_with("/cosmos/bank"))
                .count(),
            1
        );
        assert!(hits.iter().any(|p| p.contains("node_info")));
    }

    #[tokio::test]
    async fn primary_500_retries_the_next_endpoint_once() {
        let mut primary_routes = HashMap::new();
        primary_routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), Some(200)),
        );
        primary_routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(500, "{}", Some(200)),
        );
        let primary = serve(primary_routes).await;
        let mut secondary_routes = HashMap::new();
        secondary_routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), Some(210)),
        );
        secondary_routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(200, supply_body(), Some(210)),
        );
        let secondary = serve(secondary_routes).await;
        let pool = Pool::from_bases(vec![primary.url.clone(), secondary.url.clone()]);
        let mut budget = Budget::new();
        let cited = pool
            .get_json("/cosmos/bank/v1beta1/supply", &mut budget)
            .await
            .expect("failover");
        assert_eq!(cited.endpoint, host_only(&secondary.url));
        assert_eq!(
            lock(&primary.hits)
                .iter()
                .filter(|p| p.starts_with("/cosmos/bank"))
                .count(),
            1
        );
        assert_eq!(
            lock(&secondary.hits)
                .iter()
                .filter(|p| p.starts_with("/cosmos/bank"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn wrong_chain_id_is_refused() {
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info("osmo-test-5"), Some(10)),
        );
        routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(200, supply_body(), Some(10)),
        );
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);
        let mut budget = Budget::new();
        let err = pool
            .get_json("/cosmos/bank/v1beta1/supply", &mut budget)
            .await
            .expect_err("refused");
        let msg = err.to_string();
        assert!(msg.contains("refused"), "{msg}");
        assert!(msg.contains("morocco-1"), "{msg}");
        assert!(lock(&server.hits)
            .iter()
            .all(|p| !p.starts_with("/cosmos/bank")));
    }

    #[tokio::test]
    async fn stale_height_endpoint_is_skipped() {
        let mut primary_routes = HashMap::new();
        primary_routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), Some(100)),
        );
        primary_routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(
                200,
                r#"{"supply":[{"denom":"stale","amount":"1"}]}"#,
                Some(100),
            ),
        );
        let primary = serve(primary_routes).await;
        let mut secondary_routes = HashMap::new();
        secondary_routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), Some(130)),
        );
        secondary_routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(200, supply_body(), Some(130)),
        );
        let secondary = serve(secondary_routes).await;
        let pool = Pool::from_bases(vec![primary.url.clone(), secondary.url.clone()]);
        let mut budget = Budget::new();
        let cited = pool
            .get_json("/cosmos/bank/v1beta1/supply", &mut budget)
            .await
            .expect("fresh");
        assert_eq!(cited.data["supply"][0]["denom"], "uterp");
        assert_eq!(cited.height, 130);
        assert!(
            lock(&primary.hits)
                .iter()
                .all(|p| !p.starts_with("/cosmos/bank")),
            "stale data path was used"
        );
    }

    #[tokio::test]
    async fn second_read_is_cached() {
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), Some(50)),
        );
        routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(200, supply_body(), Some(50)),
        );
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);
        let mut budget = Budget::new();
        let first = pool
            .get_json("/cosmos/bank/v1beta1/supply", &mut budget)
            .await
            .unwrap();
        let second = pool
            .get_json("/cosmos/bank/v1beta1/supply", &mut budget)
            .await
            .unwrap();
        assert!(!first.cached);
        assert!(second.cached);
        assert_eq!(
            lock(&server.hits)
                .iter()
                .filter(|p| p.starts_with("/cosmos/bank"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn over_32kb_is_refused_except_node_info_height_fallback() {
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), None),
        );
        routes.insert(
            BLOCKS_LATEST_PATH.to_string(),
            route(200, r#"{"block":{"header":{"height":"42"}}}"#, None),
        );
        let fat = format!("{{\"pad\":\"{}\"}}", "a".repeat(40_000));
        routes.insert(
            "/cosmos/bank/v1beta1/supply".into(),
            route(200, fat, Some(42)),
        );
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);
        let mut budget = Budget::new();
        let err = pool
            .get_json("/cosmos/bank/v1beta1/supply", &mut budget)
            .await
            .expect_err("cap");
        assert!(
            err.to_string().contains("too large") || err.to_string().contains("exceeds"),
            "{err}"
        );

        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), None),
        );
        routes.insert(
            BLOCKS_LATEST_PATH.to_string(),
            route(200, r#"{"block":{"header":{"height":"42"}}}"#, None),
        );
        routes.insert(
            "/cosmos/staking/v1beta1/pool".into(),
            route(
                200,
                r#"{"pool":{"bonded_tokens":"1","not_bonded_tokens":"2"}}"#,
                None,
            ),
        );
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);
        let mut budget = Budget::new();
        let cited = pool
            .get_json("/cosmos/staking/v1beta1/pool", &mut budget)
            .await
            .expect("height");
        assert_eq!(cited.height, 42);
    }

    #[tokio::test]
    async fn validate_refusals_never_hit_the_network() {
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, node_info(CHAIN_ID), Some(1)),
        );
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);
        let mut budget = Budget::new();
        for path in [
            "/cosmos/tx/v1beta1/txs",
            "/cosmos/bank/v1beta1/supply?height=5",
            "/../etc/passwd",
        ] {
            let err = pool.get_json(path, &mut budget).await.expect_err(path);
            assert!(err.to_string().contains("refused"), "{path} {err}");
        }
        assert!(lock(&server.hits).is_empty());
    }
}
