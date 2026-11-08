//! Combined EVM + Solana eligibility tree for `RegisterHeadstash` /
//! `RegisterHeadstashSol`. Encodings mirror `cw-headstash::registration`:
//!
//! - EVM leaf:    `sha256("terp-hs-elig-leaf-v1" || address[20] || amount_le[8])` (48-byte preimage, unchanged)
//! - Solana leaf: `sha256("terp-hs-elig-leaf-v1" || 0x02 || pubkey[32] || amount_le[8])` (61-byte preimage)
//! - node:        `sha256("terp-hs-elig-node-v1" || left || right)`
//! - padding:     the zero leaf `[0; 32]` (no preimage opens it)
//!
//! Index bit `level` clear means the running node is the left child.

use std::collections::BTreeMap;

use base64::Engine;
use sha2::{Digest, Sha256};

pub const SOL_LEAF_TAG: u8 = 0x02;
pub const MAX_DEPTH: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Identity {
    Evm([u8; 20]),
    Sol([u8; 32]),
}

impl Identity {
    /// Same key the contract stores in its one-registration-per-identity map.
    pub fn key(&self) -> String {
        match self {
            Identity::Evm(a) => hex::encode(a),
            Identity::Sol(p) => format!("sol:{}", hex::encode(p)),
        }
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

pub fn leaf(id: &Identity, amount: u64) -> [u8; 32] {
    match id {
        Identity::Evm(a) => evm_leaf(a, amount),
        Identity::Sol(p) => sol_leaf(p, amount),
    }
}

pub fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-node-v1".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    sha256(&pre)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolDrop {
    /// Neither base64(base58) nor base58 decoding produced bytes.
    Encoding,
    /// Decoded, but not exactly 32 bytes.
    Length,
    /// 32 bytes that are not a valid ed25519 point (e.g. a PDA): cannot sign.
    OffCurve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolForm {
    Base64OfBase58,
    Base58,
}

/// Decode one snapshot entry. The documented storage form is base64 of the
/// base58 string; plain base58 is accepted as a fallback and reported.
pub fn decode_sol(entry: &str) -> Result<([u8; 32], SolForm), SolDrop> {
    let entry = entry.trim();
    let (raw, form) = match base64::engine::general_purpose::STANDARD
        .decode(entry)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| bs58::decode(s.trim()).into_vec().ok())
    {
        Some(raw) => (raw, SolForm::Base64OfBase58),
        None => (
            bs58::decode(entry)
                .into_vec()
                .map_err(|_| SolDrop::Encoding)?,
            SolForm::Base58,
        ),
    };
    let pk: [u8; 32] = raw.try_into().map_err(|_| SolDrop::Length)?;
    if ed25519_dalek::VerifyingKey::from_bytes(&pk).is_err() {
        return Err(SolDrop::OffCurve);
    }
    Ok((pk, form))
}

pub fn decode_evm(entry: &str) -> Option<[u8; 20]> {
    let t = entry
        .trim()
        .strip_prefix("0x")
        .or_else(|| entry.trim().strip_prefix("0X"))?;
    let raw = hex::decode(t.to_ascii_lowercase()).ok()?;
    raw.try_into().ok()
}

/// Fixed-depth tree over sorted identities; all levels kept for path lookups.
pub struct EligTree {
    pub depth: u32,
    pub entries: Vec<(Identity, u64)>,
    levels: Vec<Vec<[u8; 32]>>,
}

impl EligTree {
    /// Aggregates duplicate identities (sum of amounts) and sorts by identity key.
    pub fn build(rows: impl IntoIterator<Item = (Identity, u64)>) -> Result<Self, String> {
        let mut agg: BTreeMap<String, (Identity, u64)> = BTreeMap::new();
        for (id, amount) in rows {
            if amount == 0 {
                continue;
            }
            let slot = agg.entry(id.key()).or_insert((id, 0));
            slot.1 = slot.1.checked_add(amount).ok_or("amount overflow")?;
        }
        let entries: Vec<(Identity, u64)> = agg.into_values().collect();
        if entries.is_empty() {
            return Err("empty".into());
        }
        let mut depth = 1u32;
        while (1usize << depth) < entries.len() {
            depth += 1;
        }
        if depth > MAX_DEPTH {
            return Err(format!(
                "{} identities exceed depth {MAX_DEPTH}",
                entries.len()
            ));
        }
        Ok(Self::with_depth(entries, depth))
    }

    pub fn with_depth(entries: Vec<(Identity, u64)>, depth: u32) -> Self {
        let mut level0 = vec![[0u8; 32]; 1usize << depth];
        for (i, (id, amount)) in entries.iter().enumerate() {
            level0[i] = leaf(id, *amount);
        }
        let mut levels = vec![level0];
        for _ in 0..depth {
            let prev = levels.last().unwrap();
            let next = prev.chunks(2).map(|p| node(&p[0], &p[1])).collect();
            levels.push(next);
        }
        Self {
            depth,
            entries,
            levels,
        }
    }

    pub fn root(&self) -> [u8; 32] {
        self.levels[self.depth as usize][0]
    }

    pub fn path(&self, index: usize) -> Vec<[u8; 32]> {
        (0..self.depth as usize)
            .map(|l| self.levels[l][(index >> l) ^ 1])
            .collect()
    }

    pub fn index_of(&self, id: &Identity) -> Option<usize> {
        self.entries.iter().position(|(e, _)| e == id)
    }
}

pub fn open(leaf: [u8; 32], index: usize, path: &[[u8; 32]]) -> [u8; 32] {
    let mut n = leaf;
    for (l, s) in path.iter().enumerate() {
        n = if (index >> l) & 1 == 0 {
            node(&n, s)
        } else {
            node(s, &n)
        };
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_leaf_encoding_is_unchanged() {
        // Live loopback root from the two-leaf EVM eligibility file (relay test vector).
        let a: [u8; 20] = hex::decode("7e5f4552091a69125d5dfcb7b8c2659029395bdf")
            .unwrap()
            .try_into()
            .unwrap();
        let b: [u8; 20] = hex::decode("2b5ad5c4795c026514f8317c7a215e218dccd6cf")
            .unwrap()
            .try_into()
            .unwrap();
        let t = EligTree::with_depth(vec![(Identity::Evm(a), 15), (Identity::Evm(b), 15)], 1);
        assert!(hex::encode(t.root()).starts_with("ddfd4cb0"));
    }

    #[test]
    fn sol_and_evm_leaves_differ_and_paths_open() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let mut a = [0u8; 20];
        a.copy_from_slice(&pk[..20]);
        assert_ne!(sol_leaf(&pk, 5), evm_leaf(&a, 5));
        let t = EligTree::build(vec![
            (Identity::Sol(pk), 5),
            (Identity::Evm(a), 5),
            (Identity::Sol(pk), 1),
        ])
        .unwrap();
        assert_eq!(t.entries.len(), 2);
        let i = t.index_of(&Identity::Sol(pk)).unwrap();
        assert_eq!(t.entries[i].1, 6);
        assert_eq!(open(sol_leaf(&pk, 6), i, &t.path(i)), t.root());
    }

    #[test]
    fn decode_forms_and_drops() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let b58 = bs58::encode(pk).into_string();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&b58);
        assert_eq!(decode_sol(&b64).unwrap(), (pk, SolForm::Base64OfBase58));
        assert_eq!(decode_sol(&b58).unwrap(), (pk, SolForm::Base58));
        assert_eq!(decode_sol("0OIl").unwrap_err(), SolDrop::Encoding);
        assert_eq!(
            decode_sol(&bs58::encode([1u8; 31]).into_string()).unwrap_err(),
            SolDrop::Length
        );
        // Find a 32-byte string that is not a curve point.
        let off = (0u8..=255)
            .map(|b| {
                let mut x = [b; 32];
                x[31] &= 0x7f;
                x
            })
            .find(|x| ed25519_dalek::VerifyingKey::from_bytes(x).is_err())
            .unwrap();
        assert_eq!(
            decode_sol(&bs58::encode(off).into_string()).unwrap_err(),
            SolDrop::OffCurve
        );
    }
}

// ---------------------------------------------------------------------------
// Exact decimal amounts -> minimal denom, and floor-rounded uterp allocation.
// ---------------------------------------------------------------------------

/// Parse an exact decimal string (`12.5`, `7`, `0.000000000000000001`) into
/// integer base units with `decimals` fractional digits. No floats. Rejects
/// signs, exponents, empty parts, non-digits, more fractional digits than
/// `decimals`, and u128 overflow. Surrounding whitespace (incl. CR) is trimmed.
pub fn parse_base_units(s: &str, decimals: u32) -> Result<u128, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty".into());
    }
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if int.is_empty() || (s.contains('.') && frac.is_empty()) {
        return Err(format!("malformed: {s}"));
    }
    if !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("malformed: {s}"));
    }
    let frac_t = frac.trim_end_matches('0');
    if frac_t.len() as u32 > decimals {
        return Err(format!("more than {decimals} fractional digits: {s}"));
    }
    let scale = 10u128.checked_pow(decimals).ok_or("decimals too large")?;
    let mut iv: u128 = 0;
    for b in int.bytes() {
        iv = iv
            .checked_mul(10)
            .and_then(|v| v.checked_add((b - b'0') as u128))
            .ok_or(format!("overflow: {s}"))?;
    }
    let mut fv: u128 = 0;
    for b in frac_t.bytes() {
        fv = fv * 10 + (b - b'0') as u128;
    }
    fv *= 10u128.pow(decimals - frac_t.len() as u32);
    iv.checked_mul(scale)
        .and_then(|v| v.checked_add(fv))
        .ok_or(format!("overflow: {s}"))
}

/// floor(a * b / d) without overflow for any u128 inputs where the true result fits u128.
pub fn mul_div_floor(a: u128, b: u128, d: u128) -> Option<u128> {
    if d == 0 {
        return None;
    }
    if let Some(p) = a.checked_mul(b) {
        return Some(p / d);
    }
    // Exact 256-bit product (hi, lo), then bitwise long division by d.
    const M: u128 = u64::MAX as u128;
    let (a0, a1, b0, b1) = (a & M, a >> 64, b & M, b >> 64);
    let (ll, lh, hl, hh) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (ll >> 64) + (lh & M) + (hl & M);
    let lo = (ll & M) | (mid << 64);
    let hi = hh + (lh >> 64) + (hl >> 64) + (mid >> 64);
    if hi >= d {
        return None;
    } // quotient would not fit u128
    let (mut r, mut q) = (hi, 0u128);
    for i in (0..128).rev() {
        let carry = r >> 127;
        r = (r << 1) | ((lo >> i) & 1);
        q <<= 1;
        if carry == 1 || r >= d {
            r = r.wrapping_sub(d);
            q |= 1;
        }
    }
    Some(q)
}

/// Pro-rata split of `budget` uterp over `balances` (base units), floor rounded.
/// Returns per-holder u64 amounts (same order) and the undistributed dust.
/// Deterministic: depends only on the inputs, not their order of evaluation.
pub fn allocate_floor(balances: &[u128], budget: u64) -> Result<(Vec<u64>, u64), String> {
    let total = balances
        .iter()
        .try_fold(0u128, |a, b| a.checked_add(*b))
        .ok_or("total overflow")?;
    if total == 0 {
        return Ok((vec![0; balances.len()], budget));
    }
    let mut out = Vec::with_capacity(balances.len());
    let mut given: u128 = 0;
    for &bal in balances {
        let v = mul_div_floor(budget as u128, bal, total).ok_or("mul_div overflow")?;
        given += v;
        out.push(u64::try_from(v).map_err(|_| "allocation exceeds u64")?);
    }
    Ok((out, (budget as u128 - given) as u64))
}

/// Drop excluded holders after the pro-rata split. The split is computed over
/// the ORIGINAL community total, so kept holders' amounts are unchanged and the
/// excluded holders' share is returned as unallocated (never redistributed).
/// Returns (kept rows, unallocated uterp, excluded row count).
pub fn apply_exclusions(
    rows: Vec<(Identity, u64)>,
    excluded: &std::collections::BTreeSet<Identity>,
) -> (Vec<(Identity, u64)>, u128, u64) {
    let (mut kept, mut unallocated, mut n) = (Vec::with_capacity(rows.len()), 0u128, 0u64);
    for (id, a) in rows {
        if excluded.contains(&id) {
            unallocated += a as u128;
            n += 1;
        } else {
            kept.push((id, a));
        }
    }
    (kept, unallocated, n)
}

#[cfg(test)]
mod exclusion_tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn excluded_share_stays_unallocated_and_others_unchanged() {
        let ids = [
            Identity::Evm([1; 20]),
            Identity::Evm([2; 20]),
            Identity::Sol([3; 32]),
        ];
        let bals = [5u128, 3, 2];
        let budget = 1_000_003u64;
        let (alloc, dust) = allocate_floor(&bals, budget).unwrap();
        let rows: Vec<_> = ids.iter().copied().zip(alloc.iter().copied()).collect();
        let ex: BTreeSet<_> = [ids[1]].into_iter().collect();
        let (kept, unalloc, n) = apply_exclusions(rows, &ex);
        assert_eq!(n, 1);
        assert_eq!(kept, vec![(ids[0], alloc[0]), (ids[2], alloc[2])]);
        assert_eq!(unalloc, alloc[1] as u128);
        let leaves: u128 = kept.iter().map(|r| r.1 as u128).sum();
        assert_eq!(leaves + dust as u128 + unalloc, budget as u128);
    }

    #[test]
    fn no_exclusions_is_identity() {
        let rows = vec![(Identity::Evm([9; 20]), 7u64)];
        let (kept, u, n) = apply_exclusions(rows.clone(), &BTreeSet::new());
        assert_eq!((kept, u, n), (rows, 0, 0));
    }
}

#[cfg(test)]
mod amount_tests {
    use super::*;

    #[test]
    fn fractional_parsing() {
        assert_eq!(
            parse_base_units("12.5", 18).unwrap(),
            12_500_000_000_000_000_000
        );
        assert_eq!(parse_base_units("0.000000000000000001", 18).unwrap(), 1);
        assert_eq!(parse_base_units("1.2300", 2).unwrap(), 123);
        assert_eq!(
            parse_base_units("52230931.14\r", 18).unwrap(),
            52_230_931_140_000_000_000_000_000
        );
        assert_eq!(parse_base_units("42", 0).unwrap(), 42);
        assert_eq!(parse_base_units("42", 6).unwrap(), 42_000_000);
        assert_eq!(parse_base_units("7.000", 0).unwrap(), 7);
    }

    #[test]
    fn invalid_values() {
        for bad in [
            "", "-1", "+1", "1e5", "1.", ".5", "1.2.3", "abc", "1,5", " 1 2",
        ] {
            assert!(parse_base_units(bad, 18).is_err(), "{bad:?}");
        }
        assert!(parse_base_units("0.001", 2).is_err());
        assert!(parse_base_units("340282366920938463463374607431768211456", 0).is_err());
    }

    #[test]
    fn eighteen_decimal_overflow() {
        // 52M tokens at 18 decimals does not fit u64 but fits u128.
        let bal = parse_base_units("52230931.14", 18).unwrap();
        assert!(u64::try_from(bal).is_err());
        // budget * balance overflows u128; mul_div must still be exact.
        let big = u128::MAX / 3;
        assert_eq!(
            mul_div_floor(big, u64::MAX as u128, big).unwrap(),
            u64::MAX as u128
        );
        let (a, dust) = allocate_floor(&[bal, bal], u64::MAX).unwrap();
        assert_eq!(a[0] as u128 * 2 + dust as u128, u64::MAX as u128);
        assert!(parse_base_units("400000000000000000000.0", 18).is_err());
    }

    #[test]
    fn rounding_is_floor_and_deterministic() {
        let (a, dust) = allocate_floor(&[1, 1, 1], 10).unwrap();
        assert_eq!((a.clone(), dust), (vec![3, 3, 3], 1));
        assert_eq!(allocate_floor(&[1, 1, 1], 10).unwrap(), (a, dust));
        let (b, d) = allocate_floor(&[2, 1], 100).unwrap();
        assert_eq!((b, d), (vec![66, 33], 1));
    }
}
