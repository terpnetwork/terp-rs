//! Notes the house view wallet has already trial-decrypted.
//!
//! A note is this checkout only when its address is that checkout's diversified
//! UA, or its memo is that checkout's payment id. A mempool note (`mined == false`)
//! is receipt, not a mint. A mined note is the block signal.

use crate::store::{dest_for_zec_watch, PendingPay};
use crate::pedersen::Commit;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenNote {
    pub txid: String,
    pub address: String,
    pub memo: String,
    pub zat: u64,
    pub mined: bool,
    /// "ironwood" is the only accepted pool. "orchard" and "sapling" never match.
    pub pool: String,
    /// Hex of the 43-byte Orchard receiver inside an Ironwood note.
    pub receiver: String,
}

pub fn match_note(pending: &HashMap<Commit, PendingPay>, note: &SeenNote) -> Option<String> {
    if !pool_accepted(&note.pool) {
        return None;
    }
    let ua = note.address.trim();
    if ua.starts_with("u1") {
        if let Some(p) = pending.values().find(|p| p.zec_ua.as_deref() == Some(ua)) {
            return Some(p.terp.clone());
        }
    }
    let receiver = note.receiver.trim();
    if !receiver.is_empty() {
        if let Some(p) = pending.values().find(|p| {
            p.zec_ua
                .as_deref()
                .is_some_and(|ua| ua_contains_orchard_receiver(ua, receiver))
        }) {
            return Some(p.terp.clone());
        }
    }
    if !note.memo.is_empty() && note.memo != "Memo::Empty" {
        return dest_for_zec_watch(pending, &note.memo, note.memo.as_bytes());
    }
    None
}

fn pool_accepted(pool: &str) -> bool {
    let p = pool.trim();
    p.is_empty() || p.eq_ignore_ascii_case("ironwood")
}

/// ZIP-316 unified address contains this Ironwood/Orchard receiver.
pub fn ua_contains_orchard_receiver(ua: &str, receiver_hex: &str) -> bool {
    let Ok(raw) = hex::decode(receiver_hex.trim()) else {
        return false;
    };
    if raw.len() != 43 {
        return false;
    }
    let mut receiver = [0u8; 43];
    receiver.copy_from_slice(&raw);
    use zcash_address::unified::{Container, Encoding};
    let Ok((_, addr)) = zcash_address::unified::Address::decode(ua) else {
        return false;
    };
    addr.items().into_iter().any(|item| {
        matches!(
            item,
            zcash_address::unified::Receiver::Orchard(bytes) if bytes == receiver
        )
    })
}

/// What a matched list-tx line may do. Unmined is receipt only, never a mint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListTxCredit {
    Skip,
    Mempool,
    Mint,
}

pub fn list_tx_credit(matched_checkout: bool, unmined: bool) -> ListTxCredit {
    if !matched_checkout {
        return ListTxCredit::Skip;
    }
    if unmined {
        ListTxCredit::Mempool
    } else {
        ListTxCredit::Mint
    }
}

/// `zcash-devtool wallet list-tx` text. Incoming outputs only.
pub fn parse_list_tx(text: &str) -> Vec<SeenNote> {
    let mut notes = Vec::new();
    let mut txid = String::new();
    let mut mined = true;
    let mut in_output = false;
    let mut received = false;
    let mut address = String::new();
    let mut memo = String::new();
    let mut pool = String::new();
    let mut zat = 0u64;
    let flush = |notes: &mut Vec<SeenNote>,
                 txid: &str,
                 mined: bool,
                 in_output: bool,
                 received: bool,
                 address: &str,
                 memo: &str,
                 pool: &str,
                 zat: u64| {
        if in_output && received && !txid.is_empty() && pool_accepted(pool) {
            notes.push(SeenNote {
                txid: txid.to_string(),
                address: address.trim().to_string(),
                memo: if memo == "Memo::Empty" { String::new() } else { memo.trim().to_string() },
                zat,
                mined,
                pool: if pool.is_empty() { "ironwood".into() } else { pool.to_string() },
                receiver: String::new(),
            });
        }
    };
    for line in text.lines() {
        let trim = line.trim();
        if trim.len() == 64 && trim.chars().all(|c| c.is_ascii_hexdigit()) {
            flush(&mut notes, &txid, mined, in_output, received, &address, &memo, &pool, zat);
            txid = trim.to_string();
            mined = true;
            in_output = false;
            received = false;
            address.clear();
            memo.clear();
            pool.clear();
            zat = 0;
            continue;
        }
        if trim.contains("Unmined") {
            mined = false;
        }
        if trim.starts_with("Output ") {
            flush(&mut notes, &txid, mined, in_output, received, &address, &memo, &pool, zat);
            in_output = true;
            received = false;
            address.clear();
            memo.clear();
            zat = 0;
            pool = if trim.contains("(Orchard)") || trim.contains("(Sapling)") {
                "orchard".into()
            } else {
                "ironwood".into()
            };
            continue;
        }
        if let Some(rest) = trim.strip_prefix("Value:") {
            zat = zec_line_to_zat(rest);
        }
        if trim.contains("Received by account") {
            received = true;
        }
        if let Some(rest) = trim.strip_prefix("To:") {
            address = rest.trim().to_string();
        }
        if let Some(rest) = trim.strip_prefix("Memo:") {
            memo = rest.trim().to_string();
        }
    }
    flush(&mut notes, &txid, mined, in_output, received, &address, &memo, &pool, zat);
    notes
}

fn zec_line_to_zat(rest: &str) -> u64 {
    let token = rest.split_whitespace().next().unwrap_or("0");
    let mut parts = token.split('.');
    let whole: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let frac = parts.next().unwrap_or("");
    let mut frac_digits = frac.chars().take(8).collect::<String>();
    while frac_digits.len() < 8 {
        frac_digits.push('0');
    }
    whole.saturating_mul(100_000_000).saturating_add(frac_digits.parse().unwrap_or(0))
}

pub fn parse_notes_json(raw: &str) -> Result<Vec<SeenNote>, String> {
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let arr = v.get("notes").and_then(|n| n.as_array()).ok_or("notes array")?;
    let mut out = Vec::new();
    for n in arr {
        out.push(SeenNote {
            txid: n.get("txid").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            address: n.get("address").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            memo: n.get("memo").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            zat: n.get("zat").and_then(|x| x.as_u64()).unwrap_or(0),
            mined: n.get("mined").and_then(|x| x.as_bool()).unwrap_or(true),
            pool: n.get("pool").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            receiver: n.get("receiver").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pedersen;
    use crate::store::PendingPay;

    fn pending(terp: &str, ua: &str) -> (Commit, PendingPay) {
        let (c, blinding) = pedersen::commit(terp, &mut rand::rngs::OsRng);
        let p = PendingPay {
            terp: terp.into(),
            blinding,
            hd_index: 1,
            eth_addr: String::new(),
            btc_addr: String::new(),
            zec_div_hex: String::new(),
            zec_ua: Some(ua.into()),
            issued_at: 1,
            qty: 1,
            session: Vec::new(),
        };
        (c, p)
    }

    const SAMPLE: &str = "\
Transactions:\n\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
     Unmined\n\
  Output 0 (Ironwood)\n\
    Value:   0.00150000 ZEC\n\
    Received by account: acct (dregg)\n\
    To: u1checkoutaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
    Memo: Memo::Empty\n\
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n\
     Mined: 10\n\
  Output 0 (Ironwood)\n\
    Value:   0.00150000 ZEC\n\
    Received by account: acct (dregg)\n\
    To: u1otherbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n\
    Memo: Memo::Empty\n";

    #[test]
    fn empty_memo_mempool_matches_only_this_address() {
        let (c, p) = pending(
            "terp1desk",
            "u1checkoutaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let mut map = HashMap::new();
        map.insert(c, p);
        let notes = parse_list_tx(SAMPLE);
        assert_eq!(notes.len(), 2);
        assert!(!notes[0].mined);
        assert_eq!(notes[0].zat, 150_000);
        assert_eq!(match_note(&map, &notes[0]).as_deref(), Some("terp1desk"));
        assert!(match_note(&map, &notes[1]).is_none());
    }

    #[test]
    fn mined_note_is_the_block_signal() {
        let notes = parse_list_tx(SAMPLE);
        assert!(notes[1].mined);
        assert!(!notes[0].mined);
    }

    #[test]
    fn second_checkout_does_not_take_the_first_address() {
        let (c1, p1) = pending("terp1a", "u1aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (c2, p2) = pending("terp1b", "u1bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let mut map = HashMap::new();
        map.insert(c1, p1);
        map.insert(c2, p2);
        let note = SeenNote {
            txid: "cc".into(),
            address: "u1bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            memo: String::new(),
            zat: 1,
            mined: false,
            pool: "ironwood".into(),
            receiver: String::new(),
        };
        assert_eq!(match_note(&map, &note).as_deref(), Some("terp1b"));
        let first = SeenNote {
            address: "u1aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ..note
        };
        assert_eq!(match_note(&map, &first).as_deref(), Some("terp1a"));
    }

    #[test]
    fn unmined_match_is_receipt_not_a_mint() {
        assert_eq!(list_tx_credit(true, true), ListTxCredit::Mempool);
        assert_eq!(list_tx_credit(true, false), ListTxCredit::Mint);
        assert_eq!(list_tx_credit(false, true), ListTxCredit::Skip);
        assert_eq!(list_tx_credit(false, false), ListTxCredit::Skip);
    }

    #[test]
    fn orchard_output_is_not_accepted() {
        let ua = "u1checkoutaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let text = format!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
     Mined: 10\n\
  Output 0 (Orchard)\n\
    Value:   0.00150000 ZEC\n\
    Received by account: acct (dregg)\n\
    To: {ua}\n\
    Memo: Memo::Empty\n"
        );
        assert!(parse_list_tx(&text).is_empty());
        let (c, p) = pending("terp1desk", ua);
        let mut map = HashMap::new();
        map.insert(c, p);
        let note = SeenNote {
            txid: "aa".into(),
            address: ua.into(),
            memo: String::new(),
            zat: 1,
            mined: true,
            pool: "orchard".into(),
            receiver: String::new(),
        };
        assert!(match_note(&map, &note).is_none());
    }

    #[test]
    fn ironwood_receiver_inside_ua_matches() {
        use zcash_address::unified::{Address as UAddr, Encoding, Receiver};
        let raw = [0x22u8; 43];
        let ua = UAddr::try_from_items(vec![Receiver::Orchard(raw)])
            .expect("receiver")
            .encode(&zcash_protocol::consensus::NetworkType::Main);
        let (c, p) = pending("terp1iron", &ua);
        let mut map = HashMap::new();
        map.insert(c, p);
        let note = SeenNote {
            txid: "dd".into(),
            address: String::new(),
            memo: String::new(),
            zat: 150_000,
            mined: false,
            pool: "ironwood".into(),
            receiver: hex::encode(raw),
        };
        assert_eq!(match_note(&map, &note).as_deref(), Some("terp1iron"));
        let mut other = note.clone();
        other.receiver = hex::encode([0x23u8; 43]);
        assert!(match_note(&map, &other).is_none());
    }
}
