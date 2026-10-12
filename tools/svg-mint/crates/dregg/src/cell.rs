//! One-step cell: no mint without an unspent deposit.

use crate::commit::{
    bind, commit_action, commit_deposit, nullifier, ActionOpening, DepositOpening,
};
use crate::observer::confirm_rail;
use crate::packet::MintPacket;
use crate::payout::RailPayout;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CellError {
    #[error("underpay")]
    Underpay,
    #[error("nullifier already spent")]
    DoubleSpend,
    #[error("no unspent deposit")]
    NoDeposit,
    #[error("not entitled")]
    NotEntitled,
    #[error("bind mismatch")]
    BindMismatch,
    #[error("already terminal")]
    Terminal,
    #[error("collection sold out")]
    SoldOut,
    #[error("dummy receipt")]
    DummyReceipt,
    #[error("unsigned or bad quorum")]
    BadQc,
    #[error("rail payout xor")]
    PayoutXor,
    #[error("checkout closed")]
    CheckoutClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum Phase {
    Empty,
    Deposited,
    Entitled,
    Settled,
    Refunded,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Slot {
    phase: Phase,
    c_deposit: [u8; 32],
    n_deposit: [u8; 32],
    amount: u128,
    price: u128,
    #[allow(dead_code)]
    witness_raw: Vec<u8>,
    c_action: Option<[u8; 32]>,
    bind: Option<[u8; 32]>,
    session: Option<[u8; 32]>,
    token_id: Option<String>,
    terp_tx: Option<String>,
    collection: Option<String>,
    #[serde(default)]
    rail: Option<crate::commit::Rail>,
    #[serde(default)]
    payout: crate::payout::RailPayout,
    /// NFTs owed for this deposit (`floor(amount/price)` × BOGO).
    #[serde(default)]
    qty: u32,
    /// Unix seconds when entitled (0 = untimed; sweep ignores).
    #[serde(default)]
    entitled_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CollectionSupply {
    max: u64,
    minted: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Cell {
    slots: HashMap<[u8; 32], Slot>,
    spent: HashSet<[u8; 32]>,
    collections: HashMap<String, CollectionSupply>,
    /// Test/fed clock (unix seconds). 0 = sweep does not fire.
    now: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CellSnap {
    slots: Vec<(String, Slot)>,
    spent: Vec<String>,
    collections: HashMap<String, CollectionSupply>,
}

fn hex32(n: &[u8; 32]) -> String {
    n.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse32(s: &str) -> Option<[u8; 32]> {
    let b = hex::decode(s).ok()?;
    if b.len() != 32 {
        return None;
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&b);
    Some(a)
}

impl serde::Serialize for Cell {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let snap = CellSnap {
            slots: self
                .slots
                .iter()
                .map(|(k, v)| (hex32(k), v.clone()))
                .collect(),
            spent: self.spent.iter().map(hex32).collect(),
            collections: self.collections.clone(),
        };
        snap.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Cell {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let snap = CellSnap::deserialize(deserializer)?;
        let mut slots = HashMap::new();
        for (k, v) in snap.slots {
            if let Some(n) = parse32(&k) {
                slots.insert(n, v);
            }
        }
        let mut spent = HashSet::new();
        for k in snap.spent {
            if let Some(n) = parse32(&k) {
                spent.insert(n);
            }
        }
        Ok(Cell {
            slots,
            spent,
            collections: snap.collections,
            now: 0,
        })
    }
}

#[derive(Clone, Debug)]
pub enum Turn {
    DepositSeen { opening: DepositOpening },
    MintEntitled { n_deposit: [u8; 32], action: ActionOpening },
    MintSettled {
        n_deposit: [u8; 32],
        token_id: String,
        terp_tx: String,
        receipt: [u8; 32],
    },
    DepositRefunded { n_deposit: [u8; 32] },
    /// Bind a collection max supply (query-refunds use this).
    SetCollection { id: String, max_supply: u64 },
    /// On-chain supply observed at a queried height (must carry a real receipt digest).
    ObserveSupply {
        collection: String,
        minted: u64,
        max_supply: u64,
        height: u64,
        receipt: [u8; 32],
    },
    /// Set cell clock so entitle timestamps and stale-mint refunds are deterministic.
    Tick { now: u64 },
    /// Refund entitled slots whose mint was not observed within `timeout_secs`.
    SweepStaleEntitlements { timeout_secs: u64 },
}

impl Cell {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn checkout_now(&self) -> crate::checkout::CheckoutStatus {
        self.checkout_with_tip(crate::zec_tip::probe())
    }

    pub fn checkout_with_tip(&self, tip: crate::zec_tip::ZecTip) -> crate::checkout::CheckoutStatus {
        let (bound, minted, max) = self.supply("svg");
        let mut s = crate::checkout::CheckoutStatus::evaluate(
            bound,
            minted,
            max,
            crate::checkout::minter_balance_native(),
        );
        s.apply_zec_tip(tip);
        s
    }

    fn refuse_closed(&self) -> Result<(), CellError> {
        if self.checkout_now().checkout_ready {
            Ok(())
        } else {
            Err(CellError::CheckoutClosed)
        }
    }

    pub fn apply(&mut self, turn: Turn) -> Result<Option<MintPacket>, CellError> {
        match turn {
            Turn::DepositSeen { opening } => {
                let price = opening.witness.rail.price_units();
                // Amount is a UI concern. Dust / short pay is not a cell, not a refund.
                if opening.witness.amount < price {
                    return Err(CellError::Underpay);
                }
                let units = crate::checkout::units_paid(opening.witness.amount, price);
                if units == 0 {
                    return Err(CellError::Underpay);
                }
                self.refuse_closed()?;
                let n = nullifier(&opening);
                if self.spent.contains(&n) || self.slots.contains_key(&n) {
                    return Err(CellError::DoubleSpend);
                }
                confirm_rail(opening.witness.rail, &opening.witness.raw)
                    .map_err(|_| CellError::DummyReceipt)?;
                let c = commit_deposit(&opening);
                self.slots.insert(
                    n,
                    Slot {
                        phase: Phase::Deposited,
                        c_deposit: c,
                        n_deposit: n,
                        amount: opening.witness.amount,
                        price,
                        witness_raw: opening.witness.raw.clone(),
                        c_action: None,
                        bind: None,
                        session: None,
                        token_id: None,
                        terp_tx: None,
                        collection: None,
                        rail: Some(opening.witness.rail),
                        payout: RailPayout::None,
                        qty: units.min(u32::MAX as u64) as u32,
                        entitled_at: 0,
                    },
                );
                Ok(None)
            }
            Turn::MintEntitled { n_deposit, action } => {
                self.refuse_closed()?;
                if let Some(col) = self.collections.get(&action.collection) {
                    if col.minted >= col.max && col.max > 0 {
                        return Err(CellError::SoldOut);
                    }
                }
                if self.spent.contains(&n_deposit) {
                    return Err(CellError::DoubleSpend);
                }
                let slot = self.slots.get_mut(&n_deposit).ok_or(CellError::NoDeposit)?;
                if slot.phase != Phase::Deposited {
                    return Err(CellError::NoDeposit);
                }
                if slot.amount < slot.price {
                    return Err(CellError::Underpay);
                }
                let c_action = commit_action(&action);
                let b = bind(&slot.c_deposit, &c_action, &action.session);
                slot.c_action = Some(c_action);
                slot.bind = Some(b);
                slot.session = Some(action.session);
                slot.collection = Some(action.collection.clone());
                slot.phase = Phase::Entitled;
                slot.entitled_at = self.now;
                Ok(None)
            }
            Turn::MintSettled {
                n_deposit,
                token_id,
                terp_tx,
                receipt,
            } => {
                if crate::qc::is_dummy_receipt(&receipt) {
                    return Err(CellError::DummyReceipt);
                }
                let (collection, add) = {
                    let slot = self.slots.get(&n_deposit).ok_or(CellError::NotEntitled)?;
                    if slot.phase != Phase::Entitled {
                        return Err(CellError::NotEntitled);
                    }
                    (slot.collection.clone(), slot.qty.max(1) as u64)
                };
                if let Some(id) = collection {
                    if let Some(col) = self.collections.get_mut(&id) {
                        if col.minted >= col.max {
                            return Err(CellError::SoldOut);
                        }
                        let room = col.max.saturating_sub(col.minted);
                        col.minted = col.minted.saturating_add(add.min(room));
                    }
                }
                let slot = self.slots.get_mut(&n_deposit).ok_or(CellError::NotEntitled)?;
                let c_action = slot.c_action.ok_or(CellError::NotEntitled)?;
                let b = slot.bind.ok_or(CellError::NotEntitled)?;
                let packet = MintPacket {
                    c_deposit: slot.c_deposit,
                    n_deposit: slot.n_deposit,
                    c_action,
                    bind: b,
                    receipt,
                    token_recipient: String::new(),
                    collection: String::new(),
                };
                if slot.payout == RailPayout::Refund {
                    return Err(CellError::PayoutXor);
                }
                slot.payout = RailPayout::Release;
                slot.phase = Phase::Settled;
                slot.token_id = Some(token_id);
                slot.terp_tx = Some(terp_tx);
                self.spent.insert(n_deposit);
                Ok(Some(packet))
            }
            Turn::DepositRefunded { n_deposit } => {
                let (phase, collection) = {
                    let slot = self.slots.get(&n_deposit).ok_or(CellError::NoDeposit)?;
                    (slot.phase, slot.collection.clone())
                };
                let _ = collection;
                let ok = matches!(phase, Phase::Deposited | Phase::Entitled);
                if !ok {
                    return Err(CellError::Terminal);
                }
                let slot = self.slots.get_mut(&n_deposit).ok_or(CellError::NoDeposit)?;
                if slot.payout == RailPayout::Release {
                    return Err(CellError::PayoutXor);
                }
                slot.payout = RailPayout::Refund;
                slot.phase = Phase::Refunded;
                self.spent.insert(n_deposit);
                Ok(None)
            }
            Turn::SetCollection { id, max_supply } => {
                self.collections
                    .entry(id)
                    .or_insert(CollectionSupply {
                        max: max_supply,
                        minted: 0,
                    })
                    .max = max_supply;
                Ok(None)
            }
            Turn::ObserveSupply {
                collection,
                minted,
                max_supply,
                height: _,
                receipt,
            } => {
                if crate::qc::is_dummy_receipt(&receipt) {
                    return Err(CellError::DummyReceipt);
                }
                let e = self.collections.entry(collection).or_insert(CollectionSupply {
                    max: max_supply,
                    minted: 0,
                });
                e.max = max_supply;
                if minted > e.minted {
                    e.minted = minted;
                }
                Ok(None)
            }
            Turn::Tick { now } => {
                self.now = now;
                Ok(None)
            }
            Turn::SweepStaleEntitlements { timeout_secs } => {
                if timeout_secs == 0 {
                    return Ok(None);
                }
                let clock = if self.now > 0 {
                    self.now
                } else {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                };
                if clock == 0 {
                    return Ok(None);
                }
                let stale: Vec<[u8; 32]> = self
                    .slots
                    .iter()
                    .filter(|(_, s)| {
                        s.phase == Phase::Entitled
                            && s.entitled_at > 0
                            && clock.saturating_sub(s.entitled_at) >= timeout_secs
                    })
                    .map(|(n, _)| *n)
                    .collect();
                for n in stale {
                    self.apply(Turn::DepositRefunded { n_deposit: n })?;
                }
                Ok(None)
            }
        }
    }

    /// DREGG query action: when a collection is at max supply, list depositor
    /// nullifiers that must be refunded (entitled, unspent).
    pub fn query_sold_out_refunds(&self, collection: &str) -> SoldOutRefundAnswer {
        let col = self.collections.get(collection);
        let (max, minted) = col.map(|c| (c.max, c.minted)).unwrap_or((0, 0));
        let sold_out = col.map(|c| c.minted >= c.max && c.max > 0).unwrap_or(false);
        let mut refund_nullifiers = Vec::new();
        if sold_out {
            for (n, slot) in &self.slots {
                if slot.collection.as_deref() == Some(collection)
                    && matches!(slot.phase, Phase::Entitled | Phase::Deposited)
                    && !self.spent.contains(n)
                {
                    refund_nullifiers.push(*n);
                }
            }
        }
        SoldOutRefundAnswer {
            collection: collection.into(),
            sold_out,
            minted,
            max_supply: max,
            refund_nullifiers,
        }
    }

    /// Query action + apply: refund every remaining depositor on a sold-out collection (all rails).
    pub fn query_and_refund_sold_out(
        &mut self,
        collection: &str,
    ) -> Result<SoldOutRefundAnswer, CellError> {
        let q = self.query_sold_out_refunds(collection);
        if !q.sold_out {
            return Ok(q);
        }
        for n in q.refund_nullifiers.clone() {
            self.apply(Turn::DepositRefunded { n_deposit: n })?;
        }
        let mut out = self.query_sold_out_refunds(collection);
        out.refund_nullifiers = q.refund_nullifiers;
        Ok(out)
    }

    pub fn phase(&self, n: &[u8; 32]) -> Option<Phase> {
        self.slots.get(n).map(|s| s.phase)
    }

    /// Rail wire for refunds: original `witness_raw` (EVM tx hash / BTC txid).
    pub fn refund_wire(&self, n: &[u8; 32]) -> Option<(crate::commit::Rail, u128, Vec<u8>)> {
        let s = self.slots.get(n)?;
        let rail = s.rail?;
        Some((rail, s.amount, s.witness_raw.clone()))
    }

    pub fn refunded_nullifiers(&self) -> Vec<[u8; 32]> {
        self.slots
            .iter()
            .filter(|(_, s)| s.phase == Phase::Refunded)
            .map(|(n, _)| *n)
            .collect()
    }

    pub fn qty(&self, n: &[u8; 32]) -> Option<u32> {
        self.slots.get(n).map(|s| s.qty)
    }

    pub fn set_nft_qty(&mut self, n: &[u8; 32], qty: u32) {
        if let Some(s) = self.slots.get_mut(n) {
            s.qty = qty.max(1);
        }
    }

    pub fn supply(&self, collection: &str) -> (bool, u64, u64) {
        match self.collections.get(collection) {
            Some(c) => (true, c.minted, c.max),
            None => (false, 0, 0),
        }
    }

    pub fn cell_root(&self) -> [u8; 32] {
        let mut ns: Vec<_> = self.slots.keys().copied().collect();
        ns.sort_unstable();
        let mut h = blake3::Hasher::new();
        for n in ns {
            h.update(&n);
            if let Some(s) = self.slots.get(&n) {
                h.update(&[s.phase as u8]);
            }
        }
        *h.finalize().as_bytes()
    }
}

/// Attested query answer (DREGG query action): refunds due when sold out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoldOutRefundAnswer {
    pub collection: String,
    pub sold_out: bool,
    pub minted: u64,
    pub max_supply: u64,
    pub refund_nullifiers: Vec<[u8; 32]>,
}
