//! Committee primitives shared by rotation and aggregator.
//!
//! * [`derive_committee`]: weighted, deterministic membership + threshold.
//! * [`parse_key`] / [`verify`]: committee keys reuse the existing wire formats
//!   — `FE01` (frost_escrow FROST-Ed25519 group key, verified with the
//!   `ed25519_verify` host import: a FROST group signature is a plain Ed25519
//!   signature) and `TC01` (threshold_committee secp256k1 roster, ≥ t distinct
//!   partials each checked with `secp256k1_verify`). Share `i` of the epoch is
//!   FROST identifier `i` (1-based) / TC01 member index `i - 1`.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Api};

use crate::error::ContractError;

#[cw_serde]
pub struct CommitteePolicy {
    /// Threshold as basis points of total shares (5001..=10000). 6667 = 2/3.
    pub threshold_bps: u16,
    pub min_members: u32,
    pub max_members: u32,
    /// Weight is capped at this many shares per operator.
    pub max_shares_per_member: u16,
    /// An operator counts as active if its last registry report is this recent.
    pub active_window_blocks: u64,
    /// A pending epoch not activated within this many blocks expires.
    pub dkg_timeout_blocks: u64,
    /// Registry capability tag an operator must advertise to join.
    pub capability: String,
}

impl Default for CommitteePolicy {
    fn default() -> Self {
        Self {
            threshold_bps: 6667,
            min_members: 3,
            max_members: 32,
            max_shares_per_member: 4,
            active_window_blocks: 100,
            dkg_timeout_blocks: 200,
            capability: "preconf".into(),
        }
    }
}

impl CommitteePolicy {
    pub fn validate(&self) -> Result<(), ContractError> {
        let bad = |m: &str| Err(ContractError::BadPolicy(m.into()));
        if !(5001..=10000).contains(&self.threshold_bps) {
            return bad("threshold_bps must be a strict majority (5001..=10000)");
        }
        if self.min_members < 2 || self.max_members < self.min_members || self.max_members > 64 {
            return bad("need 2 <= min_members <= max_members <= 64");
        }
        if self.max_shares_per_member == 0 || self.max_shares_per_member > 16 {
            return bad("max_shares_per_member must be 1..=16");
        }
        if self.active_window_blocks == 0 || self.dkg_timeout_blocks == 0 {
            return bad("windows must be > 0");
        }
        if self.capability.trim().is_empty() {
            return bad("capability must not be empty");
        }
        Ok(())
    }
}

#[cw_serde]
pub struct Member {
    pub operator: Addr,
    pub weight: u64,
    pub shares: u16,
    /// Shares are ids `first_share .. first_share + shares` (1-based).
    pub first_share: u16,
}

impl Member {
    pub fn share_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.first_share..self.first_share + self.shares
    }
}

#[cw_serde]
pub struct Committee {
    pub members: Vec<Member>,
    pub total_shares: u16,
    pub threshold: u16,
}

/// Derive the committee from `(operator, weight)` candidates.
///
/// Deterministic: the heaviest `max_members` (ties by address) are kept, then
/// ordered by address for share assignment. `threshold =
/// ceil(total * bps / 10000)`; it is a strict weighted majority by
/// construction, and no member may hold `>= threshold` shares.
pub fn derive_committee(
    mut candidates: Vec<(Addr, u64)>,
    p: &CommitteePolicy,
) -> Result<Committee, ContractError> {
    candidates.retain(|(_, w)| *w > 0);
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    candidates.dedup_by(|a, b| a.0 == b.0);
    candidates.truncate(p.max_members as usize);
    if (candidates.len() as u32) < p.min_members {
        return Err(ContractError::TooFewMembers {
            have: candidates.len() as u32,
            need: p.min_members,
        });
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    let mut members = Vec::with_capacity(candidates.len());
    let mut next: u16 = 1;
    for (operator, weight) in candidates {
        let shares = weight.min(p.max_shares_per_member as u64) as u16;
        members.push(Member {
            operator,
            weight,
            shares,
            first_share: next,
        });
        next += shares;
    }
    let total = next - 1;
    let threshold = ((total as u32 * p.threshold_bps as u32).div_ceil(10_000)).max(2) as u16;
    if let Some(m) = members.iter().find(|m| m.shares >= threshold) {
        return Err(ContractError::Concentrated(m.operator.to_string()));
    }
    Ok(Committee {
        members,
        total_shares: total,
        threshold,
    })
}

#[cw_serde]
pub enum KeyScheme {
    /// `FE01`: FROST-Ed25519 group verifying key (frost_escrow).
    FrostEd25519,
    /// `TC01`: secp256k1 t-of-n roster (threshold_committee).
    ThresholdSecp256k1,
}

/// Parse a committee key → (scheme, t, n).
pub fn parse_key(key: &[u8]) -> Result<(KeyScheme, u16, u16), ContractError> {
    if frost_escrow::is_frost_public_key(key) {
        let (t, n, vk) =
            frost_escrow::decode_frost_public(key).map_err(|_| ContractError::BadKey {})?;
        if vk.len() != 32 {
            return Err(ContractError::BadKey {});
        }
        Ok((KeyScheme::FrostEd25519, t, n))
    } else if threshold_committee::is_committee_public_key(key) {
        let r = threshold_committee::CommitteeRoster::decode_public(key)
            .map_err(|_| ContractError::BadKey {})?;
        Ok((KeyScheme::ThresholdSecp256k1, r.threshold, r.n()))
    } else {
        Err(ContractError::BadKey {})
    }
}

/// Verify a committee signature over a 32-byte digest using host crypto.
pub fn verify(api: &dyn Api, key: &[u8], digest: &[u8; 32], sig: &[u8]) -> bool {
    if frost_escrow::is_frost_public_key(key) {
        let Ok((_, _, vk)) = frost_escrow::decode_frost_public(key) else {
            return false;
        };
        sig.len() == 64 && api.ed25519_verify(digest, sig, &vk).unwrap_or(false)
    } else if threshold_committee::is_committee_public_key(key) {
        let (Ok(roster), Ok(combined)) = (
            threshold_committee::CommitteeRoster::decode_public(key),
            threshold_committee::CombinedSignature::decode(sig),
        ) else {
            return false;
        };
        let mut seen = std::collections::BTreeSet::new();
        for p in &combined.partials {
            let Some(pk) = roster.member_pks_sec1.get(p.member_index as usize) else {
                return false;
            };
            if !seen.insert(p.member_index)
                || !api
                    .secp256k1_verify(digest, &p.signature, pk)
                    .unwrap_or(false)
            {
                return false;
            }
        }
        seen.len() >= roster.threshold as usize
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Addr {
        Addr::unchecked(s)
    }

    #[test]
    fn weighted_supermajority() {
        let p = CommitteePolicy::default();
        let c = derive_committee(
            vec![
                (a("c"), 1),
                (a("a"), 1),
                (a("b"), 2),
                (a("e"), 1),
                (a("z"), 0),
            ],
            &p,
        )
        .unwrap();
        // sorted by address, contiguous 1-based shares
        let ids: Vec<(String, u16, u16)> = c
            .members
            .iter()
            .map(|m| (m.operator.to_string(), m.first_share, m.shares))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("a".into(), 1, 1),
                ("b".into(), 2, 2),
                ("c".into(), 4, 1),
                ("e".into(), 5, 1)
            ]
        );
        assert_eq!((c.total_shares, c.threshold), (5, 4));
        // weight caps at max_shares_per_member
        let c = derive_committee(vec![(a("a"), 50), (a("b"), 3), (a("c"), 3)], &p).unwrap();
        assert_eq!(
            (c.members[0].shares, c.total_shares, c.threshold),
            (4, 10, 7)
        );
    }

    #[test]
    fn size_and_concentration_limits() {
        let p = CommitteePolicy::default();
        assert!(matches!(
            derive_committee(vec![(a("a"), 1), (a("b"), 1)], &p),
            Err(ContractError::TooFewMembers { have: 2, need: 3 })
        ));
        // 60%: 4 + 1 + 1 = 6 shares, t = 4 → `a` alone would sign
        let mut q = p.clone();
        q.threshold_bps = 6000;
        assert!(matches!(
            derive_committee(vec![(a("a"), 4), (a("b"), 1), (a("c"), 1)], &q),
            Err(ContractError::Concentrated(_))
        ));
        let mut q = p.clone();
        q.max_members = 3;
        let c =
            derive_committee(vec![(a("a"), 1), (a("b"), 3), (a("c"), 2), (a("d"), 2)], &q).unwrap();
        let ops: Vec<_> = c.members.iter().map(|m| m.operator.as_str()).collect();
        assert_eq!(ops, vec!["b", "c", "d"], "heaviest kept");
        q.threshold_bps = 5000;
        assert!(q.validate().is_err());
    }
}
