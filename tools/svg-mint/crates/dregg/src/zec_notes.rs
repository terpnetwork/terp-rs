//! Notes the house view wallet has already trial-decrypted.
//!
//! A note is a checkout's payment only when its memo is that checkout's one-time
//! payment id (Pedersen C, carried in the ZIP-321 `memo=`), it pays at least the
//! checkout price, and it was not mined before the checkout was issued. Every
//! checkout shares one house UA, so the address or receiver never picks a checkout.
//! A mempool note (`mined == false`) is receipt, not a mint. A mined note is the block signal.

use crate::pedersen::Commit;
use crate::store::{PendingPay, dest_for_commit};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    /// The full note was decrypted and `memo` is its real memo (possibly empty).
    pub memo_checked: bool,
    /// Block time of the mined tx (unix seconds). 0 = unknown or mempool.
    pub time: u64,
    pub height: u64,
}

/// Block timestamps may run behind wall clock; consensus allows about two hours.
pub const PAY_TIME_SKEW_SECS: u64 = 2 * 60 * 60;

/// Why a note did or did not bind to a checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoMatch {
    Match { commit: Commit, terp: String },
    Pool,
    NoMemo,
    UnknownMemo,
    Underpaid { need: u64 },
    BeforeCheckout,
}

impl MemoMatch {
    pub fn reason(&self) -> &'static str {
        match self {
            MemoMatch::Match { .. } => "matched",
            MemoMatch::Pool => "pool",
            MemoMatch::NoMemo => "no memo",
            MemoMatch::UnknownMemo => "unknown memo",
            MemoMatch::Underpaid { .. } => "underpaid",
            MemoMatch::BeforeCheckout => "before checkout",
        }
    }
}

/// Memo-only binding. `unit_fallback` is the per-NFT zat for snapshots with no quote.
pub fn match_note_memo(
    pending: &HashMap<Commit, PendingPay>,
    note: &SeenNote,
    unit_fallback: u64,
) -> MemoMatch {
    if !pool_accepted(&note.pool) {
        return MemoMatch::Pool;
    }
    let memo = note.memo.trim();
    if memo.is_empty() || memo == "Memo::Empty" {
        return MemoMatch::NoMemo;
    }
    let Some(c) = crate::zip321::commit_from_watch_memo(memo) else {
        return MemoMatch::UnknownMemo;
    };
    let Some(terp) = dest_for_commit(pending, &c) else {
        return MemoMatch::UnknownMemo;
    };
    let Some(p) = pending.get(&c) else {
        return MemoMatch::UnknownMemo;
    };
    let unit = if p.zec_zat > 0 { p.zec_zat } else { unit_fallback };
    let need = unit.saturating_mul(p.qty.max(1) as u64);
    if note.zat < need {
        return MemoMatch::Underpaid { need };
    }
    if note.time > 0 && p.issued_at > 0 && note.time.saturating_add(PAY_TIME_SKEW_SECS) < p.issued_at {
        return MemoMatch::BeforeCheckout;
    }
    MemoMatch::Match { commit: c, terp }
}

/// Checkout address for this note, by memo only (any amount, any time). See `match_note_memo`.
pub fn match_note(pending: &HashMap<Commit, PendingPay>, note: &SeenNote) -> Option<String> {
    match match_note_memo(pending, note, 0) {
        MemoMatch::Match { terp, .. } => Some(terp),
        MemoMatch::Underpaid { .. } | MemoMatch::BeforeCheckout => {
            let c = crate::zip321::commit_from_watch_memo(note.memo.trim())?;
            dest_for_commit(pending, &c)
        }
        _ => None,
    }
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
                memo: if memo == "Memo::Empty" {
                    String::new()
                } else {
                    memo.trim().to_string()
                },
                zat,
                mined,
                pool: if pool.is_empty() {
                    "ironwood".into()
                } else {
                    pool.to_string()
                },
                receiver: String::new(),
                memo_checked: true,
                time: 0,
                height: 0,
            });
        }
    };
    for line in text.lines() {
        let trim = line.trim();
        if trim.len() == 64 && trim.chars().all(|c| c.is_ascii_hexdigit()) {
            flush(
                &mut notes, &txid, mined, in_output, received, &address, &memo, &pool, zat,
            );
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
            flush(
                &mut notes, &txid, mined, in_output, received, &address, &memo, &pool, zat,
            );
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
    flush(
        &mut notes, &txid, mined, in_output, received, &address, &memo, &pool, zat,
    );
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
    whole
        .saturating_mul(100_000_000)
        .saturating_add(frac_digits.parse().unwrap_or(0))
}

pub fn parse_notes_json(raw: &str) -> Result<Vec<SeenNote>, String> {
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let arr = v
        .get("notes")
        .and_then(|n| n.as_array())
        .ok_or("notes array")?;
    let mut out = Vec::new();
    for n in arr {
        out.push(SeenNote {
            txid: n
                .get("txid")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            address: n
                .get("address")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            memo: n
                .get("memo")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            zat: n.get("zat").and_then(|x| x.as_u64()).unwrap_or(0),
            mined: n.get("mined").and_then(|x| x.as_bool()).unwrap_or(true),
            pool: n
                .get("pool")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            receiver: n
                .get("receiver")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            memo_checked: n.get("memo_checked").and_then(|x| x.as_bool()).unwrap_or(false),
            time: n.get("time").and_then(|x| x.as_u64()).unwrap_or(0),
            height: n.get("height").and_then(|x| x.as_u64()).unwrap_or(0),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pedersen;
    use crate::store::PendingPay;

    /// The one house UA every checkout shares (receiver 0x22…).
    fn house_ua() -> String {
        use zcash_address::unified::{Address as UAddr, Encoding, Receiver};
        UAddr::try_from_items(vec![Receiver::Orchard([0x22u8; 43])])
            .expect("receiver")
            .encode(&zcash_protocol::consensus::NetworkType::Main)
    }

    fn pending(terp: &str, qty: u32, issued_at: u64) -> (Commit, PendingPay) {
        let (c, blinding) = pedersen::commit(terp, &mut rand::rngs::OsRng);
        let p = PendingPay {
            terp: terp.into(),
            blinding,
            hd_index: 1,
            eth_addr: String::new(),
            btc_addr: String::new(),
            zec_div_hex: String::new(),
            zec_ua: Some(house_ua()),
            issued_at,
            qty,
            session: Vec::new(),
            collection: String::new(),
            zec_zat: 100_000,
        };
        (c, p)
    }

    fn note(memo: &str, zat: u64, time: u64) -> SeenNote {
        SeenNote {
            txid: "ee".repeat(32),
            address: String::new(),
            memo: memo.into(),
            zat,
            mined: true,
            pool: "ironwood".into(),
            receiver: hex::encode([0x22u8; 43]),
            memo_checked: true,
            time,
            height: 10,
        }
    }

    #[test]
    fn memo_matches_only_its_checkout() {
        let (c1, p1) = pending("terp1a", 1, 1_000);
        let (c2, p2) = pending("terp1b", 1, 1_000);
        let mut map = HashMap::new();
        map.insert(c1, p1);
        map.insert(c2, p2);
        let n = note(&pedersen::memo_hex(&c2), 100_000, 2_000);
        assert_eq!(
            match_note_memo(&map, &n, 0),
            MemoMatch::Match { commit: c2, terp: "terp1b".into() }
        );
        assert_eq!(match_note(&map, &n).as_deref(), Some("terp1b"));
        // The same memo as the ZIP-321 base64url param also binds.
        let b64 = crate::zip321::memo_b64url(pedersen::memo_hex(&c1).as_bytes()).unwrap();
        assert_eq!(match_note(&map, &note(&b64, 100_000, 2_000)).as_deref(), Some("terp1a"));
    }

    #[test]
    fn zip321_uri_carries_the_matching_memo() {
        let (c, p) = pending("terp1uri", 1, 1_000);
        let uri = crate::zip321::payment_request(&house_ua(), 100_000, &c).unwrap();
        let b64 = uri.split("memo=").nth(1).expect("memo param");
        let mut map = HashMap::new();
        map.insert(c, p);
        let raw = crate::zip321::decode_memo_bytes(b64).unwrap();
        let memo = String::from_utf8(raw).unwrap();
        assert!(memo.starts_with("6431"));
        assert_eq!(match_note(&map, &note(&memo, 100_000, 2_000)).as_deref(), Some("terp1uri"));
    }

    #[test]
    fn wrong_memo_does_not_match() {
        let (c1, p1) = pending("terp1a", 1, 1_000);
        let (other, _) = pending("terp1z", 1, 1_000);
        let mut map = HashMap::new();
        map.insert(c1, p1);
        assert_eq!(match_note_memo(&map, &note(&pedersen::memo_hex(&other), 100_000, 2_000), 0), MemoMatch::UnknownMemo);
        assert_eq!(match_note_memo(&map, &note("thanks for the art", 100_000, 2_000), 0), MemoMatch::UnknownMemo);
        assert!(match_note(&map, &note(&pedersen::memo_hex(&other), 100_000, 2_000)).is_none());
    }

    #[test]
    fn no_memo_does_not_match_even_on_the_checkout_receiver() {
        let (c1, p1) = pending("terp1a", 1, 1_000);
        let mut map = HashMap::new();
        map.insert(c1, p1);
        assert_eq!(match_note_memo(&map, &note("", 100_000, 2_000), 0), MemoMatch::NoMemo);
        assert_eq!(match_note_memo(&map, &note("Memo::Empty", 100_000, 2_000), 0), MemoMatch::NoMemo);
        let mut by_addr = note("", 100_000, 2_000);
        by_addr.address = house_ua();
        assert!(match_note(&map, &by_addr).is_none());
    }

    #[test]
    fn amount_and_time_are_checked() {
        let (c, p) = pending("terp1q", 2, 100_000);
        let mut map = HashMap::new();
        map.insert(c, p);
        let memo = pedersen::memo_hex(&c);
        assert_eq!(match_note_memo(&map, &note(&memo, 199_999, 100_100), 0), MemoMatch::Underpaid { need: 200_000 });
        assert!(matches!(match_note_memo(&map, &note(&memo, 200_000, 100_100), 0), MemoMatch::Match { .. }));
        assert!(matches!(match_note_memo(&map, &note(&memo, 250_000, 100_100), 0), MemoMatch::Match { .. }));
        assert_eq!(match_note_memo(&map, &note(&memo, 200_000, 10), 0), MemoMatch::BeforeCheckout);
        // Block time inside the consensus skew still counts; mempool (time 0) is not dated.
        assert!(matches!(match_note_memo(&map, &note(&memo, 200_000, 100_000 - 600), 0), MemoMatch::Match { .. }));
        assert!(matches!(match_note_memo(&map, &note(&memo, 200_000, 0), 0), MemoMatch::Match { .. }));
    }

    #[test]
    fn the_nov18_waiting_note_stays_unmatched() {
        // a39435e8…104b: 200000 zat to the shared house receiver, paid to a settled checkout.
        let (settled, _) = pending("terp1paidbefore", 2, 1_000);
        let (fresh_c, fresh_p) = pending("terp1someoneelse", 2, 1_795_100_000);
        let mut map = HashMap::new();
        map.insert(fresh_c, fresh_p);
        let mut n = note("", 200_000, 1_795_090_000);
        n.txid = "a39435e84e3d8a919f0c03682c5f008db79f266d71488e0dcd198a13750f104b".into();
        assert_eq!(match_note_memo(&map, &n, 0), MemoMatch::NoMemo);
        n.memo = pedersen::memo_hex(&settled);
        assert_eq!(match_note_memo(&map, &n, 0), MemoMatch::UnknownMemo);
        assert!(match_note(&map, &n).is_none());
    }

    const SAMPLE: &str = "\
Transactions:\n\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
     Unmined\n\
  Output 0 (Ironwood)\n\
    Value:   0.00150000 ZEC\n\
    Received by account: acct (dregg)\n\
    To: u1checkout\n\
    Memo: Memo::Empty\n\
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n\
     Mined: 10\n\
  Output 0 (Ironwood)\n\
    Value:   0.00150000 ZEC\n\
    Received by account: acct (dregg)\n\
    To: u1other\n\
    Memo: Memo::Empty\n";

    #[test]
    fn mined_note_is_the_block_signal() {
        let notes = parse_list_tx(SAMPLE);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].zat, 150_000);
        assert!(notes[1].mined);
        assert!(!notes[0].mined);
        assert!(notes[0].memo.is_empty());
    }

    #[test]
    fn notes_json_reads_memo_fields() {
        let raw = r#"{"notes":[{"txid":"ab","memo":"6431","zat":5,"mined":true,"pool":"ironwood","receiver":"","memo_checked":true,"time":7,"height":9}]}"#;
        let n = parse_notes_json(raw).unwrap();
        assert!(n[0].memo_checked);
        assert_eq!((n[0].time, n[0].height), (7, 9));
        let old = parse_notes_json(r#"{"notes":[{"txid":"ab","zat":5}]}"#).unwrap();
        assert!(!old[0].memo_checked);
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
        let (c, p) = pending("terp1desk", 1, 1_000);
        let mut map = HashMap::new();
        map.insert(c, p);
        let mut n = note(&pedersen::memo_hex(&c), 100_000, 2_000);
        n.pool = "orchard".into();
        assert_eq!(match_note_memo(&map, &n, 0), MemoMatch::Pool);
    }

    #[test]
    fn ironwood_receiver_inside_ua_helper_still_works() {
        assert!(ua_contains_orchard_receiver(&house_ua(), &hex::encode([0x22u8; 43])));
        assert!(!ua_contains_orchard_receiver(&house_ua(), &hex::encode([0x23u8; 43])));
    }
}
