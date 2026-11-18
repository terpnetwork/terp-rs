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
    /// Unmatched payments: the memo the house decrypted ("" = none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub memo: String,
    /// Unmatched payments: why no checkout took it (no memo, unknown memo, underpaid…).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct File {
    next_id: u64,
    rows: Vec<SessionRow>,
    /// ZEC txids already recorded as unmatched. Survives trim and flush so one
    /// payment is never listed (or attributed) twice.
    #[serde(default)]
    unmatched_seen: Vec<String>,
}

pub struct Log {
    path: PathBuf,
    next_id: u64,
    rows: Vec<SessionRow>,
    unmatched_seen: Vec<String>,
}

impl Log {
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let (next_id, rows, unmatched_seen) = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .map(|f| (f.next_id.max(1), f.rows, f.unmatched_seen))
            .unwrap_or((1, Vec::new(), Vec::new()));
        Self {
            path,
            next_id,
            rows,
            unmatched_seen,
        }
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let file = File {
            next_id: self.next_id,
            rows: self.rows.clone(),
            unmatched_seen: self.unmatched_seen.clone(),
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
            memo: String::new(),
            reason: String::new(),
        });
        self.trim();
        self.save();
    }

    /// A ZEC payment no checkout took. Listed once per txid (either byte order) for an
    /// admin to attribute or refund. Never credits anything. Returns true when newly listed.
    pub fn record_unmatched(
        &mut self,
        txids: &[String],
        zat: u64,
        qty: u32,
        memo: &str,
        reason: &str,
    ) -> bool {
        let Some(first) = txids.first() else {
            return false;
        };
        if txids
            .iter()
            .any(|t| self.unmatched_seen.iter().any(|s| s == t))
        {
            return false;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.rows.push(SessionRow {
            id,
            at: now(),
            terp: String::new(),
            phase: "unmatched".into(),
            qty: qty.max(1),
            zat,
            txid: first.clone(),
            token_id: String::new(),
            memo: memo.chars().take(140).collect(),
            reason: reason.into(),
        });
        self.unmatched_seen.extend(txids.iter().cloned());
        self.trim();
        self.save();
        true
    }

    pub fn row(&self, id: u64) -> Option<SessionRow> {
        self.rows.iter().find(|r| r.id == id).cloned()
    }

    /// Admin names the owner of an unmatched payment. Only `unmatched` / `attributed` rows.
    pub fn attribute(&mut self, id: u64, terp: &str, qty: u32) -> Result<SessionRow, String> {
        let r = self
            .rows
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("no such row")?;
        if !matches!(r.phase.as_str(), "unmatched" | "attributed") {
            return Err(format!("row is {}", r.phase));
        }
        r.terp = terp.to_string();
        r.phase = "attributed".into();
        if qty > 0 {
            r.qty = qty;
        }
        r.at = now();
        let out = r.clone();
        self.save();
        Ok(out)
    }

    /// Move the newest open row for this address forward. No-op if none.
    pub fn advance(&mut self, terp: &str, phase: &str, txid: &str, zat: u64, token_id: &str) {
        let Some(row) = self.rows.iter_mut().rev().find(|r| {
            r.terp == terp
                && r.phase != "settled"
                && r.phase != "expired"
                && r.phase != "refunded"
                && r.phase != "unmatched"
                && r.phase != "attributed"
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

    /// Row by id, else the newest row for this address.
    pub fn find(&self, terp: &str, id: Option<u64>) -> Option<SessionRow> {
        self.rows
            .iter()
            .rev()
            .find(|r| r.terp == terp && id.map_or(true, |i| r.id == i))
            .cloned()
    }

    pub fn set_row(&mut self, id: u64, phase: &str, txid: &str, token_id: &str) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.id == id) {
            r.phase = phase.into();
            r.txid = txid.into();
            r.token_id = token_id.into();
            r.at = now();
            self.save();
        }
    }

    pub fn refund(&mut self, terp: &str) -> bool {
        let Some(row) = self.rows.iter_mut().rev().find(|r| {
            r.terp == terp
                && matches!(
                    r.phase.as_str(),
                    "issued" | "mempool" | "deposited" | "entitled"
                )
        }) else {
            return false;
        };
        row.phase = "refunded".into();
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
        rows.sort_by_key(|r| std::cmp::Reverse(r.id));
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more {
            rows.last().map(|r| r.id)
        } else {
            None
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_log(tag: &str) -> Log {
        let p = std::env::temp_dir().join(format!(
            "svg-mint-sessions-{tag}-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&p);
        Log::open(p)
    }

    #[test]
    fn unmatched_payment_is_listed_once() {
        let mut log = tmp_log("once");
        let t = vec!["aa".repeat(32), "bb".repeat(32)];
        assert!(log.record_unmatched(&t, 200_000, 2, "", "no memo"));
        assert!(!log.record_unmatched(&t, 200_000, 2, "", "no memo"));
        assert!(!log.record_unmatched(&[t[1].clone()], 200_000, 2, "", "no memo"));
        let _ = log.export_once();
        assert!(
            !log.record_unmatched(&t, 200_000, 2, "", "no memo"),
            "flush does not re-list it"
        );
        let reopened = Log::open(&log.path);
        assert_eq!(reopened.unmatched_seen.len(), 2);
    }

    #[test]
    fn attribute_only_touches_unmatched_rows() {
        let mut log = tmp_log("attr");
        log.issue("terp1x", 1, 100_000);
        log.record_unmatched(&["cc".repeat(32)], 200_000, 2, "", "no memo");
        let un = log.rows.iter().find(|r| r.phase == "unmatched").unwrap().id;
        let issued = log.rows.iter().find(|r| r.phase == "issued").unwrap().id;
        assert!(log.attribute(issued, "terp1y", 1).is_err());
        // advance() never moves an unmatched or attributed row.
        log.advance("", "deposited", "dd", 1, "");
        assert_eq!(log.row(un).unwrap().phase, "unmatched");
        let r = log.attribute(un, "terp1me", 0).unwrap();
        assert_eq!(
            (r.phase.as_str(), r.terp.as_str(), r.qty),
            ("attributed", "terp1me", 2)
        );
        log.advance("terp1me", "deposited", "dd", 1, "");
        assert_eq!(log.row(un).unwrap().phase, "attributed");
        assert!(!log.refund("terp1me"));
    }
}
