//! Parallel Headstash ETH pubkey scrape.
//!
//! Explorer lookups run concurrently; `eth_getTransactionByHash` is JSON-RPC **batched**.
//! RPC URLs rotate on 429 / Cloudflare / timeout (same pool as the JS scraper).
//!
//! ```bash
//! ETH_RPC_MODE=public ETH_SCRAPE_LIMIT=50 cargo run -p headstash-eth-pubkeys --release
//! ETH_ADDR=0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 cargo run -p headstash-eth-pubkeys --release
//! ```

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy_primitives::{B256, U256};
use alloy_rpc_types_eth::Transaction;
use anyhow::{Context, Result};
use k256::ecdsa::{RecoveryId, Signature as K256Sig, VerifyingKey};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use reqwest::Client; // ToEncodedPoint used by VerifyingKey::to_encoded_point
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore};

const DEFAULT_PUBLIC_RPCS: &[&str] = &[
    "https://public.1rpc.io/eth",
    "https://eth-mainnet.g.alchemy.com/public",
    "https://0xrpc.io/eth",
    // Execution JSON-RPC (not Beacon /eth/v1/). Cloudflare's POST endpoint is the origin.
    "https://cloudflare-eth.com",
];

const DEFAULT_EXPLORERS: &[&str] = &[
    "https://eth.blockscout.com/api/v2",
    "https://api.routescan.io/v2/network/mainnet/evm/1/etherscan/api",
    "https://eth.blockscout.com/api",
];

#[derive(Debug, Deserialize)]
struct HeadstashYaml {
    projects: Vec<Project>,
}

#[derive(Debug, Deserialize)]
struct Project {
    name: Option<String>,
    csv: String,
    #[serde(default = "evm")]
    chain_type: String,
}

fn evm() -> String {
    "evm".into()
}

struct RpcPool {
    urls: Vec<String>,
    disabled_until: Vec<AtomicU64>,
    ok: Vec<AtomicU64>,
    fail: Vec<AtomicU64>,
    client: Client,
}

impl RpcPool {
    fn new(urls: Vec<String>) -> Self {
        let n = urls.len();
        Self {
            urls,
            disabled_until: (0..n).map(|_| AtomicU64::new(0)).collect(),
            ok: (0..n).map(|_| AtomicU64::new(0)).collect(),
            fail: (0..n).map(|_| AtomicU64::new(0)).collect(),
            client: Client::builder()
                .timeout(Duration::from_secs(8))
                .connect_timeout(Duration::from_secs(3))
                .user_agent("headstash-eth-pubkeys/0.1")
                .build()
                .expect("reqwest"),
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn cooldown(&self, idx: usize, ms: u64) {
        self.disabled_until[idx].store(Self::now_ms() + ms, Ordering::Relaxed);
        self.fail[idx].fetch_add(1, Ordering::Relaxed);
    }

    fn mark_ok(&self, idx: usize) {
        self.ok[idx].fetch_add(1, Ordering::Relaxed);
    }

    fn score(&self, idx: usize) -> u64 {
        let ok = self.ok[idx].load(Ordering::Relaxed);
        let fail = self.fail[idx].load(Ordering::Relaxed);
        (ok + 1) * 1000 / (fail + 1)
    }

    fn ready_ranked(&self, skip: Option<usize>) -> Vec<(usize, String)> {
        let now = Self::now_ms();
        let mut v: Vec<(u64, usize)> = (0..self.urls.len())
            .filter(|&i| skip != Some(i) && self.disabled_until[i].load(Ordering::Relaxed) <= now)
            .map(|i| (self.score(i), i))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0));
        v.into_iter()
            .map(|(_, i)| (i, self.urls[i].clone()))
            .collect()
    }

    fn soonest_ready_ms(&self) -> u64 {
        let now = Self::now_ms();
        self.disabled_until
            .iter()
            .map(|a| a.load(Ordering::Relaxed).saturating_sub(now))
            .min()
            .unwrap_or(0)
    }

    fn is_hard_limit(status: u16, body: &str) -> bool {
        if matches!(status, 401 | 403 | 429 | 503) {
            return true;
        }
        let b = body.to_ascii_lowercase();
        b.contains("just a moment") || b.contains("<!doctype html")
    }

    fn parse_batch(text: &str, n: usize) -> Option<Vec<Option<Value>>> {
        let parsed: Value = serde_json::from_str(text).ok()?;
        let arr = if parsed.is_array() {
            parsed.as_array()?.clone()
        } else {
            vec![parsed]
        };
        let mut out = vec![None; n];
        let mut any = false;
        for item in arr {
            let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if id < n {
                if let Some(r) = item.get("result") {
                    out[id] = Some(r.clone());
                    any = true;
                }
            }
        }
        if any { Some(out) } else { None }
    }

    async fn post_batch(
        &self,
        idx: usize,
        url: &str,
        req: &Vec<Value>,
        n: usize,
    ) -> Result<Vec<Option<Value>>> {
        let resp = self.client.post(url).json(req).send().await?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        if Self::is_hard_limit(status, &text) {
            let wait = if status == 429 { 20_000 } else { 3_000 };
            self.cooldown(idx, wait);
            anyhow::bail!("{url} status={status}");
        }
        // HTTP 200 with JSON results: never cooldown (Alchemy often returns mixed item errors).
        match Self::parse_batch(&text, n) {
            Some(out) => {
                self.mark_ok(idx);
                Ok(out)
            }
            None => {
                self.cooldown(idx, 1_500);
                anyhow::bail!("{url} empty/unparsed json")
            }
        }
    }

    async fn batch_rpc(&self, calls: &[(String, Value)]) -> Result<Vec<Option<Value>>> {
        if calls.is_empty() {
            return Ok(vec![]);
        }
        let req: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(id, (method, params))| {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": params
                })
            })
            .collect();
        let n = calls.len();
        let mut last = anyhow::anyhow!("no rpc");
        let mut tried = 0u32;
        while tried < (self.urls.len() as u32).max(2) * 2 {
            tried += 1;
            let ranked = self.ready_ranked(None);
            if ranked.is_empty() {
                let wait = self.soonest_ready_ms().clamp(40, 3_000);
                tokio::time::sleep(Duration::from_millis(wait)).await;
                continue;
            }
            for (idx, url) in ranked {
                match self.post_batch(idx, &url, &req, n).await {
                    Ok(v) => return Ok(v),
                    Err(e) => last = e,
                }
            }
        }
        Err(last)
    }

    async fn batch_get_tx(&self, hashes: &[B256]) -> Result<Vec<Option<Value>>> {
        let calls: Vec<(String, Value)> = hashes
            .iter()
            .map(|h| ("eth_getTransactionByHash".into(), json!([h])))
            .collect();
        self.batch_rpc(&calls).await
    }

    async fn batch_get_code(&self, addrs: &[String]) -> Result<Vec<Option<Value>>> {
        let calls: Vec<(String, Value)> = addrs
            .iter()
            .map(|a| ("eth_getCode".into(), json!([a, "latest"])))
            .collect();
        self.batch_rpc(&calls).await
    }

    async fn batch_get_nonce(&self, addrs: &[String]) -> Result<Vec<Option<Value>>> {
        let calls: Vec<(String, Value)> = addrs
            .iter()
            .map(|a| ("eth_getTransactionCount".into(), json!([a, "latest"])))
            .collect();
        self.batch_rpc(&calls).await
    }

    /// One round-trip: getCode[i] then getTransactionCount[i] for each addr.
    async fn batch_code_and_nonce(&self, addrs: &[String]) -> Result<(Vec<Option<Value>>, Vec<Option<Value>>)> {
        let mut calls = Vec::with_capacity(addrs.len() * 2);
        for a in addrs {
            calls.push(("eth_getCode".into(), json!([a, "latest"])));
        }
        for a in addrs {
            calls.push(("eth_getTransactionCount".into(), json!([a, "latest"])));
        }
        let all = self.batch_rpc(&calls).await?;
        let n = addrs.len();
        let codes = all.iter().take(n).cloned().collect();
        let nonces = all.iter().skip(n).cloned().collect();
        Ok((codes, nonces))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeadReason {
    Contract,
    NeverSent,
    NoSignedTx,
    RecoverFailed,
    RecoverMismatch,
}

impl DeadReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::NeverSent => "never-sent",
            Self::NoSignedTx => "no-signed-tx",
            Self::RecoverFailed => "recover-failed",
            Self::RecoverMismatch => "recover-mismatch",
        }
    }
}

async fn explorer_get(client: &Client, url: &str) -> Result<reqwest::Response, ()> {
    match tokio::time::timeout(Duration::from_secs(6), client.get(url).send()).await {
        Ok(Ok(resp)) => Ok(resp),
        _ => Err(()),
    }
}

/// `Ok(Some)` hash, `Ok(None)` confirmed no outgoing txs, `Err` explorer busy/failed (retry later).
async fn explorer_outgoing_tx(client: &Client, address: &str) -> Result<Option<B256>, ()> {
    let extras: Vec<String> = std::env::var("ETH_EXPLORER_URLS")
        .ok()
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let bases: Vec<String> = if extras.is_empty() {
        DEFAULT_EXPLORERS.iter().map(|s| s.to_string()).collect()
    } else {
        extras
    };
    let key = std::env::var("ETHERSCAN_API_KEY").or_else(|_| std::env::var("ETHERSCAN_KEY")).unwrap_or_default();
    let addr_lc = address.to_ascii_lowercase();
    let mut saw_empty = false;
    let mut busy = false;

    for base in bases {
        let is_bs_v2 = base.contains("/api/v2") && !base.contains("etherscan");
        let url = if is_bs_v2 {
            format!(
                "{}/addresses/{}/transactions?filter=from",
                base.trim_end_matches('/'),
                address
            )
        } else {
            let mut url = format!(
                "{base}?module=account&action=txlist&address={address}&startblock=0&endblock=99999999&page=1&offset=10&sort=desc"
            );
            if !key.is_empty() {
                url.push_str("&apikey=");
                url.push_str(&key);
            }
            url
        };
        let Ok(resp) = explorer_get(client, &url).await else {
            busy = true;
            continue;
        };
        let status = resp.status().as_u16();
        if RpcPool::is_hard_limit(status, "") {
            busy = true;
            continue;
        }
        let Ok(v) = resp.json::<Value>().await else {
            busy = true;
            continue;
        };
        if is_bs_v2 {
            let items = v.get("items").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            for t in items {
                let from = t
                    .pointer("/from/hash")
                    .or_else(|| t.get("from"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if from == addr_lc {
                    if let Some(h) = t.get("hash").or_else(|| t.get("transaction_hash")).and_then(|x| x.as_str()) {
                        if let Ok(b) = h.parse::<B256>() {
                            return Ok(Some(b));
                        }
                    }
                }
            }
            saw_empty = true;
            continue;
        }
        if let Some(rows) = v.get("result").and_then(|x| x.as_array()) {
            for t in rows {
                let from = t.get("from").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
                if from == addr_lc {
                    if let Some(h) = t.get("hash").and_then(|x| x.as_str()) {
                        if let Ok(b) = h.parse::<B256>() {
                            return Ok(Some(b));
                        }
                    }
                }
            }
            saw_empty = true;
        } else if v.get("result").and_then(|x| x.as_str()).is_some_and(|s| s.to_ascii_lowercase().contains("rate")) {
            busy = true;
        } else {
            saw_empty = true;
        }
    }
    if saw_empty && !busy {
        Ok(None)
    } else {
        Err(())
    }
}

fn u256_be32(x: U256) -> [u8; 32] {
    x.to_be_bytes::<32>()
}

fn recover_pubkeys(tx_json: &Value) -> Option<(String, String, String)> {
    let tx: Transaction = serde_json::from_value(tx_json.clone()).ok()?;
    let recovered = &tx.inner;
    let from = format!("{:#x}", recovered.signer());
    let env = recovered.inner();
    let hash = env.signature_hash();
    let sig = env.signature();
    let recid = RecoveryId::try_from(u8::from(sig.v()) % 4).ok()?;
    let mut compact = [0u8; 64];
    compact[..32].copy_from_slice(&u256_be32(sig.r()));
    compact[32..].copy_from_slice(&u256_be32(sig.s()));
    let ksig = K256Sig::from_bytes((&compact).into()).ok()?;
    let vk = VerifyingKey::recover_from_prehash(hash.as_slice(), &ksig, recid).ok()?;
    let uncompressed = format!("0x{}", hex::encode(vk.to_encoded_point(false).as_bytes()));
    let compressed = format!("0x{}", hex::encode(vk.to_encoded_point(true).as_bytes()));
    Some((from, compressed, uncompressed))
}

fn same_addr(a: &str, b: &str) -> bool {
    a.trim_start_matches("0x").eq_ignore_ascii_case(b.trim_start_matches("0x"))
}

fn parse_hex_u64(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

fn is_contract_code(v: &Value) -> bool {
    match v.as_str() {
        None | Some("") | Some("0x") | Some("0x0") => false,
        Some(s) => s.len() > 2,
    }
}

fn rpc_mode() -> String {
    std::env::var("ETH_RPC_MODE").unwrap_or_else(|_| "auto".into()).to_ascii_lowercase()
}

fn rpc_urls() -> Vec<String> {
    if let Ok(list) = std::env::var("ETH_RPC_URLS") {
        let v: Vec<String> = list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if !v.is_empty() {
            return v;
        }
    }
    let local = std::env::var("ETH_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8545".into());
    let public: Vec<String> = DEFAULT_PUBLIC_RPCS.iter().map(|s| s.to_string()).collect();
    match rpc_mode().as_str() {
        "local" | "geth" | "self-hosted" => vec![local],
        "public" | "pg" => public,
        _ => {
            let mut u = vec![local];
            u.extend(public);
            u
        }
    }
}

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts-data")
}

fn norm_addr(s: &str) -> String {
    s.trim().trim_start_matches("0x").to_ascii_lowercase()
}

fn load_csv_addrs(path: &Path, pred: impl Fn(&[&str]) -> bool) -> HashSet<String> {
    let mut s = HashSet::new();
    let Ok(f) = File::open(path) else { return s };
    for (i, line) in BufReader::new(f).lines().enumerate() {
        let Ok(line) = line else { continue };
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if !pred(&cols) {
            continue;
        }
        if let Some(addr) = cols.first() {
            let n = norm_addr(addr);
            if n.len() == 40 {
                s.insert(n);
            }
        }
    }
    s
}

/// Addresses that already have a recovered pubkey — never query again.
fn load_have_pubkey(map_path: &Path) -> HashSet<String> {
    load_csv_addrs(map_path, |cols| {
        let pk = cols.get(1).map(|s| s.trim()).unwrap_or("");
        pk.starts_with("0x") && pk.len() >= 66
    })
}

/// Permanently unrecoverable (no secp key on mainnet). Transient misses are retried.
fn load_permanent_dead(dead_path: &Path) -> HashSet<String> {
    let retry_all = std::env::var("ETH_RETRY_DEAD").ok().as_deref() == Some("all");
    if retry_all {
        return HashSet::new();
    }
    load_csv_addrs(dead_path, |cols| {
        matches!(
            cols.get(1).map(|s| s.trim()),
            Some("contract" | "never-sent" | "recover-mismatch")
        )
    })
}

fn yaml_path() -> PathBuf {
    if let Ok(p) = std::env::var("HEADSTASH_YAML") {
        return PathBuf::from(p);
    }
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let c = here.join("../../../tools/headstash/circuit/headstash.yaml");
    if c.exists() { c } else { here.join("../../../tools/headstash/headstash.yaml") }
}

#[derive(Clone, Debug, Default)]
struct KeyRow {
    compressed: String,
    uncompressed: String,
    tx: String,
    status: String,
}

fn load_key_index(map_path: &Path, dead_path: &Path) -> HashMap<String, KeyRow> {
    let mut idx: HashMap<String, KeyRow> = HashMap::new();
    if let Ok(f) = File::open(dead_path) {
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let Ok(line) = line else { continue };
            if i == 0 || line.trim().is_empty() {
                continue;
            }
            let mut cols = line.split(',');
            let addr = cols.next().unwrap_or("");
            let reason = cols.next().unwrap_or("unknown").trim();
            idx.insert(
                norm_addr(addr),
                KeyRow {
                    status: reason.to_string(),
                    ..Default::default()
                },
            );
        }
    }
    if let Ok(f) = File::open(map_path) {
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let Ok(line) = line else { continue };
            if i == 0 || line.trim().is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split(',').collect();
            let addr = cols.first().copied().unwrap_or("");
            let compressed = cols.get(1).copied().unwrap_or("").trim().to_string();
            if !compressed.starts_with("0x") {
                continue;
            }
            idx.insert(
                norm_addr(addr),
                KeyRow {
                    compressed,
                    uncompressed: cols.get(2).copied().unwrap_or("").trim().to_string(),
                    tx: cols.get(3).copied().unwrap_or("").trim().to_string(),
                    status: "ok".into(),
                },
            );
        }
    }
    idx
}

fn community_csvs() -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let yp = yaml_path();
    if !yp.exists() {
        return Ok(out);
    }
    let raw = fs::read_to_string(&yp)?;
    let Ok(doc) = serde_yaml::from_str::<HeadstashYaml>(&raw) else {
        return Ok(out);
    };
    for p in doc.projects {
        if p.chain_type != "evm" {
            continue;
        }
        let name = p.name.clone().unwrap_or_else(|| p.csv.clone());
        if let Some(csv) = resolve_csv(&yp, &p.csv) {
            out.push((name, csv));
        }
    }
    Ok(out)
}

/// Write `<stem>.enriched.csv` beside each community snapshot.
/// Original `addr,amount` files are never overwritten.
fn enrich_community_csvs(map_path: &Path, dead_path: &Path) -> Result<usize> {
    let idx = load_key_index(map_path, dead_path);
    let extra = [
        "pubkey_compressed",
        "pubkey_uncompressed",
        "tx",
        "pubkey_status",
    ];
    let mut files = 0usize;
    let mut md = String::from(
        "# Headstash unallocated (burn) — EVM communities\n\n\
         Original snapshot CSVs (`addr,amount`) are **not** modified.\n\n\
         Rows without a recoverable secp256k1 pubkey cannot receive a Headstash note.\n\
         Their snapshot **amount** is documented here to **burn** rather than allocate.\n\
         Amounts are **per-community snapshot units** (NFT counts vs token balances) — do not sum across rows as one denom.\n\n\
         | Class | Meaning |\n\
         |-------|--------|\n\
         | `dead:contract` | `eth_getCode` nonempty |\n\
         | `dead:never-sent` | mainnet nonce 0 |\n\
         | `dead:recover-mismatch` | recovered signer ≠ holder |\n\
         | `blind:pending` | explorer/RPC did not return a signed tx this scrape |\n\
         | `blind:no-signed-tx` | confirmed no outgoing signed tx |\n\
         | `blind:recover-failed` | tx present but signature recover failed |\n\n\
         | Community | keyed | unallocated rows | unallocated amount | dead amount | blind amount |\n\
         |-----------|------:|-----------------:|-------------------:|------------:|-------------:|\n",
    );
    let mut grand_amt: f64 = 0.0;
    let mut grand_rows = 0usize;
    let mut grand_keyed = 0usize;
    let mut grand_all = 0usize;
    for (name, src) in community_csvs()? {
        if src.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.contains(".enriched.")) {
            continue;
        }
        let dest = src.with_file_name(format!(
            "{}.enriched.csv",
            src.file_stem().and_then(|s| s.to_str()).unwrap_or("community")
        ));
        let mut rdr = csv::Reader::from_path(&src)
            .with_context(|| format!("read {}", src.display()))?;
        let orig_headers: Vec<String> = rdr.headers()?.iter().map(|s| s.to_string()).collect();
        if orig_headers.iter().any(|h| h == "pubkey_compressed") {
            eprintln!("[enrich] skip already-keyed {}", src.display());
            continue;
        }
        let mut wtr = csv::Writer::from_path(&dest)
            .with_context(|| format!("write {}", dest.display()))?;
        let mut headers = orig_headers.clone();
        headers.extend(extra.iter().map(|s| s.to_string()));
        wtr.write_record(&headers)?;
        let mut keyed = 0usize;
        let mut rows = 0usize;
        let burn_path = src.with_file_name(format!(
            "{}.unallocated.csv",
            src.file_stem().and_then(|s| s.to_str()).unwrap_or("community")
        ));
        let mut burn = csv::Writer::from_path(&burn_path)?;
        burn.write_record(["addr", "amount", "class", "reason", "community"])?;
        let mut burn_amt: f64 = 0.0;
        let mut dead_amt: f64 = 0.0;
        let mut blind_amt: f64 = 0.0;
        let mut burn_rows = 0usize;
        for rec in rdr.records() {
            let rec = rec?;
            rows += 1;
            let addr = rec.get(0).unwrap_or("");
            let amount = rec.get(1).unwrap_or("0");
            let amt: f64 = amount.parse().unwrap_or(0.0);
            let hit = idx.get(&norm_addr(addr));
            let status = hit.map(|h| h.status.as_str()).unwrap_or("pending");
            if status == "ok" {
                keyed += 1;
            }
            let mut out: Vec<String> = rec.iter().map(|s| s.to_string()).collect();
            while out.len() < orig_headers.len() {
                out.push(String::new());
            }
            match hit {
                Some(k) if k.status == "ok" => {
                    out.push(k.compressed.clone());
                    out.push(k.uncompressed.clone());
                    out.push(k.tx.clone());
                    out.push(k.status.clone());
                }
                Some(k) => {
                    out.push(String::new());
                    out.push(String::new());
                    out.push(String::new());
                    out.push(k.status.clone());
                    let class = if matches!(k.status.as_str(), "contract" | "never-sent" | "recover-mismatch")
                    {
                        "dead"
                    } else {
                        "blind"
                    };
                    burn.write_record([addr, amount, class, &k.status, &name])?;
                    burn_amt += amt;
                    burn_rows += 1;
                    if class == "dead" {
                        dead_amt += amt;
                    } else {
                        blind_amt += amt;
                    }
                }
                None => {
                    out.extend(["", "", "", "pending"].map(String::from));
                    burn.write_record([addr, amount, "blind", "pending", &name])?;
                    burn_amt += amt;
                    blind_amt += amt;
                    burn_rows += 1;
                }
            }
            wtr.write_record(&out)?;
        }
        wtr.flush()?;
        burn.flush()?;
        files += 1;
        grand_amt += burn_amt;
        grand_rows += burn_rows;
        grand_keyed += keyed;
        grand_all += rows;
        md.push_str(&format!(
            "| {name} | {keyed}/{rows} | {burn_rows} | {burn_amt:.4} | {dead_amt:.4} | {blind_amt:.4} |\n"
        ));
        eprintln!(
            "[enrich] {name}: {keyed}/{rows} keyed, unallocated {burn_rows} amt={burn_amt} → {} + {}",
            dest.display(),
            burn_path.display()
        );
    }
    md.push_str(&format!(
        "\n**Total keyed {grand_keyed}/{grand_all}.** Unallocated **{grand_rows} rows**, amount **{grand_amt:.4}** (burn, not mint).\n"
    ));
    if let Some((_, first)) = community_csvs()?.first() {
        if let Some(dir) = first.parent().and_then(|p| p.parent()) {
            fs::write(dir.join("UNALLOCATED.md"), md)?;
            eprintln!("[enrich] wrote {}", dir.join("UNALLOCATED.md").display());
        }
    }
    Ok(files)
}

fn collect_addresses() -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(a) = std::env::var("ETH_ADDR") {
        let a = a.trim().to_string();
        if a.len() == 42 {
            seen.insert(a.to_ascii_lowercase());
            out.push(a);
        }
    }
    let yp = yaml_path();
    if yp.exists() {
        let raw = fs::read_to_string(&yp)?;
        if let Ok(doc) = serde_yaml::from_str::<HeadstashYaml>(&raw) {
            for p in doc.projects {
                if p.chain_type != "evm" {
                    continue;
                }
                let csv = resolve_csv(&yp, &p.csv);
                let Some(csv) = csv else {
                    eprintln!(
                        "[eth-pubkeys] missing csv {} ({})",
                        p.csv,
                        p.name.unwrap_or_default()
                    );
                    continue;
                };
                let mut rdr = csv::Reader::from_path(&csv)?;
                for rec in rdr.records() {
                    let rec = rec?;
                    let addr = rec.get(0).unwrap_or("").trim();
                    if addr.len() == 42 && seen.insert(addr.to_ascii_lowercase()) {
                        out.push(addr.to_string());
                    }
                }
            }
        }
    }
    if let Ok(extra) = std::env::var("ETH_ADDR_CSV") {
        if Path::new(&extra).exists() {
            let mut rdr = csv::Reader::from_path(&extra)?;
            for rec in rdr.records() {
                let rec = rec?;
                let addr = rec.get(0).unwrap_or("").trim();
                if addr.len() == 42 && seen.insert(addr.to_ascii_lowercase()) {
                    out.push(addr.to_string());
                }
            }
        }
    }
    if let Ok(lim) = std::env::var("ETH_SCRAPE_LIMIT") {
        if let Ok(n) = lim.parse::<usize>() {
            out.truncate(n);
        }
    }
    Ok(out)
}

fn resolve_csv(yaml: &Path, rel: &str) -> Option<PathBuf> {
    let yaml_dir = yaml.parent().unwrap_or(Path::new("."));
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let roots = [
        yaml_dir.join(rel),
        crate_dir.join(rel),
        crate_dir.join("../../../tools/headstash").join(rel),
        yaml_dir.join("headstash").join(rel.rsplit('/').next().unwrap_or(rel)),
        crate_dir.join("../../../tools/headstash/docs/circuit").join(rel),
    ];
    roots.into_iter().find(|p| p.exists())
}

#[tokio::main]
async fn main() -> Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let map_path = dir.join("address_pubkey_map.csv");
    let dead_path = dir.join("dead_addresses.csv");
    if std::env::args().any(|a| a == "--enrich-only") {
        let n = enrich_community_csvs(&map_path, &dead_path)?;
        eprintln!("[enrich] wrote {n} community *.enriched.csv (snapshots unchanged)");
        return Ok(());
    }
    if !map_path.exists() {
        fs::write(&map_path, "addr,pubkey_compressed,pubkey_uncompressed,tx\n")?;
    }
    if !dead_path.exists() {
        fs::write(&dead_path, "addr,reason\n")?;
    }
    let have_pk = load_have_pubkey(&map_path);
    let perm_dead = load_permanent_dead(&dead_path);
    let all = collect_addresses()?;
    let skip_n = all.len();
    let addrs: Vec<String> = all
        .into_iter()
        .filter(|a| {
            let n = norm_addr(a);
            !have_pk.contains(&n) && !perm_dead.contains(&n)
        })
        .collect();
    eprintln!(
        "[eth-pubkeys] holders={} skip have_pubkey={} permanent_dead={} query={}",
        skip_n,
        have_pk.len(),
        perm_dead.len(),
        addrs.len()
    );

    let conc: usize = std::env::var("ETH_SCRAPE_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);
    let batch: usize = std::env::var("ETH_RPC_BATCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let rpc_par: usize = std::env::var("ETH_RPC_PARALLEL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    let urls = rpc_urls();
    eprintln!(
        "[eth-pubkeys] mode={} rpcs={} remaining={} conc={} batch={} rpc_par={}",
        rpc_mode(),
        urls.len(),
        addrs.len(),
        conc,
        batch,
        rpc_par
    );
    let pool = Arc::new(RpcPool::new(urls));
    let http = pool.client.clone();
    let sem = Arc::new(Semaphore::new(conc));
    let map_file = Arc::new(Mutex::new(OpenOptions::new().append(true).open(&map_path)?));
    let dead_file = Arc::new(Mutex::new(OpenOptions::new().append(true).open(&dead_path)?));

    let t0 = Instant::now();
    let n_contract = Arc::new(AtomicUsize::new(0));
    let n_never = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(Mutex::new(Vec::new()));
    let rpc_sem = Arc::new(Semaphore::new(rpc_par.max(1)));

    let mut class_futs = Vec::new();
    for chunk in addrs.chunks(batch) {
        let chunk: Vec<String> = chunk.to_vec();
        let pool = pool.clone();
        let dead_file = dead_file.clone();
        let live = live.clone();
        let n_contract = n_contract.clone();
        let n_never = n_never.clone();
        let rpc_sem = rpc_sem.clone();
        class_futs.push(tokio::spawn(async move {
            let _g = rpc_sem.acquire().await.ok();
            let (codes, nonces) = match pool.batch_code_and_nonce(&chunk).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("classify batch: {e}");
                    return;
                }
            };
            let mut df = dead_file.lock().await;
            let mut lv = live.lock().await;
            for (i, addr) in chunk.iter().enumerate() {
                let contract = codes.get(i).and_then(|c| c.as_ref()).is_some_and(is_contract_code);
                let nonce = nonces
                    .get(i)
                    .and_then(|c| c.as_ref())
                    .and_then(parse_hex_u64)
                    .unwrap_or(u64::MAX);
                if contract {
                    let _ = writeln!(df, "{addr},{}", DeadReason::Contract.as_str());
                    n_contract.fetch_add(1, Ordering::Relaxed);
                } else if nonce == 0 {
                    let _ = writeln!(df, "{addr},{}", DeadReason::NeverSent.as_str());
                    n_never.fetch_add(1, Ordering::Relaxed);
                } else {
                    lv.push(addr.clone());
                }
            }
        }));
    }
    for f in class_futs {
        let _ = f.await;
    }
    let n_contract = n_contract.load(Ordering::Relaxed);
    let n_never = n_never.load(Ordering::Relaxed);
    let live = live.lock().await.clone();
    eprintln!(
        "[eth-pubkeys] classified contract={} never-sent={} live_eoa={}",
        n_contract,
        n_never,
        live.len()
    );

    let mut found: Vec<(String, B256)> = Vec::new();
    let mut no_tx = Vec::new();
    let mut deferred = 0usize;
    let mut futs = Vec::new();
    for addr in live {
        let http = http.clone();
        let sem = sem.clone();
        futs.push(tokio::spawn(async move {
            let _g = sem.acquire().await.ok();
            let h = explorer_outgoing_tx(&http, &addr).await;
            (addr, h)
        }));
    }
    for f in futs {
        match f.await {
            Ok((addr, Ok(Some(h)))) => found.push((addr, h)),
            Ok((addr, Ok(None))) => no_tx.push(addr),
            Ok((_, Err(()))) => deferred += 1,
            Err(e) => eprintln!("join: {e}"),
        }
    }
    {
        let mut df = dead_file.lock().await;
        for a in &no_tx {
            writeln!(df, "{a},{}", DeadReason::NoSignedTx.as_str())?;
        }
    }
    eprintln!(
        "[eth-pubkeys] explorer hash={} empty={} deferred_retry={}",
        found.len(),
        no_tx.len(),
        deferred
    );

    let ok = Arc::new(AtomicUsize::new(0));
    let n_fail = Arc::new(AtomicUsize::new(0));
    let n_mismatch = Arc::new(AtomicUsize::new(0));
    let mut tx_futs = Vec::new();
    for chunk in found.chunks(batch) {
        let chunk: Vec<(String, B256)> = chunk.to_vec();
        let pool = pool.clone();
        let map_file = map_file.clone();
        let dead_file = dead_file.clone();
        let ok = ok.clone();
        let n_fail = n_fail.clone();
        let n_mismatch = n_mismatch.clone();
        let rpc_sem = rpc_sem.clone();
        tx_futs.push(tokio::spawn(async move {
            let _g = rpc_sem.acquire().await.ok();
            let hashes: Vec<B256> = chunk.iter().map(|(_, h)| *h).collect();
            let txs = match pool.batch_get_tx(&hashes).await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("batch rpc: {e}");
                    n_fail.fetch_add(chunk.len(), Ordering::Relaxed);
                    return;
                }
            };
            let mut mf = map_file.lock().await;
            let mut df = dead_file.lock().await;
            for (i, (addr, hash)) in chunk.iter().enumerate() {
                let Some(Some(v)) = txs.get(i) else {
                    let _ = writeln!(df, "{addr},{}", DeadReason::RecoverFailed.as_str());
                    n_fail.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                if v.is_null() {
                    let _ = writeln!(df, "{addr},{}", DeadReason::RecoverFailed.as_str());
                    n_fail.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                match recover_pubkeys(v) {
                    Some((from, c, u)) if same_addr(&from, addr) => {
                        let _ = writeln!(mf, "{addr},{c},{u},{hash:#x}");
                        ok.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(_) => {
                        let _ = writeln!(df, "{addr},{}", DeadReason::RecoverMismatch.as_str());
                        n_mismatch.fetch_add(1, Ordering::Relaxed);
                    }
                    None => {
                        let _ = writeln!(df, "{addr},{}", DeadReason::RecoverFailed.as_str());
                        n_fail.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }
    for f in tx_futs {
        let _ = f.await;
    }
    let ok = ok.load(Ordering::Relaxed);
    let n_fail = n_fail.load(Ordering::Relaxed);
    let n_mismatch = n_mismatch.load(Ordering::Relaxed);

    let summary = serde_json::json!({
        "ok": ok,
        "contract": n_contract,
        "never_sent": n_never,
        "no_signed_tx": no_tx.len(),
        "explorer_deferred": deferred,
        "recover_failed": n_fail,
        "recover_mismatch": n_mismatch,
        "secs": t0.elapsed().as_secs_f64(),
        "map": map_path,
        "dead": dead_path,
    });
    fs::write(dir.join("pubkey_scrape_progress.json"), serde_json::to_vec_pretty(&summary)?)?;
    eprintln!("✅ rust scrape {summary}");
    match enrich_community_csvs(&map_path, &dead_path) {
        Ok(n) => eprintln!("[enrich] wrote {n} *.enriched.csv (original community CSVs unchanged)"),
        Err(e) => eprintln!("[enrich] failed: {e}"),
    }
    Ok(())
}
