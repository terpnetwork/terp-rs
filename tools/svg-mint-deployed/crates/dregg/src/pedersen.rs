//! Blinded Pedersen commitment over Ristretto.
//!
//! `C = r·G + m·H` with `m = H_scalar(terp1)`. On-chain memo is `C` only.
//! DREGG holds `(terp1, r)` off-chain and checks `C` against the payment.

use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use rand::{CryptoRng, RngCore};

/// 32-byte compressed Ristretto commitment (payment memo body).
pub type Commit = [u8; 32];

fn h_point() -> RistrettoPoint {
    let mut u = [0u8; 64];
    u[..32].copy_from_slice(blake3::hash(b"svg-mint-pedersen-H-v1").as_bytes());
    u[32..].copy_from_slice(blake3::hash(b"svg-mint-pedersen-H-v1-xof").as_bytes());
    RistrettoPoint::from_uniform_bytes(&u)
}

fn msg_scalar(terp: &str, session: &[u8]) -> Scalar {
    if session.is_empty() {
        let h = blake3::hash(terp.trim().as_bytes());
        return Scalar::from_bytes_mod_order(*h.as_bytes());
    }
    let mut h = blake3::Hasher::new();
    h.update(b"svg-mint-pedersen-m-v2");
    h.update(terp.trim().as_bytes());
    h.update(session);
    Scalar::from_bytes_mod_order(*h.finalize().as_bytes())
}

pub fn commit<R: RngCore + CryptoRng>(terp: &str, rng: &mut R) -> (Commit, [u8; 32]) {
    commit_session(terp, &[], rng)
}

/// `session` is per checkout instance. Same address, new session, different memo.
pub fn commit_session<R: RngCore + CryptoRng>(
    terp: &str,
    session: &[u8],
    rng: &mut R,
) -> (Commit, [u8; 32]) {
    let mut r_bytes = [0u8; 32];
    rng.fill_bytes(&mut r_bytes);
    let r = Scalar::from_bytes_mod_order(r_bytes);
    let c = r * RISTRETTO_BASEPOINT_POINT + msg_scalar(terp, session) * h_point();
    (c.compress().to_bytes(), r_bytes)
}

pub fn verify(terp: &str, blinding: &[u8; 32], c: &Commit) -> bool {
    verify_session(terp, blinding, &[], c)
}

pub fn verify_session(terp: &str, blinding: &[u8; 32], session: &[u8], c: &Commit) -> bool {
    let Some(p) = CompressedRistretto(*c).decompress() else {
        return false;
    };
    let r = Scalar::from_bytes_mod_order(*blinding);
    let expect = r * RISTRETTO_BASEPOINT_POINT + msg_scalar(terp, session) * h_point();
    p == expect
}

/// Memo bytes on ETH/BTC/ZIP-321: tag `d1` + compressed C (34 bytes). Never `terp1`.
pub fn memo_bytes(c: &Commit) -> Vec<u8> {
    let mut v = Vec::with_capacity(34);
    v.extend_from_slice(b"d1");
    v.extend_from_slice(c);
    v
}

pub fn memo_hex(c: &Commit) -> String {
    hex::encode(memo_bytes(c))
}

/// First Pedersen C in raw payment data (tag `d1`, ASCII hex, or 32-byte compressed).
/// Sapling memos often append `0xF6` padding — hex ASCII is read from the prefix.
pub fn parse_commit(raw: &[u8]) -> Option<Commit> {
    if let Some(c) = find_tagged(raw) {
        return Some(c);
    }
    if let Some(c) = from_ascii_hex_prefix(raw) {
        return Some(c);
    }
    let s = std::str::from_utf8(raw).unwrap_or("");
    if let Some(c) = from_hexish(s) {
        return Some(c);
    }
    let h = s.trim().trim_start_matches("0x");
    if let Ok(b) = hex::decode(h) {
        if let Some(c) = find_tagged(&b) {
            return Some(c);
        }
        if let Some(c) = from_ascii_hex_prefix(&b) {
            return Some(c);
        }
        if b.len() == 32 {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            if CompressedRistretto(a).decompress().is_some() {
                return Some(a);
            }
        }
    }
    None
}

fn from_ascii_hex_prefix(raw: &[u8]) -> Option<Commit> {
    let hex: String = raw
        .iter()
        .copied()
        .take_while(u8::is_ascii_hexdigit)
        .map(|b| b as char)
        .collect();
    if hex.len() < 68 {
        return None;
    }
    from_hexish(&hex)
}

fn find_tagged(raw: &[u8]) -> Option<Commit> {
    if raw.len() >= 34 {
        for i in 0..=raw.len() - 34 {
            if raw[i] == b'd' && raw[i + 1] == b'1' {
                let mut a = [0u8; 32];
                a.copy_from_slice(&raw[i + 2..i + 34]);
                if CompressedRistretto(a).decompress().is_some() {
                    return Some(a);
                }
            }
        }
    }
    if raw.len() == 32 {
        let mut a = [0u8; 32];
        a.copy_from_slice(raw);
        if CompressedRistretto(a).decompress().is_some() {
            return Some(a);
        }
    }
    None
}

fn from_hexish(s: &str) -> Option<Commit> {
    let i = s.find("6431")?; // hex of "d1"
    let rest = s[i..].trim_start_matches("0x");
    let b = hex::decode(rest.chars().take(68).collect::<String>()).ok()?;
    find_tagged(&b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn sessions_change_the_memo() {
        use rand::rngs::OsRng;
        let mut rng = OsRng;
        let (a, _) = super::commit_session("terp1x", b"session-a", &mut rng);
        let (b, _) = super::commit_session("terp1x", b"session-b", &mut rng);
        assert_ne!(a, b);
        assert_ne!(super::memo_hex(&a), super::memo_hex(&b));
    }

    use super::*;
    use rand::rngs::OsRng;

    #[test]
    fn roundtrip_and_wrong_terp_fails() {
        let mut rng = OsRng;
        let terp = "terp1cyyzpxplxdzkeea7kwsydadg87357qnacvvp23";
        let (c, r) = commit(terp, &mut rng);
        assert!(verify(terp, &r, &c));
        assert!(!verify("terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq2ve3x0", &r, &c));
        let memo = memo_bytes(&c);
        assert!(!String::from_utf8_lossy(&memo).contains("terp1"));
        assert_eq!(parse_commit(&memo), Some(c));
        assert_eq!(parse_commit(memo_hex(&c).as_bytes()), Some(c));
        let mut padded = memo_hex(&c).into_bytes();
        padded.resize(512, 0xf6);
        assert_eq!(parse_commit(&padded), Some(c));
        let hex_of_padded = hex::encode(&padded);
        assert_eq!(parse_commit(hex_of_padded.as_bytes()), Some(c));
    }
}
