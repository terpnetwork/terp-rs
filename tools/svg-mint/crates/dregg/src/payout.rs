//! Rail payout XOR: RELEASE xor REFUND, never both. Not cell-phase only.

use crate::qc::is_dummy_receipt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RailPayout {
    #[default]
    None,
    Refund,
    Release,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PayoutError {
    #[error("xor: already paid other way")]
    Xor,
    #[error("dummy rail payout")]
    Dummy,
    #[error("already paid")]
    Spent,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RailBooks {
    paid: HashMap<String, RailPayout>,
    /// Opaque rail settlement bytes (vault refund/release tx, btc, ironwood).
    raw: HashMap<String, Vec<u8>>,
}

fn hex32(n: &[u8; 32]) -> String {
    n.iter().map(|b| format!("{b:02x}")).collect()
}

impl RailBooks {
    pub fn apply(
        &mut self,
        n_deposit: [u8; 32],
        kind: RailPayout,
        rail_raw: Vec<u8>,
    ) -> Result<(), PayoutError> {
        if kind == RailPayout::None {
            return Ok(());
        }
        if rail_raw.is_empty() || rail_raw == b"aa" {
            return Err(PayoutError::Dummy);
        }
        if rail_raw.len() == 32 {
            let mut a = [0u8; 32];
            a.copy_from_slice(&rail_raw);
            if is_dummy_receipt(&a) {
                return Err(PayoutError::Dummy);
            }
        }
        let k = hex32(&n_deposit);
        match self.paid.get(&k).copied().unwrap_or(RailPayout::None) {
            RailPayout::None => {
                self.paid.insert(k.clone(), kind);
                self.raw.insert(k, rail_raw);
                Ok(())
            }
            existing if existing == kind => Err(PayoutError::Spent),
            _ => Err(PayoutError::Xor),
        }
    }

    pub fn of(&self, n: &[u8; 32]) -> RailPayout {
        self.paid.get(&hex32(n)).copied().unwrap_or(RailPayout::None)
    }
}
