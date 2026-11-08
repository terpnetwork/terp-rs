//! FROST presignature **inventory**.
//!
//! A slot is one future `sign` (message-independent `(D_i, E_i)`). It is **not**
//! a prevote, precommit, or proposal. Vote type is decided when CometBFT sends
//! the request; earmarks exist only so a precommit does not stall because the
//! last nonce was spent on a prevote.
//!
//! Rolling pool: do not tag slots by height. Consume one slot per successful
//! FROST sign; drop (zeroize via Drop) on abort / signer-set change / leak.

use std::collections::VecDeque;

/// FilePV bookkeeping. Crypto ignores this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresigKind {
    Any,
    Proposal,
    Prevote,
    Precommit,
}

impl PresigKind {
    /// Map FilePV step / lab JSON `step`.
    /// Prevote=1, precommit=2, proposal=3 (matches `vote_step` + SignProposal).
    pub fn from_step(step: i8) -> Self {
        match step {
            1 => Self::Prevote,
            2 => Self::Precommit,
            3 => Self::Proposal,
            _ => Self::Any,
        }
    }
}

/// `need ≈ R_buf · (2 + p_propose) + margin`
#[derive(Clone, Copy, Debug)]
pub struct StockPolicy {
    pub rounds_buf: u16,
    pub may_propose: bool,
    pub margin: u16,
}

impl Default for StockPolicy {
    fn default() -> Self {
        Self {
            rounds_buf: 3,
            may_propose: true,
            margin: 2,
        }
    }
}

impl StockPolicy {
    pub fn target(&self) -> usize {
        let per = 2 + usize::from(self.may_propose);
        (self.rounds_buf as usize) * per + self.margin as usize
    }

    /// Preferred earmark counts: (proposal, prevote, precommit). Remainder is free.
    pub fn earmark_targets(&self) -> (usize, usize, usize) {
        let r = self.rounds_buf as usize;
        let prop = if self.may_propose { r } else { 0 };
        (prop, r, r)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StockSnapshot {
    pub free: usize,
    pub proposal: usize,
    pub prevote: usize,
    pub precommit: usize,
}

impl StockSnapshot {
    pub fn total(&self) -> usize {
        self.free + self.proposal + self.prevote + self.precommit
    }
}

/// Host-side queue. `T` is the FROST slot (nonces + commitments).
pub struct PresigInventory<T> {
    policy: StockPolicy,
    free: VecDeque<T>,
    proposal: VecDeque<T>,
    prevote: VecDeque<T>,
    precommit: VecDeque<T>,
}

impl<T> PresigInventory<T> {
    pub fn new(policy: StockPolicy) -> Self {
        Self {
            policy,
            free: VecDeque::new(),
            proposal: VecDeque::new(),
            prevote: VecDeque::new(),
            precommit: VecDeque::new(),
        }
    }

    pub fn policy(&self) -> StockPolicy {
        self.policy
    }

    pub fn snapshot(&self) -> StockSnapshot {
        StockSnapshot {
            free: self.free.len(),
            proposal: self.proposal.len(),
            prevote: self.prevote.len(),
            precommit: self.precommit.len(),
        }
    }

    pub fn len(&self) -> usize {
        self.snapshot().total()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn deficit(&self) -> usize {
        self.policy.target().saturating_sub(self.len())
    }

    /// Push a freshly minted slot. Earmark by current deficits; extra goes free.
    pub fn push_minted(&mut self, slot: T) {
        let (tp, tv, tc) = self.policy.earmark_targets();
        if self.proposal.len() < tp {
            self.proposal.push_back(slot);
        } else if self.prevote.len() < tv {
            self.prevote.push_back(slot);
        } else if self.precommit.len() < tc {
            self.precommit.push_back(slot);
        } else {
            self.free.push_back(slot);
        }
    }

    /// Take one slot for this FilePV step. Never steal the other vote queue.
    pub fn take(&mut self, kind: PresigKind) -> Option<T> {
        match kind {
            PresigKind::Prevote => self
                .prevote
                .pop_front()
                .or_else(|| self.free.pop_front())
                .or_else(|| self.proposal.pop_front()),
            PresigKind::Precommit => self
                .precommit
                .pop_front()
                .or_else(|| self.free.pop_front())
                .or_else(|| self.proposal.pop_front()),
            PresigKind::Proposal => self.proposal.pop_front().or_else(|| self.free.pop_front()),
            PresigKind::Any => self
                .free
                .pop_front()
                .or_else(|| self.proposal.pop_front())
                .or_else(|| self.prevote.pop_front())
                .or_else(|| self.precommit.pop_front()),
        }
    }

    /// Drop unused slots (height committed / signer set change). Rolling pool
    /// prefers refill over height tags; this is the fail-closed abort path.
    pub fn drain(&mut self) -> usize {
        let n = self.len();
        self.free.clear();
        self.proposal.clear();
        self.prevote.clear();
        self.precommit.clear();
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_formula() {
        let p = StockPolicy {
            rounds_buf: 3,
            may_propose: true,
            margin: 2,
        };
        assert_eq!(p.target(), 3 * 3 + 2);
        let q = StockPolicy {
            rounds_buf: 2,
            may_propose: false,
            margin: 1,
        };
        assert_eq!(q.target(), 2 * 2 + 1);
    }

    #[test]
    fn earmark_does_not_steal_precommit_for_prevote() {
        let mut inv = PresigInventory::new(StockPolicy {
            rounds_buf: 1,
            may_propose: false,
            margin: 0,
        });
        inv.push_minted(1u8);
        inv.push_minted(2u8);
        assert_eq!(inv.snapshot().prevote, 1);
        assert_eq!(inv.snapshot().precommit, 1);
        let a = inv.take(PresigKind::Prevote).unwrap();
        assert_eq!(a, 1);
        assert_eq!(inv.snapshot().precommit, 1);
        assert!(inv.take(PresigKind::Prevote).is_none());
        let b = inv.take(PresigKind::Precommit).unwrap();
        assert_eq!(b, 2);
    }

    #[test]
    fn drain_drops_all() {
        let mut inv = PresigInventory::new(StockPolicy::default());
        inv.push_minted(());
        inv.push_minted(());
        assert!(inv.len() >= 2);
        assert_eq!(inv.drain(), 2);
        assert!(inv.is_empty());
    }
}
