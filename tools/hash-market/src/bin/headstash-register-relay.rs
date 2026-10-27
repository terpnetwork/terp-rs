//! Headstash registration sidecar ("submit to any sidecar").
//!
//! Any sidecar accepts holders' signed eligibility registrations over HTTP,
//! verifies them cheaply, and batches them into its own transactions: one tx
//! carrying several `RegisterHeadstash` / `RegisterHeadstashSol` messages, sent
//! from the sidecar's own account with a locally ordered sequence. No fee
//! grants, no fresh wallets. cw-headstash re-verifies every signature and
//! eligibility path, so a sidecar cannot forge a registration; it can only
//! refuse or delay one, which is why the page falls back across sidecars.
//!
//! Pipeline per request (cheapest first):
//!   body cap -> JSON/format -> ctx == campaign ctx -> rate limits ->
//!   native signature verify (secp256k1 recover / ed25519 strict) ->
//!   eligibility path against the configured root (== on-chain root) ->
//!   on-chain `registered` query (cached) -> dedupe by identity -> queue.
//! One broadcaster task flushes every `FLUSH_SECS` or at `MAX_BATCH`.
//!
//! Binds 127.0.0.1 only (put the single nginx in front). The operator key is
//! the existing keyring reference (`--from` + `--home` + backend) handed to
//! `terpd tx sign`; this process never reads key material. It does not log
//! bodies, signatures, or key commitments.
//!
//! Direct-path fee grants (`POST /headstash/feegrant`, same process, same
//! account, same tx serialisation as the batcher): a fresh wallet asks for gas
//! for one `ClaimNote` (or one `RegisterNotes` chunk). The sidecar first runs
//! the contract's own read-only dry-run (`check_note_claim` with dest = the
//! grantee, or `check_register_notes`), so it grants only for a proof that
//! verifies against an accepted anchor with an unspent nullifier, or a valid
//! eligible signature. One grant per nullifier / per (signer, chunk) and per
//! grantee; per-IP (in memory only, never logged) and global rate limits. The
//! allowance is `AllowedMsgAllowance[MsgExecuteContract]` over a
//! `BasicAllowance` whose spend limit is one tx fee with a short expiry.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use ff::PrimeField;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use pasta_curves::pallas;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};
use tokio::sync::{Mutex, Notify};

const DEFAULT_BIND: &str = "127.0.0.1:8791";
const DEFAULT_ORIGIN: &str = "http://127.0.0.1:3921";
const BODY_LIMIT: usize = 4096;
const GRANT_BODY_LIMIT: usize = 64 * 1024;
const SOL_LEAF_TAG: u8 = 0x02;

// ---------------------------------------------------------------- types

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Evm { address: [u8; 20], signature: [u8; 65] },
    Sol { pubkey: [u8; 32], signature: [u8; 64] },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reg {
    kind: Kind,
    key_com: [u8; 32],
    amount: u64,
    index: u32,
    path: Vec<[u8; 32]>,
}

impl Reg {
    fn identity(&self) -> String {
        match &self.kind {
            Kind::Evm { address, .. } => hex::encode(address),
            Kind::Sol { pubkey, .. } => format!("sol:{}", hex::encode(pubkey)),
        }
    }

    fn execute_msg(&self) -> Value {
        let path: Vec<String> = self.path.iter().map(|s| b64(s)).collect();
        match &self.kind {
            Kind::Evm { signature, .. } => json!({"register_headstash": {
                "key_com": b64(&self.key_com), "amount": self.amount,
                "signature": b64(signature),
                "eligibility_index": self.index, "eligibility_path": path }}),
            Kind::Sol { pubkey, signature } => json!({"register_headstash_sol": {
                "key_com": b64(&self.key_com), "amount": self.amount,
                "pubkey": b64(pubkey), "signature": b64(signature),
                "eligibility_index": self.index, "eligibility_path": path }}),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Submitted { txhash: String },
    Registered { txhash: Option<String>, height: Option<u64> },
    Failed { error: String },
}

impl Status {
    fn to_json(&self, id: &str) -> Value {
        match self {
            Status::Queued => json!({"id": id, "status": "queued"}),
            Status::Submitted { txhash } => json!({"id": id, "status": "submitted", "txhash": txhash}),
            Status::Registered { txhash, height } => {
                json!({"id": id, "status": "registered", "txhash": txhash, "height": height})
            }
            Status::Failed { error } => json!({"id": id, "status": "failed", "error": error}),
        }
    }
}

#[derive(Debug)]
pub enum Reject {
    Bad(&'static str),
    Rate,
    Full,
}

// ---------------------------------------------------------------- chain

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainErr {
    /// Simulation failed for message `index` (if the node said which).
    Msg { index: Option<usize>, log: String },
    Sequence,
    Other(String),
}

pub struct TxResult {
    pub code: u32,
    pub height: u64,
    pub log: String,
}

#[async_trait]
pub trait Chain: Send + Sync + 'static {
    async fn registered(&self, identity: &str) -> Result<bool, String>;
    async fn sequence(&self) -> Result<(u64, u64), String>;
    /// Returns gas used.
    async fn simulate(&self, msgs: &[Value], acct: u64, seq: u64) -> Result<u64, ChainErr>;
    /// Returns txhash once CheckTx accepted it.
    async fn broadcast(&self, msgs: &[Value], gas: u64, acct: u64, seq: u64) -> Result<String, ChainErr>;
    async fn wait(&self, txhash: &str) -> Result<TxResult, String>;
    /// Contract dry-run query that must return `true`; Err carries the chain reason.
    async fn check(&self, q: &Value) -> Result<(), String>;
}

// ---------------------------------------------------------------- eligibility

pub struct Elig {
    depth: u32,
    levels: Vec<Vec<[u8; 32]>>,
    index: HashMap<String, (u32, u64)>,
}

impl Elig {
    pub fn load(bytes: &[u8]) -> anyhow::Result<Self> {
        let v: Value = serde_json::from_slice(bytes)?;
        let depth = v.get("depth").and_then(Value::as_u64).ok_or_else(|| anyhow::anyhow!("eligibility depth"))? as u32;
        if !(1..=16).contains(&depth) {
            anyhow::bail!("eligibility depth");
        }
        let rows = v.get("entries").and_then(Value::as_array).ok_or_else(|| anyhow::anyhow!("eligibility entries"))?;
        if rows.len() > 1usize << depth {
            anyhow::bail!("eligibility entries exceed depth");
        }
        let mut leaves = vec![[0u8; 32]; 1usize << depth];
        let mut index = HashMap::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let amount = row.get("amount").and_then(Value::as_u64).filter(|a| *a > 0).ok_or_else(|| anyhow::anyhow!("eligibility amount"))?;
            let (id, leaf) = if let Some(a) = row.get("address_hex").and_then(Value::as_str) {
                let a: [u8; 20] = decode_hex_n(a, 20).ok_or_else(|| anyhow::anyhow!("eligibility address"))?.try_into().unwrap();
                (hex::encode(a), evm_leaf(&a, amount))
            } else if let Some(p) = row.get("sol_pubkey_hex").and_then(Value::as_str) {
                let p: [u8; 32] = decode_hex_n(p, 32).ok_or_else(|| anyhow::anyhow!("eligibility pubkey"))?.try_into().unwrap();
                (format!("sol:{}", hex::encode(p)), sol_leaf(&p, amount))
            } else {
                anyhow::bail!("eligibility row");
            };
            if index.insert(id, (i as u32, amount)).is_some() {
                anyhow::bail!("eligibility duplicate identity");
            }
            leaves[i] = leaf;
        }
        Ok(Self::from_leaves(depth, leaves, index))
    }

    fn from_leaves(depth: u32, leaves: Vec<[u8; 32]>, index: HashMap<String, (u32, u64)>) -> Self {
        let mut levels = vec![leaves];
        for _ in 0..depth {
            let next = levels.last().unwrap().chunks(2).map(|p| elig_node(&p[0], &p[1])).collect();
            levels.push(next);
        }
        Self { depth, levels, index }
    }

    pub fn root(&self) -> [u8; 32] {
        self.levels[self.depth as usize][0]
    }

    /// Index + path when `identity` is eligible for exactly `amount`.
    fn lookup(&self, identity: &str, amount: u64) -> Option<(u32, Vec<[u8; 32]>)> {
        let (i, a) = *self.index.get(identity)?;
        if a != amount {
            return None;
        }
        let path = (0..self.depth as usize).map(|l| self.levels[l][((i as usize) >> l) ^ 1]).collect();
        Some((i, path))
    }
}

pub fn evm_leaf(address: &[u8; 20], amount: u64) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-leaf-v1".to_vec();
    pre.extend_from_slice(address);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

pub fn sol_leaf(pubkey: &[u8; 32], amount: u64) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-leaf-v1".to_vec();
    pre.push(SOL_LEAF_TAG);
    pre.extend_from_slice(pubkey);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

pub fn elig_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-node-v1".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    sha256(&pre)
}

fn open_path(mut node: [u8; 32], index: u32, path: &[[u8; 32]]) -> [u8; 32] {
    for (l, s) in path.iter().enumerate() {
        node = if (index >> l) & 1 == 0 { elig_node(&node, s) } else { elig_node(s, &node) };
    }
    node
}

// ---------------------------------------------------------------- verify

pub fn register_message(ctx: &[u8], key_com: &[u8], amount: u64) -> String {
    format!("headstash-register-v1:{}:{}:{amount}", hex::encode(ctx), hex::encode(key_com))
}

pub fn register_sol_message(ctx: &[u8], key_com: &[u8], amount: u64) -> String {
    format!("headstash-register-sol-v1:{}:{}:{amount}", hex::encode(ctx), hex::encode(key_com))
}

pub struct Parsed {
    ctx: [u8; 32],
    key_com: [u8; 32],
    amount: u64,
    sig: ParsedSig,
}

enum ParsedSig {
    Evm([u8; 65]),
    Sol { pubkey: [u8; 32], signature: [u8; 64] },
}

/// Stateless format checks only.
pub fn parse(bytes: &[u8]) -> Result<Parsed, Reject> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| Reject::Bad("registration"))?;
    let ctx = decode_hex_n(v.get("ctx_hex").and_then(Value::as_str).unwrap_or(""), 32).ok_or(Reject::Bad("registration"))?;
    let key_com: [u8; 32] = decode_hex_n(v.get("key_com_hex").and_then(Value::as_str).unwrap_or(""), 32)
        .ok_or(Reject::Bad("key commitment"))?
        .try_into()
        .unwrap();
    if Option::<pallas::Base>::from(pallas::Base::from_repr(key_com)).is_none() {
        return Err(Reject::Bad("key commitment"));
    }
    let amount = match v.get("amount") {
        Some(Value::String(t)) if !t.starts_with('0') || t == "0" => t.parse::<u64>().map_err(|_| Reject::Bad("amount"))?,
        Some(Value::Number(n)) => n.as_u64().ok_or(Reject::Bad("amount"))?,
        _ => return Err(Reject::Bad("amount")),
    };
    if amount == 0 {
        return Err(Reject::Bad("amount"));
    }
    let sig_hex = v.get("signature_hex").and_then(Value::as_str).unwrap_or("");
    let sig = match v.get("kind").and_then(Value::as_str).unwrap_or("evm") {
        "evm" => ParsedSig::Evm(decode_hex_n(sig_hex, 65).ok_or(Reject::Bad("signature"))?.try_into().unwrap()),
        "sol" => ParsedSig::Sol {
            pubkey: decode_hex_n(v.get("pubkey_hex").and_then(Value::as_str).unwrap_or(""), 32)
                .ok_or(Reject::Bad("pubkey"))?
                .try_into()
                .unwrap(),
            signature: decode_hex_n(sig_hex, 64).ok_or(Reject::Bad("signature"))?.try_into().unwrap(),
        },
        _ => return Err(Reject::Bad("kind")),
    };
    Ok(Parsed { ctx: ctx.try_into().unwrap(), key_com, amount, sig })
}

/// Identity the signature claims (before verification) — used for rate limiting.
fn claimed_identity(p: &Parsed) -> Option<String> {
    match &p.sig {
        ParsedSig::Sol { pubkey, .. } => Some(format!("sol:{}", hex::encode(pubkey))),
        ParsedSig::Evm(_) => None,
    }
}

/// Native signature check, then eligibility against the configured tree.
pub fn verify(p: &Parsed, ctx: &[u8; 32], elig: &Elig) -> Result<Reg, Reject> {
    if &p.ctx != ctx {
        return Err(Reject::Bad("registration"));
    }
    let (kind, leaf) = match &p.sig {
        ParsedSig::Evm(sig) => {
            let msg = register_message(ctx, &p.key_com, p.amount);
            let address = recover_address(&msg, sig).ok_or(Reject::Bad("signature"))?;
            (Kind::Evm { address, signature: *sig }, evm_leaf(&address, p.amount))
        }
        ParsedSig::Sol { pubkey, signature } => {
            let vk = ed25519_dalek::VerifyingKey::from_bytes(pubkey).map_err(|_| Reject::Bad("pubkey"))?;
            let msg = register_sol_message(ctx, &p.key_com, p.amount);
            vk.verify_strict(msg.as_bytes(), &ed25519_dalek::Signature::from_bytes(signature))
                .map_err(|_| Reject::Bad("signature"))?;
            (Kind::Sol { pubkey: *pubkey, signature: *signature }, sol_leaf(pubkey, p.amount))
        }
    };
    let mut reg = Reg { kind, key_com: p.key_com, amount: p.amount, index: 0, path: vec![] };
    let (index, path) = elig.lookup(&reg.identity(), p.amount).ok_or(Reject::Bad("eligibility"))?;
    if open_path(leaf, index, &path) != elig.root() {
        return Err(Reject::Bad("eligibility"));
    }
    reg.index = index;
    reg.path = path;
    Ok(reg)
}

pub fn recover_address(message: &str, signature: &[u8; 65]) -> Option<[u8; 20]> {
    let recid = match signature[64] {
        0 | 27 => 0,
        1 | 28 => 1,
        _ => return None,
    };
    let mut env = format!("\x19Ethereum Signed Message:\n{}", message.len()).into_bytes();
    env.extend_from_slice(message.as_bytes());
    let digest = keccak256(&env);
    let sig = Signature::from_slice(&signature[..64]).ok()?;
    let vk = VerifyingKey::recover_from_prehash(&digest, &sig, RecoveryId::from_byte(recid)?).ok()?;
    let point = vk.to_encoded_point(false);
    let hash = keccak256(&point.as_bytes()[1..]);
    let mut out = [0u8; 20];
    out.copy_from_slice(&hash[12..]);
    Some(out)
}

// ---------------------------------------------------------------- batcher

pub struct Limits {
    pub flush: Duration,
    pub max_batch: usize,
    pub max_queue: usize,
    pub per_identity: (usize, Duration),
    pub per_ip: (usize, Duration),
    pub cache_ttl: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            flush: Duration::from_secs(6),
            max_batch: 25,
            max_queue: 2000,
            per_identity: (5, Duration::from_secs(600)),
            per_ip: (30, Duration::from_secs(60)),
            cache_ttl: Duration::from_secs(30),
        }
    }
}

#[derive(Default)]
struct Inner {
    queue: VecDeque<(String, Reg, u8)>,
    queued: HashSet<String>,
    status: HashMap<String, Status>,
    by_identity: HashMap<String, String>,
    hits: HashMap<String, VecDeque<Instant>>,
    cache: HashMap<String, Instant>,
}

pub struct Batcher<C: Chain> {
    chain: C,
    limits: Limits,
    inner: Mutex<Inner>,
    wake: Notify,
    seq: Mutex<Option<(u64, u64)>>,
    /// Serialises every tx this account signs (batches and grants).
    tx: Mutex<()>,
    granted: Mutex<HashMap<String, Instant>>,
}

pub fn request_id(identity: &str) -> String {
    hex::encode(&sha256(identity.as_bytes())[..16])
}

impl<C: Chain> Batcher<C> {
    pub fn new(chain: C, limits: Limits) -> Self {
        Self {
            chain, limits, inner: Mutex::new(Inner::default()), wake: Notify::new(), seq: Mutex::new(None),
            tx: Mutex::new(()), granted: Mutex::new(HashMap::new()),
        }
    }

    fn hit(inner: &mut Inner, key: String, (n, window): (usize, Duration)) -> bool {
        let now = Instant::now();
        let q = inner.hits.entry(key).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) > window) {
            q.pop_front();
        }
        if q.len() >= n {
            return false;
        }
        q.push_back(now);
        true
    }

    pub async fn rate_ip(&self, ip: &str) -> Result<(), Reject> {
        let mut inner = self.inner.lock().await;
        if Self::hit(&mut inner, format!("ip:{ip}"), self.limits.per_ip) { Ok(()) } else { Err(Reject::Rate) }
    }

    pub async fn rate_key(&self, key: &str, limit: (usize, Duration)) -> Result<(), Reject> {
        let mut inner = self.inner.lock().await;
        if Self::hit(&mut inner, key.to_string(), limit) { Ok(()) } else { Err(Reject::Rate) }
    }

    /// Reserve grant dedupe keys (nullifier / chunk, grantee) for `ttl`.
    pub async fn reserve(&self, keys: &[String], ttl: Duration) -> Result<(), Reject> {
        let mut g = self.granted.lock().await;
        g.retain(|_, t| t.elapsed() < ttl);
        if keys.iter().any(|k| g.contains_key(k)) {
            return Err(Reject::Bad("already granted"));
        }
        let now = Instant::now();
        for k in keys {
            g.insert(k.clone(), now);
        }
        Ok(())
    }

    pub async fn release(&self, keys: &[String]) {
        let mut g = self.granted.lock().await;
        for k in keys {
            g.remove(k);
        }
    }

    /// Sign + broadcast one raw message (a fee grant) and wait for inclusion.
    pub async fn send_one(&self, msg: Value) -> Result<(String, u64, u64), Reject> {
        let _tx = self.tx.lock().await;
        let msgs = [msg];
        let (mut acct, mut seq) = self.account(false).await.map_err(|_| Reject::Bad("relay"))?;
        let gas = match self.chain.simulate(&msgs, acct, seq).await {
            Ok(g) => g,
            Err(ChainErr::Sequence) => {
                (acct, seq) = self.account(true).await.map_err(|_| Reject::Bad("relay"))?;
                self.chain.simulate(&msgs, acct, seq).await.map_err(|_| Reject::Bad("grant rejected"))?
            }
            Err(_) => return Err(Reject::Bad("grant rejected")),
        };
        let gas = gas.saturating_mul(13) / 10 + 20_000;
        let txhash = match self.chain.broadcast(&msgs, gas, acct, seq).await {
            Ok(h) => h,
            Err(_) => {
                let _ = self.account(true).await;
                return Err(Reject::Bad("relay"));
            }
        };
        *self.seq.lock().await = Some((acct, seq + 1));
        match self.chain.wait(&txhash).await {
            Ok(r) if r.code == 0 => Ok((txhash, r.height, gas)),
            _ => {
                let _ = self.account(true).await;
                Err(Reject::Bad("grant rejected"))
            }
        }
    }

    pub async fn rate_identity(&self, id: &str) -> Result<(), Reject> {
        let mut inner = self.inner.lock().await;
        if Self::hit(&mut inner, format!("id:{id}"), self.limits.per_identity) { Ok(()) } else { Err(Reject::Rate) }
    }

    /// Dedupe, on-chain registered check (cached), then queue.
    pub async fn submit(&self, reg: Reg) -> Result<(String, Status), Reject> {
        let identity = reg.identity();
        let id = request_id(&identity);
        {
            let inner = self.inner.lock().await;
            if let Some(s) = inner.status.get(&id) {
                if !matches!(s, Status::Failed { .. }) {
                    return Ok((id, s.clone()));
                }
            }
            if inner.queue.len() >= self.limits.max_queue {
                return Err(Reject::Full);
            }
        }
        let fresh = {
            let inner = self.inner.lock().await;
            inner.cache.get(&identity).is_some_and(|t| t.elapsed() < self.limits.cache_ttl)
        };
        if !fresh {
            match self.chain.registered(&identity).await {
                Ok(true) => {
                    let s = Status::Registered { txhash: None, height: None };
                    self.inner.lock().await.status.insert(id.clone(), s.clone());
                    return Ok((id, s));
                }
                Ok(false) => {
                    self.inner.lock().await.cache.insert(identity.clone(), Instant::now());
                }
                Err(_) => return Err(Reject::Bad("relay")),
            }
        }
        let mut inner = self.inner.lock().await;
        if inner.queued.contains(&identity) {
            return Ok((id.clone(), inner.status.get(&id).cloned().unwrap_or(Status::Queued)));
        }
        if inner.queue.len() >= self.limits.max_queue {
            return Err(Reject::Full);
        }
        inner.queued.insert(identity.clone());
        inner.by_identity.insert(identity, id.clone());
        inner.status.insert(id.clone(), Status::Queued);
        inner.queue.push_back((id.clone(), reg, 0));
        let full = inner.queue.len() >= self.limits.max_batch;
        drop(inner);
        if full {
            self.wake.notify_one();
        }
        Ok((id, Status::Queued))
    }

    pub async fn status(&self, id: &str) -> Option<Status> {
        self.inner.lock().await.status.get(id).cloned()
    }

    /// The single in-flight broadcaster.
    pub async fn run(self: Arc<Self>) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.limits.flush) => {}
                _ = self.wake.notified() => {}
            }
            while self.flush_once().await > 0 {}
        }
    }

    async fn set(&self, id: &str, s: Status) {
        self.inner.lock().await.status.insert(id.to_string(), s);
    }

    async fn finish(&self, item: &(String, Reg, u8), s: Status) {
        let mut inner = self.inner.lock().await;
        inner.queued.remove(&item.1.identity());
        inner.cache.remove(&item.1.identity());
        inner.status.insert(item.0.clone(), s);
    }

    async fn account(&self, refresh: bool) -> Result<(u64, u64), String> {
        let mut seq = self.seq.lock().await;
        if refresh || seq.is_none() {
            *seq = Some(self.chain.sequence().await?);
        }
        Ok(seq.unwrap())
    }

    /// Flush up to `max_batch`. Returns the number of items taken.
    pub async fn flush_once(&self) -> usize {
        let mut batch: Vec<(String, Reg, u8)> = {
            let mut inner = self.inner.lock().await;
            let n = inner.queue.len().min(self.limits.max_batch);
            inner.queue.drain(..n).collect()
        };
        let taken = batch.len();
        if taken == 0 {
            return 0;
        }
        let _tx = self.tx.lock().await;
        let Ok((mut acct, mut seq)) = self.account(false).await else {
            self.requeue(batch).await;
            return 0;
        };
        // Simulate; drop the offending message and retry the rest.
        let mut gas = 0u64;
        let mut refreshed = false;
        while !batch.is_empty() {
            let msgs: Vec<Value> = batch.iter().map(|b| b.1.execute_msg()).collect();
            match self.chain.simulate(&msgs, acct, seq).await {
                Ok(g) => {
                    gas = g;
                    break;
                }
                Err(ChainErr::Msg { index: Some(i), log }) if i < batch.len() => {
                    let bad = batch.remove(i);
                    self.finish(&bad, Status::Failed { error: phrase_in(&log).unwrap_or("rejected").into() }).await;
                }
                Err(ChainErr::Sequence) if !refreshed => {
                    refreshed = true;
                    match self.account(true).await {
                        Ok(a) => (acct, seq) = a,
                        Err(_) => break,
                    }
                }
                Err(_) => {
                    // Unknown index: isolate one message at a time.
                    let mut keep = Vec::new();
                    for item in batch.drain(..) {
                        match self.chain.simulate(&[item.1.execute_msg()], acct, seq).await {
                            Ok(_) => keep.push(item),
                            Err(ChainErr::Msg { log, .. }) => {
                                self.finish(&item, Status::Failed { error: phrase_in(&log).unwrap_or("rejected").into() }).await
                            }
                            Err(_) => keep.push(item),
                        }
                    }
                    if !keep.is_empty() {
                        let msgs: Vec<Value> = keep.iter().map(|b| b.1.execute_msg()).collect();
                        match self.chain.simulate(&msgs, acct, seq).await {
                            Ok(g) => gas = g,
                            Err(_) => {
                                self.requeue(keep).await;
                                return taken;
                            }
                        }
                    }
                    batch = keep;
                    break;
                }
            }
        }
        if batch.is_empty() {
            return taken;
        }
        if gas == 0 {
            self.requeue(batch).await;
            return taken;
        }
        let gas = gas.saturating_mul(13) / 10 + 50_000;
        let msgs: Vec<Value> = batch.iter().map(|b| b.1.execute_msg()).collect();
        let mut sent = self.chain.broadcast(&msgs, gas, acct, seq).await;
        if matches!(sent, Err(ChainErr::Sequence)) {
            if let Ok(a) = self.account(true).await {
                (acct, seq) = a;
                sent = self.chain.broadcast(&msgs, gas, acct, seq).await;
            }
        }
        let txhash = match sent {
            Ok(h) => h,
            Err(_) => {
                let _ = self.account(true).await;
                self.requeue(batch).await;
                return taken;
            }
        };
        *self.seq.lock().await = Some((acct, seq + 1));
        for item in &batch {
            self.set(&item.0, Status::Submitted { txhash: txhash.clone() }).await;
        }
        match self.chain.wait(&txhash).await {
            Ok(r) if r.code == 0 => {
                for item in &batch {
                    self.finish(item, Status::Registered { txhash: Some(txhash.clone()), height: Some(r.height) }).await;
                }
            }
            Ok(_) => {
                // DeliverTx failed (state moved since simulate): requeue; requeue caps retries.
                self.requeue(batch).await;
            }
            Err(_) => {
                let _ = self.account(true).await;
                self.requeue(batch).await;
            }
        }
        taken
    }

    async fn requeue(&self, items: Vec<(String, Reg, u8)>) {
        let mut inner = self.inner.lock().await;
        for (id, reg, tries) in items.into_iter().rev() {
            if tries >= 2 {
                inner.queued.remove(&reg.identity());
                inner.status.insert(id, Status::Failed { error: "relay".into() });
                continue;
            }
            inner.status.insert(id.clone(), Status::Queued);
            inner.queue.push_front((id, reg, tries + 1));
        }
    }
}

// ---------------------------------------------------------------- terpd + REST chain

pub struct TerpdChain {
    terpd: PathBuf,
    home: String,
    from: String,
    backend: String,
    chain_id: String,
    rest: String,
    contract: String,
    gas_price: (f64, String),
    sender: String,
    http: reqwest::Client,
    tmp: PathBuf,
}

impl TerpdChain {
    pub async fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let terpd = PathBuf::from(var("TERPD", "terpd"));
        let home = std::env::var("HEADSTASH_RELAY_HOME").map_err(|_| anyhow::anyhow!("HEADSTASH_RELAY_HOME (keyring home) is required"))?;
        let from = var("HEADSTASH_RELAY_FROM", "operator");
        let backend = var("HEADSTASH_KEYRING_BACKEND", "test");
        let rest = var("HEADSTASH_REST", "http://127.0.0.1:1317");
        if !rest.starts_with("http://127.0.0.1:") && !rest.starts_with("http://localhost:") && std::env::var("HEADSTASH_ALLOW_REMOTE_REST").is_err() {
            anyhow::bail!("HEADSTASH_REST must be loopback unless HEADSTASH_ALLOW_REMOTE_REST is set");
        }
        let contract = parse_contract(&var("HEADSTASH_CONTRACT", "")).map_err(|e| anyhow::anyhow!(e))?;
        let gp = var("HEADSTASH_GAS_PRICE", "0.025uterp");
        let split = gp.find(|c: char| c.is_ascii_alphabetic()).ok_or_else(|| anyhow::anyhow!("gas price"))?;
        let gas_price = (gp[..split].parse::<f64>()?, gp[split..].to_string());
        let tmp = std::env::temp_dir().join(format!("hs-relay-{}", std::process::id()));
        std::fs::create_dir_all(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut me = Self {
            terpd, home, from, backend, chain_id: String::new(), rest, contract, gas_price,
            sender: String::new(), http: reqwest::Client::builder().timeout(Duration::from_secs(20)).build()?, tmp,
        };
        // Address only: `keys show -a` prints the bech32 address, never key material.
        let out = me.terpd(&["keys", "show", &me.from.clone(), "-a", "--home", &me.home.clone(), "--keyring-backend", &me.backend.clone()]).await?;
        me.sender = out.trim().to_string();
        parse_contract(&me.sender).map_err(|_| anyhow::anyhow!("operator address"))?;
        let info: Value = me.http.get(format!("{}/cosmos/base/tendermint/v1beta1/node_info", me.rest)).send().await?.json().await?;
        me.chain_id = info.pointer("/default_node_info/network").and_then(Value::as_str).unwrap_or("").to_string();
        if let Ok(want) = std::env::var("HEADSTASH_CHAIN_ID") {
            if want != me.chain_id {
                anyhow::bail!("chain id mismatch");
            }
        }
        Ok(me)
    }

    async fn terpd(&self, args: &[&str]) -> anyhow::Result<String> {
        let mut cmd = tokio::process::Command::new(&self.terpd);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).env_clear();
        for k in ["HOME", "PATH", "TMPDIR", "USER"] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        let out = cmd.output().await?;
        if !out.status.success() {
            anyhow::bail!("terpd {}", args.first().unwrap_or(&""));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub async fn smart(&self, q: &Value) -> Result<Value, String> {
        let url = format!("{}/cosmwasm/wasm/v1/contract/{}/smart/{}", self.rest, self.contract, b64(q.to_string().as_bytes()));
        let v: Value = self.http.get(url).send().await.map_err(|_| "query")?.json().await.map_err(|_| "query")?;
        v.get("data").cloned().ok_or_else(|| "query".to_string())
    }

    async fn signed_bytes(&self, msgs: &[Value], gas: u64, acct: u64, seq: u64) -> Result<String, ChainErr> {
        let fee = ((gas as f64) * self.gas_price.0).ceil() as u64;
        // Raw Any messages (fee grants) pass through; contract msgs get wrapped.
        let body: Vec<Value> = msgs.iter().map(|m| if m.get("@type").is_some() { m.clone() } else { json!({
            "@type": "/cosmwasm.wasm.v1.MsgExecuteContract",
            "sender": self.sender, "contract": self.contract, "msg": m, "funds": []
        })}).collect();
        let unsigned = json!({
            "body": {"messages": body, "memo": "", "timeout_height": "0", "extension_options": [], "non_critical_extension_options": []},
            "auth_info": {"signer_infos": [], "fee": {"amount": [{"denom": self.gas_price.1, "amount": fee.to_string()}], "gas_limit": gas.to_string(), "payer": "", "granter": ""}},
            "signatures": []
        });
        let n = format!("{}-{}", seq, rand_suffix());
        let upath = self.tmp.join(format!("u-{n}.json"));
        let spath = self.tmp.join(format!("s-{n}.json"));
        std::fs::write(&upath, unsigned.to_string()).map_err(|e| ChainErr::Other(e.to_string()))?;
        let (a, s) = (acct.to_string(), seq.to_string());
        let res = async {
            let signed = self.terpd(&["tx", "sign", upath.to_str().unwrap(), "--from", &self.from, "--home", &self.home,
                "--keyring-backend", &self.backend, "--chain-id", &self.chain_id, "--offline",
                "--account-number", &a, "--sequence", &s, "--sign-mode", "direct", "-o", "json"]).await?;
            std::fs::write(&spath, signed)?;
            let enc = self.terpd(&["tx", "encode", spath.to_str().unwrap()]).await?;
            Ok::<String, anyhow::Error>(enc.trim().to_string())
        }.await;
        let _ = std::fs::remove_file(&upath);
        let _ = std::fs::remove_file(&spath);
        res.map_err(|e| ChainErr::Other(e.to_string()))
    }
}

fn rand_suffix() -> String {
    let mut b = [0u8; 6];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut b);
    hex::encode(b)
}

fn classify(log: &str) -> ChainErr {
    if log.contains("account sequence mismatch") || log.contains("incorrect account sequence") {
        return ChainErr::Sequence;
    }
    let index = log.find("message index: ").and_then(|p| {
        log[p + 15..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
    });
    if index.is_some() || phrase_in(log).is_some() {
        ChainErr::Msg { index, log: log.to_string() }
    } else {
        ChainErr::Other(log.chars().take(200).collect())
    }
}

#[async_trait]
impl Chain for TerpdChain {
    async fn registered(&self, identity: &str) -> Result<bool, String> {
        self.smart(&json!({"registered": {"identity": identity}})).await?.as_bool().ok_or_else(|| "query".into())
    }

    async fn sequence(&self) -> Result<(u64, u64), String> {
        let v: Value = self.http.get(format!("{}/cosmos/auth/v1beta1/accounts/{}", self.rest, self.sender))
            .send().await.map_err(|_| "account")?.json().await.map_err(|_| "account")?;
        let acct = v.get("account").ok_or("account")?;
        let base = acct.get("base_account").unwrap_or(acct);
        let num = |k: &str| base.get(k).and_then(|x| x.as_str().and_then(|s| s.parse().ok()).or(x.as_u64())).unwrap_or(0);
        Ok((num("account_number"), num("sequence")))
    }

    async fn simulate(&self, msgs: &[Value], acct: u64, seq: u64) -> Result<u64, ChainErr> {
        let tx = self.signed_bytes(msgs, 50_000_000, acct, seq).await?;
        let v: Value = self.http.post(format!("{}/cosmos/tx/v1beta1/simulate", self.rest)).json(&json!({"tx_bytes": tx}))
            .send().await.map_err(|e| ChainErr::Other(e.to_string()))?.json().await.map_err(|e| ChainErr::Other(e.to_string()))?;
        if let Some(g) = v.pointer("/gas_info/gas_used").and_then(|g| g.as_str().and_then(|s| s.parse().ok())) {
            return Ok(g);
        }
        Err(classify(v.get("message").and_then(Value::as_str).unwrap_or("")))
    }

    async fn broadcast(&self, msgs: &[Value], gas: u64, acct: u64, seq: u64) -> Result<String, ChainErr> {
        let tx = self.signed_bytes(msgs, gas, acct, seq).await?;
        let v: Value = self.http.post(format!("{}/cosmos/tx/v1beta1/txs", self.rest))
            .json(&json!({"tx_bytes": tx, "mode": "BROADCAST_MODE_SYNC"}))
            .send().await.map_err(|e| ChainErr::Other(e.to_string()))?.json().await.map_err(|e| ChainErr::Other(e.to_string()))?;
        let r = v.get("tx_response").ok_or_else(|| ChainErr::Other("broadcast".into()))?;
        if r.get("code").and_then(Value::as_u64).unwrap_or(1) != 0 {
            return Err(classify(r.get("raw_log").and_then(Value::as_str).unwrap_or("")));
        }
        r.get("txhash").and_then(Value::as_str).map(str::to_string).ok_or_else(|| ChainErr::Other("txhash".into()))
    }

    async fn wait(&self, txhash: &str) -> Result<TxResult, String> {
        for _ in 0..60 {
            if let Ok(resp) = self.http.get(format!("{}/cosmos/tx/v1beta1/txs/{}", self.rest, txhash)).send().await {
                if resp.status().is_success() {
                    let v: Value = resp.json().await.map_err(|_| "tx")?;
                    if let Some(r) = v.get("tx_response") {
                        let code = r.get("code").and_then(Value::as_u64).unwrap_or(1) as u32;
                        let height = r.get("height").and_then(|h| h.as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
                        let log = r.get("raw_log").and_then(Value::as_str).unwrap_or("").to_string();
                        return Ok(TxResult { code, height, log });
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
        Err("timeout".into())
    }

    async fn check(&self, q: &Value) -> Result<(), String> {
        let url = format!("{}/cosmwasm/wasm/v1/contract/{}/smart/{}", self.rest, self.contract, b64(q.to_string().as_bytes()));
        let v: Value = self.http.get(url).send().await.map_err(|_| "query")?.json().await.map_err(|_| "query")?;
        if v.get("data") == Some(&Value::Bool(true)) {
            return Ok(());
        }
        Err(v.get("message").and_then(Value::as_str).unwrap_or("rejected").to_string())
    }
}

// ---------------------------------------------------------------- fee grants

pub struct GrantCfg {
    pub granter: String,
    pub denom: String,
    /// Exactly one tx fee; the client pays fee == spend limit so the allowance is pruned.
    pub claim_spend: u64,
    pub register_spend: u64,
    pub ttl: Duration,
    pub global: (usize, Duration),
}

fn grant_phrase(text: &str) -> &'static str {
    const P: &[&str] = &[
        "invalid note proof", "nullifier", "anchor", "denomination", "amount", "window", "cap", "public path",
        "not enabled", "root", "destination", "signature", "eligibility", "chunk", "already registered",
        "key commitment", "list hash", "note list",
    ];
    P.iter().copied().find(|p| text.contains(p)).unwrap_or("rejected")
}

fn is_wallet(a: &str) -> bool {
    a.len() == 43 && a.starts_with("terp1") && a.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// RFC 3339 UTC, seconds precision, `ttl` from now.
pub fn expiry_rfc3339(ttl: Duration) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default() + ttl;
    let secs = t.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

async fn feegrant<C: Chain>(State(app): State<Arc<App<C>>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let headers = req.headers().clone();
    match grant_handle(&app, client_ip(peer, &headers), req).await {
        Ok(v) => reply(StatusCode::OK, v, &app.origins, &headers),
        Err(Reject::Bad(p)) => reply(StatusCode::BAD_REQUEST, json!({"error": p}), &app.origins, &headers),
        Err(Reject::Rate) => reply(StatusCode::TOO_MANY_REQUESTS, json!({"error": "rate"}), &app.origins, &headers),
        Err(Reject::Full) => reply(StatusCode::SERVICE_UNAVAILABLE, json!({"error": "busy"}), &app.origins, &headers),
    }
}

async fn grant_handle<C: Chain>(app: &App<C>, ip: String, req: Request) -> Result<Value, Reject> {
    let g = app.grants.as_ref().ok_or(Reject::Bad("fee grants disabled"))?;
    let bytes: Bytes = axum::body::to_bytes(req.into_body(), GRANT_BODY_LIMIT).await.map_err(|_| Reject::Bad("body"))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| Reject::Bad("json"))?;
    let grantee = v.get("grantee").and_then(Value::as_str).filter(|a| is_wallet(a)).ok_or(Reject::Bad("grantee"))?.to_string();
    app.batcher.rate_ip(&ip).await?;
    app.batcher.rate_key("grant:global", g.global).await?;
    let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::to_string).ok_or(Reject::Bad("grant"));
    let (key, query, spend, kind) = if let Some(c) = v.get("claim") {
        let nullifier = s(c, "nullifier")?;
        let nf = decode_bin32(&nullifier).ok_or(Reject::Bad("nullifier"))?;
        let amount = c.get("amount").and_then(|a| a.as_u64().or_else(|| a.as_str().and_then(|x| x.parse().ok()))).ok_or(Reject::Bad("amount"))?;
        let q = json!({"check_note_claim": {
            "dest": grantee, "root": s(c, "root")?, "nullifier": nullifier, "amount": amount, "proof": s(c, "proof")?
        }});
        (format!("nf:{}", hex::encode(nf)), q, g.claim_spend, "claim")
    } else if let Some(r) = v.get("register") {
        let signer = r.get("signer").ok_or(Reject::Bad("grant"))?;
        let start = r.get("start").and_then(Value::as_u64).ok_or(Reject::Bad("chunk"))?;
        let id = hex::encode(&sha256(signer.to_string().as_bytes())[..16]);
        app.batcher.rate_identity(&id).await?;
        (format!("reg:{id}:{start}"), json!({"check_register_notes": r}), g.register_spend, "register")
    } else {
        return Err(Reject::Bad("grant kind"));
    };
    let keys = vec![key, format!("grantee:{grantee}")];
    app.batcher.reserve(&keys, g.ttl).await?;
    if let Err(e) = app.batcher.chain.check(&query).await {
        app.batcher.release(&keys).await;
        return Err(Reject::Bad(grant_phrase(&e)));
    }
    let expiration = expiry_rfc3339(g.ttl);
    let msg = hash_market::onboard_authz::headstash_feegrant(&g.granter, &grantee, &g.denom, spend, &expiration);
    match app.batcher.send_one(msg).await {
        Ok((txhash, height, _)) => Ok(json!({
            "granted": true, "kind": kind, "granter": g.granter, "grantee": grantee,
            "spend_limit": {"denom": g.denom, "amount": spend.to_string()}, "expiration": expiration,
            "txhash": txhash, "height": height
        })),
        Err(e) => {
            app.batcher.release(&keys).await;
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- HTTP

pub struct App<C: Chain> {
    ctx: [u8; 32],
    elig: Elig,
    batcher: Arc<Batcher<C>>,
    origins: Vec<String>,
    grants: Option<GrantCfg>,
}

pub fn router<C: Chain>(app: Arc<App<C>>) -> Router {
    Router::new()
        .route("/health", get(health::<C>))
        .route("/register", post(register::<C>).options(preflight::<C>))
        .route("/headstash/register", post(register::<C>).options(preflight::<C>))
        .route("/register/status/:id", get(status::<C>).options(preflight::<C>))
        .route("/headstash/feegrant", post(feegrant::<C>).options(preflight::<C>))
        .with_state(app)
}

async fn health<C: Chain>(State(app): State<Arc<App<C>>>, headers: HeaderMap) -> Response {
    let root = hex::encode(app.elig.root());
    reply(StatusCode::OK, json!({"ok": true, "ctx_hex": hex::encode(app.ctx), "eligibility_root_hex": root}), &app.origins, &headers)
}

async fn preflight<C: Chain>(State(app): State<Arc<App<C>>>, headers: HeaderMap) -> Response {
    reply(StatusCode::NO_CONTENT, Value::Null, &app.origins, &headers)
}

async fn status<C: Chain>(State(app): State<Arc<App<C>>>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return reply(StatusCode::BAD_REQUEST, json!({"error": "id"}), &app.origins, &headers);
    }
    match app.batcher.status(&id).await {
        Some(s) => reply(StatusCode::OK, s.to_json(&id), &app.origins, &headers),
        None => reply(StatusCode::NOT_FOUND, json!({"error": "unknown"}), &app.origins, &headers),
    }
}

fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> String {
    // Trust X-Real-IP only from the loopback nginx.
    if peer.ip().is_loopback() {
        if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            return v.chars().take(64).collect();
        }
    }
    peer.ip().to_string()
}

async fn register<C: Chain>(State(app): State<Arc<App<C>>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let headers = req.headers().clone();
    let r = handle(&app, client_ip(peer, &headers), req).await;
    match r {
        Ok((id, s)) => {
            let code = if matches!(s, Status::Registered { .. }) { StatusCode::OK } else { StatusCode::ACCEPTED };
            reply(code, s.to_json(&id), &app.origins, &headers)
        }
        Err(Reject::Bad(p)) => reply(StatusCode::BAD_REQUEST, json!({"error": p}), &app.origins, &headers),
        Err(Reject::Rate) => reply(StatusCode::TOO_MANY_REQUESTS, json!({"error": "rate"}), &app.origins, &headers),
        Err(Reject::Full) => reply(StatusCode::SERVICE_UNAVAILABLE, json!({"error": "queue full"}), &app.origins, &headers),
    }
}

async fn handle<C: Chain>(app: &App<C>, ip: String, req: Request) -> Result<(String, Status), Reject> {
    let bytes: Bytes = axum::body::to_bytes(req.into_body(), BODY_LIMIT).await.map_err(|_| Reject::Bad("body"))?;
    let parsed = parse(&bytes)?;
    if parsed.ctx != app.ctx {
        return Err(Reject::Bad("registration"));
    }
    app.batcher.rate_ip(&ip).await?;
    if let Some(id) = claimed_identity(&parsed) {
        app.batcher.rate_identity(&id).await?;
    }
    let reg = verify(&parsed, &app.ctx, &app.elig)?;
    if matches!(reg.kind, Kind::Evm { .. }) {
        app.batcher.rate_identity(&reg.identity()).await?;
    }
    app.batcher.submit(reg).await
}

fn reply(status: StatusCode, body: Value, origins: &[String], headers: &HeaderMap) -> Response {
    let mut resp = if body.is_null() { status.into_response() } else { (status, Json(body)).into_response() };
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if origins.iter().any(|o| o == origin) {
            let h = resp.headers_mut();
            if let Ok(v) = HeaderValue::from_str(origin) {
                h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
            }
            h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
            h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
            h.insert(header::VARY, HeaderValue::from_static("Origin"));
        }
    }
    resp
}

// ---------------------------------------------------------------- main

fn check_bind(bind: &str) -> Result<SocketAddr, &'static str> {
    let addr: SocketAddr = bind.parse().map_err(|_| "bind")?;
    if addr.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) {
        return Err("refusing non-loopback bind");
    }
    Ok(addr)
}

fn env_num<T: std::str::FromStr>(k: &str, d: T) -> T {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bind = std::env::var("HEADSTASH_RELAY_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let addr = check_bind(&bind).map_err(|e| anyhow::anyhow!(e))?;
    let chain = TerpdChain::from_env().await?;
    let campaign = chain.smart(&json!({"epoch_campaign": {}})).await.map_err(|e| anyhow::anyhow!(e))?;
    // vk_id 3 note campaigns register on-chain (RegisterNotes, depth 17 note tree) and are
    // gated by the CheckRegisterNotes / CheckNoteClaim dry runs, so the legacy amount file is
    // optional there: without it the legacy /register route rejects every body ("eligibility").
    let note_campaign = campaign.get("vk_id").and_then(Value::as_u64) == Some(3);
    let elig_path = std::env::var("HEADSTASH_ELIGIBILITY").ok();
    if elig_path.is_none() && !note_campaign {
        anyhow::bail!("HEADSTASH_ELIGIBILITY is required");
    }
    let ctx = decode_bin32(campaign.get("ctx").and_then(Value::as_str).unwrap_or("")).ok_or_else(|| anyhow::anyhow!("chain ctx"))?;
    let reg = campaign.get("registration").ok_or_else(|| anyhow::anyhow!("chain registration"))?;
    let root = decode_bin32(reg.get("eligibility_root").and_then(Value::as_str).unwrap_or("")).ok_or_else(|| anyhow::anyhow!("chain root"))?;
    let depth = reg.get("eligibility_depth").and_then(Value::as_u64).unwrap_or(0) as u32;
    let elig = match &elig_path {
        Some(path) => Elig::load(&std::fs::read(path)?)?,
        None => Elig::from_leaves(1, vec![[0u8; 32]; 2], HashMap::new()),
    };
    if elig_path.is_some() && (root != elig.root() || depth != elig.depth) {
        anyhow::bail!("eligibility mismatch chain {} file {}", hex::encode(&root[..4]), hex::encode(&elig.root()[..4]));
    }
    let limits = Limits {
        flush: Duration::from_secs(env_num("HEADSTASH_FLUSH_SECS", 6)),
        max_batch: env_num("HEADSTASH_MAX_BATCH", 25),
        max_queue: env_num("HEADSTASH_MAX_QUEUE", 2000),
        ..Limits::default()
    };
    let origins = std::env::var("HEADSTASH_PAGE_ORIGINS").unwrap_or_else(|_| DEFAULT_ORIGIN.into())
        .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let sender = chain.sender.clone();
    let chain_id = chain.chain_id.clone();
    // Fee grants: on unless HEADSTASH_FEEGRANT=0. Spend = gas limit x gas price, one tx.
    let grants = (std::env::var("HEADSTASH_FEEGRANT").as_deref() != Ok("0")).then(|| {
        let fee = |gas: u64| ((gas as f64) * chain.gas_price.0).ceil() as u64;
        // ClaimNote is flat ~262k with the anchor map; one grant funds one 32-note RegisterNotes
        // chunk (~10.7M), so a granted tx can never carry more than one (block cap 4x).
        GrantCfg {
            granter: sender.clone(),
            denom: chain.gas_price.1.clone(),
            claim_spend: fee(env_num("HEADSTASH_CLAIM_GAS", 300_000)),
            register_spend: fee(env_num("HEADSTASH_REGISTER_GAS", 12_000_000)),
            ttl: Duration::from_secs(env_num("HEADSTASH_GRANT_TTL_SECS", 600)),
            global: (env_num("HEADSTASH_GRANTS_PER_MIN", 60), Duration::from_secs(60)),
        }
    });
    let batcher = Arc::new(Batcher::new(chain, limits));
    tokio::spawn(batcher.clone().run());
    let app = Arc::new(App { ctx, elig, batcher, origins, grants });
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    if !local.ip().is_loopback() {
        anyhow::bail!("listener is not loopback");
    }
    println!("relay_up {local} chain {chain_id} root_prefix {} sender {sender}", hex::encode(&root[..4]));
    axum::serve(listener, router(app).into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

// ---------------------------------------------------------------- helpers

fn parse_contract(raw: &str) -> Result<String, &'static str> {
    if raw.len() < 39 || raw.len() > 90 || !raw.starts_with("terp1") || !raw.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("contract");
    }
    Ok(raw.to_string())
}

fn phrase_in(text: &str) -> Option<&'static str> {
    const P: &[&str] = &["already registered", "tree full", "key commitment", "eligibility", "signature", "pubkey", "amount", "registration"];
    P.iter().copied().find(|p| text.contains(p))
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak::v256();
    h.update(bytes);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn decode_hex_n(text: &str, len: usize) -> Option<Vec<u8>> {
    if text.len() != len * 2 || !text.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        return None;
    }
    hex::decode(text).ok()
}

fn decode_bin32(text: &str) -> Option<[u8; 32]> {
    base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()?.try_into().ok()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
#[path = "headstash-register-relay/tests.rs"]
mod tests;
