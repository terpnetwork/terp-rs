//! Draw entropy for the raffle contract.
//!
//! Atlas used a Nois proxy and, later, a Drand verifier contract
//! (`packages/randomness` plus `contracts/raffles/src/randomness`). Neither
//! Nois nor a Drand client is in this workspace. Draws go through
//! [`RandomnessSource`]:
//!
//! * `Seed` — a configured provider (or anyone, when the provider is unset)
//!   submits a 32-byte seed after the raffle closes. Tests pass the seed
//!   directly. The winner index is `sha256(seed || raffle_id || salt)` reduced
//!   modulo the remaining ticket count, without replacement.
//! * `Callback` — `RequestDraw` calls [`VerifierExecuteMsg::Provide`] on an
//!   external verifier. The successful reply must include wasm attributes
//!   `raffle_id`, `round`, `randomness` (base64 of 32 bytes), and `owner`.
//!   The verifier is not shipped here.
//!
//! See `contracts/nfts/RANDOMNESS.md`.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Api, StdResult};
use sha2::{Digest, Sha256};

#[cw_serde]
pub enum RandomnessSourceMsg {
    /// `provider` is the only account allowed to submit a seed.
    /// `None` accepts a seed from any account. Use that for tests.
    Seed { provider: Option<String> },
    /// External verifier. See [`VerifierExecuteMsg`].
    Callback { verifier: String },
}

#[cw_serde]
pub enum RandomnessSource {
    Seed { provider: Option<Addr> },
    Callback { verifier: Addr },
}

impl RandomnessSourceMsg {
    pub fn validate(self, api: &dyn Api) -> StdResult<RandomnessSource> {
        Ok(match self {
            Self::Seed { provider } => RandomnessSource::Seed {
                provider: match provider {
                    Some(provider) => Some(api.addr_validate(&provider)?),
                    None => None,
                },
            },
            Self::Callback { verifier } => RandomnessSource::Callback {
                verifier: api.addr_validate(&verifier)?,
            },
        })
    }
}

/// Message the raffle sends to a callback verifier.
#[cw_serde]
pub enum VerifierExecuteMsg {
    Provide { raffle_id: u64, owner: String },
}

/// Index of one ticket in `0..ticket_count`, mixed by `salt`.
pub fn select_ticket(seed: &[u8; 32], raffle_id: u64, salt: u32, ticket_count: u32) -> u32 {
    assert!(ticket_count > 0, "ticket count");
    let mut hasher = Sha256::new();
    hasher.update(seed);
    hasher.update(raffle_id.to_be_bytes());
    hasher.update(salt.to_be_bytes());
    let digest = hasher.finalize();
    let mut wide = [0u8; 16];
    wide.copy_from_slice(&digest[..16]);
    let n = u128::from_be_bytes(wide);
    (n % u128::from(ticket_count)) as u32
}

/// `winners` distinct ticket indexes. Capped at `ticket_count`.
pub fn select_tickets(seed: &[u8; 32], raffle_id: u64, ticket_count: u32, winners: u32) -> Vec<u32> {
    let picks = winners.min(ticket_count) as usize;
    let mut pool: Vec<u32> = (0..ticket_count).collect();
    let mut out = Vec::with_capacity(picks);
    for salt in 0..picks {
        let idx = select_ticket(seed, raffle_id, salt as u32, pool.len() as u32) as usize;
        out.push(pool.swap_remove(idx));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_tickets_is_stable_and_unique() {
        let seed = [9u8; 32];
        let once = select_tickets(&seed, 4, 10, 4);
        let twice = select_tickets(&seed, 4, 10, 4);
        assert_eq!(once, twice);
        assert_eq!(once.len(), 4);
        let mut sorted = once.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 4);
        assert!(once.iter().all(|i| *i < 10));
        assert_ne!(select_tickets(&seed, 4, 10, 4), select_tickets(&[3u8; 32], 4, 10, 4));
    }
}
