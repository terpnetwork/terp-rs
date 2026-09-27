//! HashMerchant-embeddable FROST inbox HTTP.
//! Hold shares until t-of-n, then broadcast adapter (string txid until ICT_SPAWN).
//! Bind 127.0.0.1 only. Jobs seal to disk when `ITL_FROST_PERSIST_KEY` is >=16 chars.

use crate::http::HttpResponse;
use crate::itl_frost::{
    persist_required, BroadcastTarget, FrostScheme, PendingJob, FROST_N, FROST_T,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8] = b"ITLFP1\n";
const BIND_DEFAULT: &str = "127.0.0.1:19091";

#[derive(Default)]
pub struct FrostInbox {
    jobs: HashMap<String, PendingJob>,
    seq: u64,
    persist_key: Option<String>,
    persist_path: Option<PathBuf>,
}

pub type SharedInbox = Arc<Mutex<FrostInbox>>;

#[derive(Deserialize)]
struct CreateJob {
    pay_code: String,
    #[serde(default)]
    scheme: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    roster_commit: Option<String>,
}

#[derive(Deserialize)]
struct ShareBody {
    admin_id: String,
    blob: String,
    #[serde(default)]
    roster_commit: Option<String>,
}

#[derive(serde::Serialize, Deserialize)]
struct Snapshot {
    seq: u64,
    jobs: Vec<PendingJob>,
}

fn parse_scheme(s: &str) -> Result<FrostScheme, &'static str> {
    match s {
        "RedPallas" | "redpallas" | "" => Ok(FrostScheme::RedPallas),
        "Ed25519Consensus" => Ok(FrostScheme::Ed25519Consensus),
        "Bls" => Ok(FrostScheme::Bls),
        _ => Err("unknown_scheme"),
    }
}

fn parse_target(s: &str) -> Result<BroadcastTarget, &'static str> {
    match s {
        "Zcash" | "zcash" | "" => Ok(BroadcastTarget::Zcash),
        "Penumbra" => Ok(BroadcastTarget::Penumbra),
        "SmartAccountExecute" => Ok(BroadcastTarget::SmartAccountExecute),
        _ => Err("unknown_target"),
    }
}

fn err_status(status: u16, e: &str) -> HttpResponse {
    HttpResponse {
        status,
        body: json!({"ok": false, "error": e}).to_string(),
    }
}

fn persist_snapshot_ok(json: &str) -> Result<(), &'static str> {
    let lower = json.to_ascii_lowercase();
    if lower.contains("keypackage")
        || lower.contains("secret_share")
        || lower.contains("signingshare")
        || lower.contains("\"packages\"")
    {
        return Err("keypackage_rejected");
    }
    Ok(())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let d = Sha256::digest(key);
        k[..32].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = k;
    let mut opad = k;
    for b in &mut ipad {
        *b ^= 0x36;
    }
    for b in &mut opad {
        *b ^= 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let ih = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(ih);
    outer.finalize().into()
}

fn derived_key(raw: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"itl-frost-persist-v1");
    h.update(raw.as_bytes());
    h.finalize().into()
}

fn keystream(key: &[u8], nonce: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut counter = 0u32;
    while out.len() < len {
        let mut h = Sha256::new();
        h.update(b"itl-frost-ks-v1");
        h.update(key);
        h.update(nonce);
        h.update(counter.to_le_bytes());
        out.extend_from_slice(&h.finalize());
        counter = counter.wrapping_add(1);
    }
    out.truncate(len);
    out
}

fn xor_in_place(buf: &mut [u8], ks: &[u8]) {
    for (b, k) in buf.iter_mut().zip(ks.iter()) {
        *b ^= *k;
    }
}

fn fresh_nonce(seq: u64) -> [u8; 16] {
    let mut h = Sha256::new();
    h.update(b"itl-frost-nonce");
    h.update(seq.to_le_bytes());
    if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
        h.update(d.as_nanos().to_le_bytes());
    }
    let d = h.finalize();
    let mut n = [0u8; 16];
    n.copy_from_slice(&d[..16]);
    n
}

/// HASHMERCHANT_BIND / ITL_FROST_BIND, else 127.0.0.1:19091. Non-loopback is rejected.
pub fn loopback_bind() -> Result<String, &'static str> {
    let raw = std::env::var("HASHMERCHANT_BIND")
        .or_else(|_| std::env::var("ITL_FROST_BIND"))
        .unwrap_or_else(|_| BIND_DEFAULT.into());
    let bind = if raw.starts_with("127.0.0.1:") {
        raw
    } else if raw == "127.0.0.1" {
        BIND_DEFAULT.into()
    } else if raw.chars().all(|c| c.is_ascii_digit()) {
        format!("127.0.0.1:{raw}")
    } else {
        return Err("frost_bind_not_loopback");
    };
    if !bind.starts_with("127.0.0.1:") {
        return Err("frost_bind_not_loopback");
    }
    Ok(bind)
}

impl FrostInbox {
    pub fn persist_on(&self) -> bool {
        self.persist_key
            .as_ref()
            .map(|k| k.len() >= 16)
            .unwrap_or(false)
    }

    /// RAM-only when the persist key is missing/short (`persistence_off`).
    pub fn from_env() -> Result<Self, &'static str> {
        match persist_required() {
            Err(_) => Ok(Self::default()),
            Ok(()) => {
                let key = std::env::var("ITL_FROST_PERSIST_KEY").map_err(|_| "persistence_off")?;
                let dir = std::env::var("ITL_FROST_PERSIST_DIR")
                    .unwrap_or_else(|_| "data/itl-frost".into());
                Self::with_persist(&key, Path::new(&dir))
            }
        }
    }

    pub fn with_persist(key: &str, dir: &Path) -> Result<Self, &'static str> {
        if key.len() < 16 {
            return Err("persistence_off");
        }
        let persist_path = dir.join("inbox.seal");
        let mut inbox = Self {
            jobs: HashMap::new(),
            seq: 0,
            persist_key: Some(key.to_string()),
            persist_path: Some(persist_path.clone()),
        };
        if persist_path.exists() {
            inbox.restore()?;
        }
        Ok(inbox)
    }

    fn seal(&self) -> Result<(), &'static str> {
        let key_raw = self.persist_key.as_deref().ok_or("persistence_off")?;
        if key_raw.len() < 16 {
            return Err("persistence_off");
        }
        let path = self.persist_path.as_ref().ok_or("persistence_off")?;
        let mut jobs: Vec<PendingJob> = self.jobs.values().cloned().collect();
        jobs.sort_by(|a, b| a.id.cmp(&b.id));
        let snap = Snapshot {
            seq: self.seq,
            jobs,
        };
        let plain = serde_json::to_vec(&snap).map_err(|_| "persist_encode")?;
        if persist_snapshot_ok(std::str::from_utf8(&plain).unwrap_or("")).is_err() {
            return Err("keypackage_rejected");
        }
        let key = derived_key(key_raw);
        let nonce = fresh_nonce(self.seq);
        let mut ct = plain;
        let ks = keystream(&key, &nonce, ct.len());
        xor_in_place(&mut ct, &ks);
        let mut mac_input = Vec::with_capacity(MAGIC.len() + nonce.len() + ct.len());
        mac_input.extend_from_slice(MAGIC);
        mac_input.extend_from_slice(&nonce);
        mac_input.extend_from_slice(&ct);
        let mac = hmac_sha256(&key, &mac_input);
        let mut out = mac_input;
        out.extend_from_slice(&mac);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| "persist_write")?;
        }
        let tmp = path.with_extension("seal.tmp");
        std::fs::write(&tmp, &out).map_err(|_| "persist_write")?;
        std::fs::rename(&tmp, path).map_err(|_| "persist_write")?;
        Ok(())
    }

    fn restore(&mut self) -> Result<(), &'static str> {
        let key_raw = self.persist_key.as_deref().ok_or("persistence_off")?;
        let path = self.persist_path.as_ref().ok_or("persistence_off")?;
        let bytes = std::fs::read(path).map_err(|_| "persist_read")?;
        if bytes.len() < MAGIC.len() + 16 + 32 {
            return Err("persist_corrupt");
        }
        if !bytes.starts_with(MAGIC) {
            return Err("persist_corrupt");
        }
        let nonce = &bytes[MAGIC.len()..MAGIC.len() + 16];
        let mac = &bytes[bytes.len() - 32..];
        let ct = &bytes[MAGIC.len() + 16..bytes.len() - 32];
        let key = derived_key(key_raw);
        let mut mac_input = Vec::with_capacity(MAGIC.len() + 16 + ct.len());
        mac_input.extend_from_slice(MAGIC);
        mac_input.extend_from_slice(nonce);
        mac_input.extend_from_slice(ct);
        let expect = hmac_sha256(&key, &mac_input);
        if expect.as_slice() != mac {
            return Err("persist_mac");
        }
        let mut plain = ct.to_vec();
        let ks = keystream(&key, nonce, plain.len());
        xor_in_place(&mut plain, &ks);
        let text = std::str::from_utf8(&plain).map_err(|_| "persist_corrupt")?;
        persist_snapshot_ok(text)?;
        let snap: Snapshot = serde_json::from_str(text).map_err(|_| "persist_corrupt")?;
        self.seq = snap.seq;
        self.jobs = snap.jobs.into_iter().map(|j| (j.id.clone(), j)).collect();
        Ok(())
    }

    fn persist_after(&self) -> Result<(), &'static str> {
        if !self.persist_on() {
            return Ok(());
        }
        self.seal()
    }

    pub fn try_handle(&mut self, method: &str, path: &str, body: &str) -> Option<HttpResponse> {
        let path = path.split('?').next().unwrap_or(path);
        if method == "POST" && path == "/itl/frost/jobs" {
            return Some(self.create_job(body));
        }
        if method == "GET" && path.starts_with("/itl/frost/jobs/") {
            let id = path.trim_start_matches("/itl/frost/jobs/");
            if id.is_empty() || id.contains('/') {
                return None;
            }
            return Some(self.get_job(id));
        }
        if method == "POST" && path.contains("/share") && path.starts_with("/itl/frost/jobs/") {
            let id = path
                .trim_start_matches("/itl/frost/jobs/")
                .trim_end_matches("/share");
            return Some(self.share(id, body));
        }
        if method == "POST" && path.contains("/broadcast") && path.starts_with("/itl/frost/jobs/") {
            let id = path
                .trim_start_matches("/itl/frost/jobs/")
                .trim_end_matches("/broadcast");
            return Some(self.broadcast(id, body));
        }
        None
    }

    fn create_job(&mut self, body: &str) -> HttpResponse {
        let parsed: CreateJob = match serde_json::from_str(body) {
            Ok(p) => p,
            Err(e) => {
                return HttpResponse {
                    status: 400,
                    body: json!({"ok": false, "error": e.to_string()}).to_string(),
                }
            }
        };
        let scheme = match parse_scheme(parsed.scheme.as_deref().unwrap_or("RedPallas")) {
            Ok(s) => s,
            Err(e) => return err_status(400, e),
        };
        let target = match parse_target(parsed.target.as_deref().unwrap_or("Zcash")) {
            Ok(t) => t,
            Err(e) => return err_status(400, e),
        };
        self.seq += 1;
        let id = format!("job-{}", self.seq);
        let mut job = PendingJob::open_for(&id, &parsed.pay_code, FROST_T, FROST_N, scheme, target);
        if let Some(rc) = parsed.roster_commit.as_deref() {
            let c = rc.trim().to_ascii_lowercase();
            if c.len() != 64 || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
                self.seq -= 1;
                return err_status(400, "roster_commit_required");
            }
            job.roster_commit = c;
        } else if scheme == FrostScheme::RedPallas && target == BroadcastTarget::Zcash {
            self.seq -= 1;
            return err_status(400, "roster_commit_required");
        }
        self.jobs.insert(id.clone(), job);
        if let Err(e) = self.persist_after() {
            self.jobs.remove(&id);
            self.seq -= 1;
            return err_status(500, e);
        }
        HttpResponse {
            status: 200,
            body: json!({"ok": true, "id": id, "status": "pending"}).to_string(),
        }
    }

    fn get_job(&self, id: &str) -> HttpResponse {
        match self.jobs.get(id) {
            None => err_status(404, "unknown_job"),
            Some(j) => HttpResponse {
                status: 200,
                body: json!({
                    "ok": true,
                    "id": j.id,
                    "pay_code": j.pay_code,
                    "status": format!("{:?}", j.status).to_lowercase(),
                    "shares": j.share_count(),
                    "txid": j.txid,
                    "roster_commit": j.roster_commit,
                })
                .to_string(),
            },
        }
    }

    fn share(&mut self, id: &str, body: &str) -> HttpResponse {
        let parsed: ShareBody = match serde_json::from_str(body) {
            Ok(p) => p,
            Err(e) => {
                return HttpResponse {
                    status: 400,
                    body: json!({"ok": false, "error": e.to_string()}).to_string(),
                }
            }
        };
        let st = {
            let Some(job) = self.jobs.get_mut(id) else {
                return err_status(404, "unknown_job");
            };
            let r = match parsed.roster_commit.as_deref() {
                Some(rc) => job.submit_share_bound(&parsed.admin_id, &parsed.blob, rc),
                None => job.submit_share(&parsed.admin_id, &parsed.blob),
            };
            match r {
                Ok(st) => st,
                Err(e) => return err_status(400, e),
            }
        };
        if let Err(e) = self.persist_after() {
            return err_status(500, e);
        }
        HttpResponse {
            status: 200,
            body: json!({"ok": true, "status": format!("{:?}", st).to_lowercase()}).to_string(),
        }
    }

    fn broadcast(&mut self, id: &str, body: &str) -> HttpResponse {
        #[derive(Deserialize)]
        struct B {
            txid: String,
        }
        let parsed: B = match serde_json::from_str(body) {
            Ok(p) => p,
            Err(e) => {
                return HttpResponse {
                    status: 400,
                    body: json!({"ok": false, "error": e.to_string()}).to_string(),
                }
            }
        };
        let txid = parsed.txid;
        {
            let Some(job) = self.jobs.get_mut(id) else {
                return err_status(404, "unknown_job");
            };
            if let Err(e) = job.broadcast(&txid) {
                return err_status(400, e);
            }
        }
        if let Err(e) = self.persist_after() {
            return err_status(500, e);
        }
        HttpResponse {
            status: 200,
            body: json!({"ok": true, "txid": txid, "status": "broadcast"}).to_string(),
        }
    }
}

pub fn shared_inbox() -> SharedInbox {
    Arc::new(Mutex::new(FrostInbox::default()))
}

pub fn shared_inbox_from_env() -> Result<SharedInbox, &'static str> {
    Ok(Arc::new(Mutex::new(FrostInbox::from_env()?)))
}

fn handle_conn(mut s: TcpStream, inbox: &SharedInbox) {
    let mut buf = [0u8; 16384];
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
    let body = req
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("")
        .trim_end_matches('\0');
    let resp = match inbox.lock() {
        Ok(mut g) => g
            .try_handle(method, path, body)
            .unwrap_or(err_status(404, "not found")),
        Err(_) => err_status(500, "inbox_lock"),
    };
    let _ = resp.write_to(&mut s);
}

/// Blocking loop. Rejects any bind that is not 127.0.0.1.
pub fn serve(bind: &str, inbox: SharedInbox) -> std::io::Result<()> {
    if !bind.starts_with("127.0.0.1:") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "frost HTTP bind must be 127.0.0.1",
        ));
    }
    let listener = TcpListener::bind(bind)?;
    listener.set_nonblocking(false)?;
    eprintln!("frost-inbox → http://{bind}  POST /itl/frost/jobs (loopback)");
    for stream in listener.incoming() {
        let s = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let inbox = inbox.clone();
        std::thread::spawn(move || handle_conn(s, &inbox));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_bind_is_127() {
        std::env::remove_var("HASHMERCHANT_BIND");
        std::env::remove_var("ITL_FROST_BIND");
        let b = loopback_bind().unwrap();
        assert!(b.starts_with("127.0.0.1:"), "{b}");
        std::env::set_var("HASHMERCHANT_BIND", "0.0.0.0:19091");
        assert_eq!(loopback_bind(), Err("frost_bind_not_loopback"));
        std::env::remove_var("HASHMERCHANT_BIND");
        std::env::set_var("ITL_FROST_BIND", "127.0.0.1:19111");
        assert_eq!(loopback_bind().unwrap(), "127.0.0.1:19111");
        std::env::remove_var("ITL_FROST_BIND");
    }
}
