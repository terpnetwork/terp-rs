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
