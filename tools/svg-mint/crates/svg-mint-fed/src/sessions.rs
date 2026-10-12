//! Checkout sessions for the ops table. Not the payment memo.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const CAP: usize = 500;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: u64,
    pub at: u64,
    pub terp: String,
    pub phase: String,
    pub qty: u32,
    pub zat: u64,
    pub txid: String,
    pub token_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct File {
    next_id: u64,
    rows: Vec<SessionRow>,
}

pub struct Log {
    path: PathBuf,
    next_id: u64,
    rows: Vec<SessionRow>,
}

impl Log {
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let (next_id, rows) = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .map(|f| (f.next_id.max(1), f.rows))
            .unwrap_or((1, Vec::new()));
        Self { path, next_id, rows }
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let file = File {
            next_id: self.next_id,
            rows: self.rows.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&file) {
            let _ = fs::write(&self.path, bytes);
        }
    }

    pub fn issue(&mut self, terp: &str, qty: u32, zat: u64) {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.rows.push(SessionRow {
            id,
            at: now(),
            terp: terp.to_string(),
            phase: "issued".into(),
            qty,
            zat,
            txid: String::new(),
            token_id: String::new(),
        });
        self.trim();
        self.save();
    }

    /// Move the newest open row for this address forward. No-op if none.
    pub fn advance(&mut self, terp: &str, phase: &str, txid: &str, zat: u64, token_id: &str) {
        let Some(row) = self.rows.iter_mut().rev().find(|r| {
            r.terp == terp && r.phase != "settled" && r.phase != "expired" && r.phase != "refunded"
        }) else {
            return;
        };
        row.phase = phase.to_string();
        row.at = now();
        if !txid.is_empty() {
            row.txid = txid.to_string();
        }
        if zat > 0 {
            row.zat = zat;
        }
        if !token_id.is_empty() {
            row.token_id = token_id.to_string();
        }
        self.save();
    }

    pub fn open_row(&self, terp: &str) -> Option<&SessionRow> {
        self.rows.iter().rev().find(|r| {
            r.terp == terp && r.phase != "settled" && r.phase != "expired" && r.phase != "refunded"
        })
    }

    /// Refund a checkout that has not minted. A token id means a mint was recorded.
    pub fn refund(&mut self, terp: &str) -> bool {
        let Some(row) = self.rows.iter_mut().rev().find(|r| {
            r.terp == terp && matches!(r.phase.as_str(), "issued" | "mempool" | "deposited" | "entitled")
        }) else {
            return false;
        };
        if !row.token_id.is_empty() {
            return false;
        }
        row.phase = "refunded".into();
        row.token_id.clear();
        row.at = now();
        self.save();
        true
    }

    /// Settled only when `terp_tx` is a 64-hex chain hash. A failed broadcast does not qualify.
    pub fn record_chain_mint(&mut self, terp: &str, terp_tx: &str) -> bool {
        if terp_tx.len() != 64 || !terp_tx.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        let Some(row) = self.rows.iter_mut().rev().find(|r| {
            r.terp == terp && r.phase == "entitled" && r.token_id.is_empty()
        }) else {
            return false;
        };
        row.phase = "settled".into();
        row.txid = terp_tx.to_string();
        row.token_id = terp_tx.to_string();
        row.at = now();
        self.save();
        true
    }

    /// One read of the full record, then the record is empty.
    pub fn export_once(&mut self) -> Vec<SessionRow> {
        let rows = self.rows.clone();
        self.rows.clear();
        self.save();
        rows
    }

    pub fn page(&self, limit: usize, before: Option<u64>) -> (Vec<SessionRow>, Option<u64>) {
        let limit = limit.clamp(1, 50);
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .filter(|r| before.map(|b| r.id < b).unwrap_or(true))
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.id.cmp(&a.id));
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more { rows.last().map(|r| r.id) } else { None };
        (rows, next)
    }

    fn trim(&mut self) {
        if self.rows.len() > CAP {
            let drop_n = self.rows.len() - CAP;
            self.rows.drain(0..drop_n);
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
