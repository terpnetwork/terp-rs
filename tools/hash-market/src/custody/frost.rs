//! FROST coordinator for hash-market sidecar. Shares stay in this process's
//! shard objects; Noise/privval never sees KeyPackages.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use frost_ed25519 as frost;
use kms_shard::{ConsensusScheme, FrostEd25519};
use rand_core::OsRng;

use super::consensus::ConsensusSigner;
use super::presig::{PresigInventory, PresigKind, StockPolicy, StockSnapshot};

/// One FROST participant living in this sidecar (Plane S in-process).
pub struct LocalShard {
    id: u16,
    share: frost::keys::KeyPackage,
}

/// One unused FROST nonce pair set. Message is attached only at `sign_share`.
struct PresigSlot {
    commits: kms_shard::CommitmentMap,
    nonces: Vec<(u16, frost::keys::KeyPackage, frost::round1::SigningNonces)>,
}

impl LocalShard {
    pub fn load(id: u16, path: &Path) -> Result<Self> {
        let raw = std::fs::read(path).with_context(|| format!("frost share {}", path.display()))?;
        let share =
            frost::keys::KeyPackage::deserialize(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self { id, share })
    }
}

/// t-of-n coordinator. Colocated shards for a single validator sidecar;
/// remote `ShardPeer` can replace `LocalShard` later without touching Noise.
pub struct FrostCoordinator {
    shards: Vec<LocalShard>,
    group: frost::keys::PublicKeyPackage,
    pk: Vec<u8>,
    t: u16,
    mu: Mutex<PresigInventory<PresigSlot>>,
}

impl FrostCoordinator {
    pub fn load(t: u16, group_path: &Path, share_paths: &[(u16, &Path)]) -> Result<Self> {
        anyhow::ensure!(t >= 1, "frost t >= 1");
        anyhow::ensure!(share_paths.len() >= t as usize, "need at least t shares");
        let raw = std::fs::read(group_path).context("frost group")?;
        let group =
            frost::keys::PublicKeyPackage::deserialize(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
        let pk = FrostEd25519::group_pk_bytes(&group).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut shards = Vec::new();
        for (id, p) in share_paths {
            shards.push(LocalShard::load(*id, p)?);
        }
        let c = Self {
            shards,
            group,
            pk: pk.to_vec(),
            t,
            mu: Mutex::new(PresigInventory::new(StockPolicy::default())),
        };
        c.refill()?;
        Ok(c)
    }

    pub fn from_dkg(n: u16, t: u16) -> Result<Self> {
        let mut rng = OsRng;
        let (shares, group) = kms_shard::dkg_all::<FrostEd25519>(n, t, &mut rng)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let pk = FrostEd25519::group_pk_bytes(&group).map_err(|e| anyhow::anyhow!("{e}"))?;
        let shards = shares
            .into_iter()
            .map(|(id, share)| LocalShard { id, share })
            .collect();
        let c = Self {
            shards,
            group,
            pk: pk.to_vec(),
            t,
            mu: Mutex::new(PresigInventory::new(StockPolicy::default())),
        };
        c.refill()?;
        Ok(c)
    }

    pub fn with_policy(self, policy: StockPolicy) -> Result<Self> {
        {
            let mut g = self.mu.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            let drained = g.drain();
            let _ = drained;
            *g = PresigInventory::new(policy);
        }
        self.refill()?;
        Ok(self)
    }

    pub fn pubkey_bytes(&self) -> &[u8] {
        &self.pk
    }

    pub fn stock(&self) -> Result<StockSnapshot> {
        let g = self.mu.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(g.snapshot())
    }

    /// Mint unused slots up to policy target. Call from idle / NewHeight, not
    /// on the SignVote deadline.
    pub fn refill(&self) -> Result<usize> {
        let mut minted = 0usize;
        loop {
            let need = {
                let g = self.mu.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
                g.deficit()
            };
            if need == 0 {
                break;
            }
            let slot = self.mint_slot()?;
            let mut g = self.mu.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            g.push_minted(slot);
            minted += 1;
        }
        Ok(minted)
    }

    /// Height committed or signer set changed: drop leftover nonces (fail closed).
    pub fn abort_pool(&self) -> Result<usize> {
        let mut g = self.mu.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(g.drain())
    }

    fn mint_slot(&self) -> Result<PresigSlot> {
        let mut rng = OsRng;
        let use_n = self.t as usize;
        let use_shards = &self.shards[..use_n];
        let mut commit_map = BTreeMap::new();
        let mut nonces = Vec::new();
        for s in use_shards {
            let (n, c) =
                FrostEd25519::commit(&s.share, &mut rng).map_err(|e| anyhow::anyhow!("{e}"))?;
            let ident = kms_shard::identifier(s.id).map_err(|e| anyhow::anyhow!("{e}"))?;
            commit_map.insert(ident, c);
            nonces.push((s.id, s.share.clone(), n));
        }
        Ok(PresigSlot {
            commits: commit_map,
            nonces,
        })
    }

    fn take_or_mint(&self, kind: PresigKind) -> Result<PresigSlot> {
        {
            let mut g = self.mu.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Some(s) = g.take(kind) {
                return Ok(s);
            }
        }
        self.mint_slot()
    }

    fn sign_with_slot(&self, slot: PresigSlot, msg: &[u8]) -> Result<[u8; 64]> {
        let PresigSlot { commits, nonces } = slot;
        let mut partials = BTreeMap::new();
        for (id, share, n) in nonces {
            let p = FrostEd25519::sign_share(&share, n, &commits, msg)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            partials.insert(id, p);
        }
        let sig = FrostEd25519::combine(&commits, &partials, msg, &self.group)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        FrostEd25519::signature_bytes(&sig).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

#[async_trait]
impl ConsensusSigner for FrostCoordinator {
    async fn sign_consensus(&self, msg: &[u8]) -> Result<[u8; 64]> {
        self.sign_consensus_for(msg, 0).await
    }

    async fn sign_consensus_for(&self, msg: &[u8], step: i8) -> Result<[u8; 64]> {
        let kind = PresigKind::from_step(step);
        let slot = self.take_or_mint(kind)?;
        // Slot is consumed even if combine fails — nonce is dead.
        self.sign_with_slot(slot, msg)
    }

    fn group_pubkey(&self) -> &[u8] {
        &self.pk
    }

    fn label(&self) -> &str {
        "frost-ed25519-v1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custody::consensus::ConsensusSigner;

    #[tokio::test]
    async fn colocated_2_of_3_signs_ed25519() {
        let c = FrostCoordinator::from_dkg(3, 2).unwrap();
        let msg = b"sidecar-vote";
        let sig = c.sign_consensus(msg).await.unwrap();
        let pk: [u8; 32] = c.group_pubkey().try_into().unwrap();
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk).unwrap();
        vk.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(&sig))
            .unwrap();
    }

    #[tokio::test]
    async fn inventory_earmark_and_consume() {
        let c = FrostCoordinator::from_dkg(3, 2)
            .unwrap()
            .with_policy(StockPolicy {
                rounds_buf: 2,
                may_propose: true,
                margin: 0,
            })
            .unwrap();
        let before = c.stock().unwrap();
        assert_eq!(before.total(), 2 * 3);
        assert_eq!(before.prevote, 2);
        assert_eq!(before.precommit, 2);
        assert_eq!(before.proposal, 2);

        let pv = c.sign_consensus_for(b"prevote-m", 1).await.unwrap();
        let mid = c.stock().unwrap();
        assert_eq!(mid.prevote, 1);
        assert_eq!(mid.precommit, 2);

        let pc = c.sign_consensus_for(b"precommit-m", 2).await.unwrap();
        assert_ne!(&pv, &pc);
        let after = c.stock().unwrap();
        assert_eq!(after.precommit, 1);

        let pk: [u8; 32] = c.group_pubkey().try_into().unwrap();
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk).unwrap();
        vk.verify_strict(b"prevote-m", &ed25519_dalek::Signature::from_bytes(&pv))
            .unwrap();
        vk.verify_strict(b"precommit-m", &ed25519_dalek::Signature::from_bytes(&pc))
            .unwrap();
    }

    #[tokio::test]
    async fn refill_restores_after_consume() {
        let c = FrostCoordinator::from_dkg(3, 2)
            .unwrap()
            .with_policy(StockPolicy {
                rounds_buf: 1,
                may_propose: false,
                margin: 0,
            })
            .unwrap();
        let n0 = c.stock().unwrap().total();
        c.sign_consensus_for(b"a", 1).await.unwrap();
        assert!(c.stock().unwrap().total() < n0);
        c.refill().unwrap();
        assert_eq!(c.stock().unwrap().total(), n0);
    }

    #[tokio::test]
    async fn abort_pool_zeroizes_unused() {
        let c = FrostCoordinator::from_dkg(3, 2).unwrap();
        assert!(c.stock().unwrap().total() > 0);
        c.abort_pool().unwrap();
        assert_eq!(c.stock().unwrap().total(), 0);
        let sig = c.sign_consensus(b"after-abort").await.unwrap();
        let pk: [u8; 32] = c.group_pubkey().try_into().unwrap();
        ed25519_dalek::VerifyingKey::from_bytes(&pk)
            .unwrap()
            .verify_strict(b"after-abort", &ed25519_dalek::Signature::from_bytes(&sig))
            .unwrap();
    }
}
