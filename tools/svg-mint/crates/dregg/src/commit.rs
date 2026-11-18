//! Pedersen-style blake3 commitments. Rail + foreign witness never leave the cell.

use blake3::Hasher;
use serde::{Deserialize, Serialize};

pub const DOMAIN_DEPOSIT: &[u8] = b"dregg-svg-mint-deposit-v1";
pub const DOMAIN_ACTION: &[u8] = b"dregg-svg-mint-action-v1";
pub const DOMAIN_NULLIFIER: &[u8] = b"dregg-svg-mint-nullifier-v1";
pub const DOMAIN_BIND: &[u8] = b"dregg-svg-mint-bind-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rail {
    ZecIronwood,
    Eth,
    Btc,
    /// Base mainnet (chain id 8453). Distinct byte so nullifiers never collide with ETH.
    Base,
}

impl Rail {
    pub fn as_byte(self) -> u8 {
        match self {
            Rail::ZecIronwood => 1,
            Rail::Eth => 2,
            Rail::Btc => 3,
            Rail::Base => 4,
        }
    }

    pub fn price_units(self) -> u128 {
        match self {
            Rail::ZecIronwood => crate::prices::zec_zat() as u128,
            Rail::Eth => crate::prices::ETH_WEI,
            Rail::Btc => crate::prices::BTC_SATS as u128,
            Rail::Base => crate::prices::ETH_WEI,
        }
    }
}

/// Cell-private witness of a foreign payment. Never serialized into packets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignWitness {
    pub rail: Rail,
    pub amount: u128,
    /// Opaque bytes (txid, note cm, vault log). Stays in the cell.
    pub raw: Vec<u8>,
    pub salt: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositOpening {
    pub witness: ForeignWitness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOpening {
    pub collection: String,
    pub svg_hash: [u8; 32],
    pub recipient: String,
    pub session: [u8; 32],
    pub salt: [u8; 32],
}

pub fn commit_deposit(opening: &DepositOpening) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(DOMAIN_DEPOSIT);
    h.update(&[opening.witness.rail.as_byte()]);
    h.update(&opening.witness.amount.to_be_bytes());
    h.update(&(opening.witness.raw.len() as u64).to_be_bytes());
    h.update(&opening.witness.raw);
    h.update(&opening.witness.salt);
    *h.finalize().as_bytes()
}

pub fn nullifier(opening: &DepositOpening) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(DOMAIN_NULLIFIER);
    h.update(&commit_deposit(opening));
    h.update(&opening.witness.salt);
    *h.finalize().as_bytes()
}

pub fn commit_action(opening: &ActionOpening) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(DOMAIN_ACTION);
    h.update(&(opening.collection.len() as u64).to_be_bytes());
    h.update(opening.collection.as_bytes());
    h.update(&opening.svg_hash);
    h.update(&(opening.recipient.len() as u64).to_be_bytes());
    h.update(opening.recipient.as_bytes());
    h.update(&opening.session);
    h.update(&opening.salt);
    *h.finalize().as_bytes()
}

pub fn bind(c_deposit: &[u8; 32], c_action: &[u8; 32], session: &[u8; 32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(DOMAIN_BIND);
    h.update(c_deposit);
    h.update(c_action);
    h.update(session);
    *h.finalize().as_bytes()
}

pub fn verify_deposit(c: &[u8; 32], opening: &DepositOpening) -> bool {
    &commit_deposit(opening) == c
}

pub fn verify_action(c: &[u8; 32], opening: &ActionOpening) -> bool {
    &commit_action(opening) == c
}

/// Opening for an observed foreign deposit: the salt is blake3(raw), as the watchers build it.
pub fn observed_opening(rail: Rail, amount: u128, raw: Vec<u8>) -> DepositOpening {
    let salt = *blake3::hash(&raw).as_bytes();
    DepositOpening { witness: ForeignWitness { rail, amount, raw, salt } }
}

/// A ZEC txid reaches the desk in two byte orders: compact-block scans give the internal
/// order and the house RPC (getrawmempool / getrawtransaction) gives the reversed display
/// order. Both name one payment. Returns the nullifier the same note would have under the
/// other order, so a deposit already credited under one order is not credited again
/// (2026-11-18: one 200000 zat note was credited twice and minted 4 instead of 2).
pub fn zec_txid_alias_nullifier(rail: Rail, amount: u128, raw: &[u8]) -> Option<[u8; 32]> {
    if rail != Rail::ZecIronwood || raw.len() != 32 {
        return None;
    }
    let mut alt = raw.to_vec();
    alt.reverse();
    if alt == raw {
        return None;
    }
    Some(nullifier(&observed_opening(rail, amount, alt)))
}

/// Every nullifier this ZEC payment may already be credited under (both txid byte orders).
/// Read-only: the watcher uses it to tell a credited note from an unmatched one.
pub fn zec_credit_nullifiers(amount: u128, raw: &[u8]) -> Vec<[u8; 32]> {
    let mut v = vec![nullifier(&observed_opening(Rail::ZecIronwood, amount, raw.to_vec()))];
    if let Some(alias) = zec_txid_alias_nullifier(Rail::ZecIronwood, amount, raw) {
        v.push(alias);
    }
    v
}

#[cfg(test)]
mod txid_alias_tests {
    use super::*;

    #[test]
    fn reversed_zec_txid_maps_to_the_first_nullifier() {
        let a: Vec<u8> = (0u8..32).collect();
        let mut b = a.clone();
        b.reverse();
        let n_a = nullifier(&observed_opening(Rail::ZecIronwood, 200_000, a.clone()));
        let n_b = nullifier(&observed_opening(Rail::ZecIronwood, 200_000, b.clone()));
        assert_ne!(n_a, n_b, "the two orders give different nullifiers");
        assert_eq!(zec_txid_alias_nullifier(Rail::ZecIronwood, 200_000, &b), Some(n_a));
        assert_eq!(zec_txid_alias_nullifier(Rail::ZecIronwood, 200_000, &a), Some(n_b));
    }

    /// The 2026-11-18 05:49 UTC mainnet double credit: slots 30e737e8… and bf4f2393… are one note.
    #[test]
    fn the_nov18_double_credit_is_one_note() {
        let h = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
        let first = h("1966b60f6c2e1dee2f797b21dd418aa18fc7d88af04fdf04bf5b9b730fd55b20");
        let second = h("205bd50f739b5bbf04df4ff08ad8c78fa18a41dd217b792fee1d2e6c0fb66619");
        let n_first = nullifier(&observed_opening(Rail::ZecIronwood, 200_000, first));
        let n_second = nullifier(&observed_opening(Rail::ZecIronwood, 200_000, second.clone()));
        assert_eq!(n_first.to_vec(), h("30e737e8421bc458ab8f803ab96f7f6d8e6c1374bb496aff4afd645d77eb41e8"));
        assert_eq!(n_second.to_vec(), h("bf4f2393a356d5fd942688b781d5b4019179114b52002a272387e938bb71c196"));
        assert_eq!(zec_txid_alias_nullifier(Rail::ZecIronwood, 200_000, &second), Some(n_first));
    }

    /// The duplicate guard still holds: one note seen under either byte order is one credit.
    #[test]
    fn credit_nullifiers_cover_both_byte_orders() {
        let h = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
        let first = h("1966b60f6c2e1dee2f797b21dd418aa18fc7d88af04fdf04bf5b9b730fd55b20");
        let second = h("205bd50f739b5bbf04df4ff08ad8c78fa18a41dd217b792fee1d2e6c0fb66619");
        let n_first = nullifier(&observed_opening(Rail::ZecIronwood, 200_000, first.clone()));
        assert!(zec_credit_nullifiers(200_000, &first).contains(&n_first));
        assert!(zec_credit_nullifiers(200_000, &second).contains(&n_first));
        let other = h("a39435e84e3d8a919f0c03682c5f008db79f266d71488e0dcd198a13750f104b");
        assert!(!zec_credit_nullifiers(200_000, &other).contains(&n_first));
    }

    #[test]
    fn other_rails_and_amounts_do_not_alias() {
        let a: Vec<u8> = (0u8..32).collect();
        assert_eq!(zec_txid_alias_nullifier(Rail::Eth, 1, &a), None);
        assert_eq!(zec_txid_alias_nullifier(Rail::ZecIronwood, 1, &a[..20]), None);
        let mut b = a.clone();
        b.reverse();
        let n_other_amount = nullifier(&observed_opening(Rail::ZecIronwood, 100_000, a));
        assert_ne!(zec_txid_alias_nullifier(Rail::ZecIronwood, 200_000, &b), Some(n_other_amount));
    }
}
