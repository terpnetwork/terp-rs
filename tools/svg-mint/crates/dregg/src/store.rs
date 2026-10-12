//! Durable cell snapshot. Process restart must keep spent nullifiers.

use crate::cell::Cell;
use crate::payout::RailBooks;
use crate::pedersen::{self, Commit};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PendingPay {
    pub terp: String,
    pub blinding: [u8; 32],
    pub hd_index: u32,
    pub eth_addr: String,
    pub btc_addr: String,
    pub zec_div_hex: String,
    pub zec_ua: Option<String>,
    /// Unix seconds when pay-targets issued this dest. `0` = pre-TTL snapshot.
    #[serde(default)]
    pub issued_at: u64,
    /// Paid unit count requested at issue (`?qty=N`). Default 1 for old snapshots.
    #[serde(default = "pending_qty_default")]
    pub qty: u32,
    /// Per-checkout instance mixed into the Pedersen message. Empty = legacy.
    #[serde(default)]
    pub session: Vec<u8>,
}

fn pending_qty_default() -> u32 {
    1
}

/// Bind a payment memo to the unpaid dest. Verifies Pedersen so a random C cannot steal.
pub fn dest_for_commit(pending: &HashMap<Commit, PendingPay>, c: &Commit) -> Option<String> {
    let p = pending.get(c)?;
    if pedersen::verify_session(&p.terp, &p.blinding, &p.session, c) {
        Some(p.terp.clone())
    } else {
        None
    }
}

/// Same bind the ZEC watcher uses: JSON blob, then `list-received` memo field.
pub fn dest_for_zec_watch(
    pending: &HashMap<Commit, PendingPay>,
    memo: &str,
    blob: &[u8],
) -> Option<String> {
    if let Some(c) = pedersen::parse_commit(blob) {
        if let Some(t) = dest_for_commit(pending, &c) {
            return Some(t);
        }
    }
    crate::zip321::commit_from_watch_memo(memo).and_then(|c| dest_for_commit(pending, &c))
}

/// Drop unpaid pay-targets for one Terp dest (Cancel checkout). Does not touch the cell.
pub fn cancel_unpaid(pending: &mut HashMap<Commit, PendingPay>, terp: &str) -> Vec<PendingPay> {
    let mut dropped = Vec::new();
    pending.retain(|_, p| {
        if p.terp == terp {
            dropped.push(p.clone());
            false
        } else {
            true
        }
    });
    dropped
}

/// HTTP status + body for `GET /v1/cancel`. Paid cell phases win; does not flip checkout_ready.
pub fn cancel_http(paid_phase: Option<&str>, dropped: usize) -> (u16, serde_json::Value) {
    if let Some(phase) = paid_phase {
        return (
            409,
            serde_json::json!({
                "ok": false,
                "error": "payment already seen",
                "phase": phase,
            }),
        );
    }
    if dropped > 0 {
        (
            200,
            serde_json::json!({
                "ok": true,
                "cancelled": true,
                "phase": "waiting",
            }),
        )
    } else {
        (200, serde_json::json!({"ok": true, "cancelled": false}))
    }
}

/// Drop unpaid carts past TTL. Returns dropped rows (for watch-only relabel).
pub fn expire_unpaid(
    pending: &mut HashMap<Commit, PendingPay>,
    now: u64,
    ttl_secs: u64,
) -> Vec<PendingPay> {
    expire_unpaid_keeping(pending, now, ttl_secs, &std::collections::HashSet::new())
}

/// Same as [`expire_unpaid`], but a checkout already matched in the mempool stays.
pub fn expire_unpaid_keeping(
    pending: &mut HashMap<Commit, PendingPay>,
    now: u64,
    ttl_secs: u64,
    keep_terp: &std::collections::HashSet<String>,
) -> Vec<PendingPay> {
    let mut dropped = Vec::new();
    pending.retain(|_, p| {
        if keep_terp.contains(&p.terp) {
            return true;
        }
        if crate::checkout::pending_expired(p.issued_at, now, ttl_secs) {
            dropped.push(p.clone());
            false
        } else {
            true
        }
    });
    dropped
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FileSnap {
    cell: Cell,
    #[serde(default)]
    books: RailBooks,
    #[serde(default)]
    pending: HashMap<String, PendingPay>,
    #[serde(default)]
    hd_index: u32,
    #[serde(default)]
    bogo_seen: u64,
}

pub struct Durable {
    path: PathBuf,
}

impl Durable {
    pub fn open(
        path: impl AsRef<Path>,
    ) -> std::io::Result<(Self, Cell, RailBooks, HashMap<Commit, PendingPay>, u32, u64)> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let (cell, books, pending, hd_index, bogo_seen) = if path.is_file() {
            let bytes = fs::read(&path)?;
            if let Ok(snap) = serde_json::from_slice::<FileSnap>(&bytes) {
                let mut m = HashMap::new();
                for (k, v) in snap.pending {
                    if let Ok(b) = hex::decode(&k) {
                        if b.len() == 32 {
                            let mut c = [0u8; 32];
                            c.copy_from_slice(&b);
                            m.insert(c, v);
                        }
                    }
                }
                (snap.cell, snap.books, m, snap.hd_index, snap.bogo_seen)
            } else {
                (
                    serde_json::from_slice(&bytes).unwrap_or_default(),
                    RailBooks::default(),
                    HashMap::new(),
                    0,
                    0,
                )
            }
        } else {
            (Cell::default(), RailBooks::default(), HashMap::new(), 0, 0)
        };
        Ok((Self { path }, cell, books, pending, hd_index, bogo_seen))
    }

    pub fn save(
        &self,
        cell: &Cell,
        books: &RailBooks,
        pending: &HashMap<Commit, PendingPay>,
        hd_index: u32,
        bogo_seen: u64,
    ) -> std::io::Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        let mut p = HashMap::new();
        for (c, v) in pending {
            p.insert(hex::encode(c), v.clone());
        }
        let snap = FileSnap {
            cell: cell.clone(),
            books: books.clone(),
            pending: p,
            hd_index,
            bogo_seen,
        };
        let bytes = serde_json::to_vec(&snap)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pay(issued_at: u64) -> PendingPay {
        PendingPay {
            terp: "terp1x".into(),
            blinding: [1u8; 32],
            hd_index: 0,
            eth_addr: "0xabc".into(),
            btc_addr: "bcrt1qtest".into(),
            zec_div_hex: String::new(),
            zec_ua: None,
            issued_at,
            qty: 1,
            session: Vec::new(),
        }
    }

    #[test]
    fn expire_drops_stale_and_unknown_age() {
        let mut m = HashMap::new();
        m.insert([1u8; 32], pay(0));
        m.insert([2u8; 32], pay(100));
        m.insert([3u8; 32], pay(500));
        let dropped = expire_unpaid(&mut m, 200, 60);
        assert_eq!(dropped.len(), 2);
        assert!(!m.contains_key(&[1u8; 32]));
        assert!(!m.contains_key(&[2u8; 32]));
        assert!(m.contains_key(&[3u8; 32]));
    }

    #[test]
    fn cancel_unpaid_drops_only_that_dest() {
        let mut m = HashMap::new();
        let mut a = pay(10);
        a.terp = "terp1aaa".into();
        let mut b = pay(10);
        b.terp = "terp1bbb".into();
        m.insert([1u8; 32], a);
        m.insert([2u8; 32], b);
        let dropped = cancel_unpaid(&mut m, "terp1aaa");
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].terp, "terp1aaa");
        assert_eq!(m.len(), 1);
        assert!(cancel_unpaid(&mut m, "terp1aaa").is_empty());
    }

    #[test]
    fn cancel_http_table() {
        let (code, body) = cancel_http(None, 1);
        assert_eq!(code, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["cancelled"], true);
        assert_eq!(body["phase"], "waiting");
        assert!(body.get("checkout_ready").is_none());

        let (code, body) = cancel_http(None, 0);
        assert_eq!(code, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["cancelled"], false);

        for phase in ["Deposited", "Entitled", "Settled"] {
            let (code, body) = cancel_http(Some(phase), 1);
            assert_eq!(code, 409);
            assert_eq!(body["ok"], false);
            assert_eq!(body["error"], "payment already seen");
            assert_eq!(body["phase"], phase);
        }
    }

    #[test]
    fn ttl_zero_never_expires() {
        let mut m = HashMap::new();
        m.insert([9u8; 32], pay(1));
        assert!(expire_unpaid(&mut m, 99_999, 0).is_empty());
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn zec_watch_identifies_hex_ascii_and_zip321_memo() {
        use crate::pedersen::{self, memo_bytes, memo_hex};
        use crate::zip321::{self, payment_request};
        use rand::rngs::OsRng;

        let terp = "terp1cyyzpxplxdzkeea7kwsydadg87357qnacvvp23";
        let (c, r) = pedersen::commit(terp, &mut OsRng);
        let mut pending = HashMap::new();
        let mut row = pay(10);
        row.terp = terp.into();
        row.blinding = r;
        pending.insert(c, row);

        let uri = payment_request("u1testaddressxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", 450_000, &c).unwrap();
        let param = uri.split("memo=").nth(1).unwrap();
        let hex = memo_hex(&c);
        let decoded = zip321::decode_memo_bytes(param).expect("b64url");
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), hex);
        assert!(decoded.iter().all(|b| b.is_ascii_hexdigit()));

        let mut sapling = hex.as_bytes().to_vec();
        sapling.resize(512, 0xf6);
        let sapling_hex = hex::encode(&sapling);

        let list_json = serde_json::json!([{
            "txid": "aa".repeat(32),
            "amountZat": 450000,
            "memo": hex,
        }])
        .to_string();

        for (memo, blob) in [
            (param, param.as_bytes()),
            (hex.as_str(), hex.as_bytes()),
            (hex.as_str(), sapling.as_slice()),
            (param, list_json.as_bytes()),
            (hex.as_str(), sapling_hex.as_bytes()),
        ] {
            let got = dest_for_zec_watch(&pending, memo, blob);
            assert_eq!(got.as_deref(), Some(terp), "memo={memo:.24}");
        }

        let got_legacy = dest_for_zec_watch(&pending, "", &memo_bytes(&c));
        assert_eq!(got_legacy.as_deref(), Some(terp));
        assert!(dest_for_zec_watch(&pending, "not-a-memo", b"{}").is_none());
    }
}
