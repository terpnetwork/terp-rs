//! ZIP-321 payment request URI.
//!
//! `zcash:<ua>?amount=<ZEC>&memo=<base64url>`
//! Memo *bytes* are ASCII hex of Pedersen (`d1`||C) so Zallet/Zashi/Zold show
//! `6431…` instead of raw binary. ZIP-321 still base64url-encodes those bytes.
//! Never `terp1`. Spec: https://zips.z.cash/zip-0321 — memo ≤ 512 bytes.

use crate::pedersen::{memo_hex, Commit, parse_commit};

const MEMO_MAX: usize = 512;

/// RFC 4648 base64url, no padding (ZIP-321 `param=memo`).
pub fn memo_b64url(memo: &[u8]) -> Result<String, &'static str> {
    if memo.len() > MEMO_MAX {
        return Err("zip-321 memo > 512 bytes");
    }
    Ok(b64url_nopad(memo))
}

/// ZIP-321 URI: memo bytes are ASCII hex of `d1`||C (never terp1, never raw C).
pub fn payment_request(ua: &str, zatoshis: u64, c: &Commit) -> Result<String, &'static str> {
    let ua = ua.trim();
    if ua.is_empty() {
        return Err("empty unified address");
    }
    let memo = memo_b64url(memo_hex(c).as_bytes())?;
    let amount = zat_to_zec_decimal(zatoshis);
    Ok(format!("zcash:{ua}?amount={amount}&memo={memo}"))
}

/// Parse ZIP-321 `amount=` (ZEC decimal) to zatoshis.
pub fn zat_from_zec_decimal(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    let w: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let mut f = frac.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
    if f.len() > 8 {
        f.truncate(8);
    }
    while f.len() < 8 {
        f.push('0');
    }
    let frac_n: u64 = f.parse().ok()?;
    w.checked_mul(100_000_000)?.checked_add(frac_n)
}

pub fn zat_to_zec_decimal(zat: u64) -> String {
    let whole = zat / 100_000_000;
    let frac = zat % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let s = format!("{whole}.{frac:08}");
    s.trim_end_matches('0').to_string()
}

/// Decode ZIP-321 memo param to Pedersen C.
pub fn commit_from_memo_param(b64: &str) -> Option<Commit> {
    let raw = b64url_decode(b64)?;
    parse_commit(&raw)
}

/// Watcher: `list-received` memo may be ZIP-321 base64url **or** decoded hex ASCII
/// (what Zold shows) **or** raw `d1`||C. Same C either way.
pub fn commit_from_watch_memo(memo: &str) -> Option<Commit> {
    let m = memo.trim().trim_matches('"');
    if m.is_empty() {
        return None;
    }
    parse_commit(m.as_bytes()).or_else(|| commit_from_memo_param(m))
}

pub fn decode_memo_bytes(b64: &str) -> Option<Vec<u8>> {
    b64url_decode(b64)
}

fn b64url_nopad(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < data.len() {
            out.push(T[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < data.len() {
            out.push(T[(b2 & 0x3f) as usize] as char);
        }
        i += 3;
    }
    out
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut t = s.trim().replace('+', "-").replace('/', "_");
    t.retain(|c| c != '=');
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::new();
    for c in t.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' => 62,
            '_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERP: &str = "terp1cyyzpxplxdzkeea7kwsydadg87357qnacvvp23";
    const UA: &str = "u1testaddressxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

    #[test]
    fn uri_contains_zcash_scheme_amount_and_commit_memo() {
        let mut rng = rand::rngs::OsRng;
        let (c, _) = crate::pedersen::commit(TERP, &mut rng);
        let u = payment_request(UA, 10_000, &c).unwrap();
        assert!(u.starts_with("zcash:"));
        assert!(u.contains("amount=0.0001"));
        assert!(!u.contains("terp1"));
        let memo = u.split("memo=").nth(1).unwrap();
        assert_eq!(commit_from_memo_param(memo), Some(c));
        let decoded = decode_memo_bytes(memo).expect("b64url");
        assert!(decoded.iter().all(|b| b.is_ascii_hexdigit()));
        let s = std::str::from_utf8(&decoded).unwrap();
        assert!(s.starts_with("6431"));
        assert!(!s.contains("terp1"));
    }

    #[test]
    fn rejects_empty_ua() {
        let c = [2u8; 32];
        assert!(payment_request("", 1, &c).is_err());
    }

    #[test]
    fn zat_decimal() {
        assert_eq!(zat_to_zec_decimal(100_000_000), "1");
        assert_eq!(zat_to_zec_decimal(10_000), "0.0001");
        assert_eq!(zat_to_zec_decimal(1), "0.00000001");
        assert_eq!(zat_from_zec_decimal("0.0045"), Some(450_000));
        assert_eq!(zat_from_zec_decimal("0.0015"), Some(150_000));
    }

    #[test]
    fn qty_scales_amount_not_memo() {
        let mut rng = rand::rngs::OsRng;
        let (c, _) = crate::pedersen::commit(TERP, &mut rng);
        let one = payment_request(UA, 150_000, &c).unwrap();
        let three = payment_request(UA, 450_000, &c).unwrap();
        let m1 = one.split("memo=").nth(1).unwrap();
        let m3 = three.split("memo=").nth(1).unwrap();
        assert_eq!(m1, m3);
        assert!(three.contains("amount=0.0045"));
        assert!(!three.contains("terp1"));
    }
}
