//! Restored from terp-rs 491ae651 (replaces the "minimal restore" stub whose verify accepted any
//! non-empty input). Only change: OS-entropy key generation for k256 0.14.
//!
//! In-house **t-of-n committee threshold signing** for hashmerchant VE and escrow release.
//!
//! Design inspiration (not runtime deps): NEAR Chain Signatures / near-mpc, Sui Hashi,
//! Union MPC, Penumbra summonerd ceremonies.
//!
//! ## Lab model
//!
//! Each committee member holds their own secp256k1 key (lab DKG = independent keygen +
//! roster publish). A valid threshold signature is a set of **≥ t** distinct member ECDSA
//! signatures over the **same domain-separated digest**. Below threshold fails closed.
//!
//! This is production-shaped **policy** (committee, not single local key / not ops funder)
//! with lab-simple crypto. A future FROST/BLS upgrade can keep the same digests and gates.
//!
//! ## Domain separation
//!
//! ```text
//! digest = SHA256(domain_label ‖ payload)
//! ```
//!
//! Standard labels:
//! - [`DOMAIN_VE_V1`] — vote-extension style objects
//! - [`DOMAIN_ESCROW_RELEASE_V1`] — ZEC escrow release authorization
//! - [`DOMAIN_BOUNDS_INTENT_V1`] — oracle bounds / intent-shaped payloads (still bound_only)

use std::collections::BTreeSet;

use k256::ecdsa::{
    Signature, SigningKey, VerifyingKey,
    signature::hazmat::{PrehashSigner, PrehashVerifier},
};
#[cfg(feature = "lab")]
use rand_core06::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "lab")]
/// Fresh secp256k1 key from OS entropy (k256 0.14 no longer takes a rand_core 0.6 RNG).
fn random_signing_key() -> SigningKey {
    loop {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        if let Ok(sk) = SigningKey::from_slice(&bytes) {
            return sk;
        }
    }
}

/// VE / committee object domain (hashmerchant-aligned).
pub const DOMAIN_VE_V1: &[u8] = b"terp/hashmerchant/ve/v1";

/// Escrow release authorization (Option D burn → Zcash release).
pub const DOMAIN_ESCROW_RELEASE_V1: &[u8] = b"terp/threshold/escrow-release/v1";

/// Bounds / intent-shaped payload (oracle data — never mint authority).
pub const DOMAIN_BOUNDS_INTENT_V1: &[u8] = b"terp/threshold/bounds-intent/v1";

/// Magic prefix for encoded committee public material (Custody / VE wire).
pub const COMMITTEE_PK_MAGIC: &[u8; 4] = b"TC01";

/// Wire version for combined signature blobs.
pub const COMBINED_SIG_VERSION: u8 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThresholdError {
    #[error("invalid threshold parameters: t={t} n={n}")]
    InvalidParams { t: u16, n: u16 },
    #[error("member index out of range")]
    BadMemberIndex,
    #[error("duplicate member partials")]
    DuplicateMember,
    #[error("below threshold: have {have}, need {need}")]
    BelowThreshold { have: u16, need: u16 },
    #[error("partial signature invalid for member {0}")]
    InvalidPartial(u16),
    #[error("combined signature malformed")]
    MalformedCombined,
    #[error("committee public key malformed")]
    MalformedCommitteePk,
    #[error("verify failed")]
    VerifyFailed,
    #[error("crypto: {0}")]
    Crypto(String),
}

pub type Result<T> = std::result::Result<T, ThresholdError>;

/// Domain-separated digest: SHA256(domain ‖ payload).
pub fn domain_digest(domain: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(domain);
    h.update(payload);
    let out = h.finalize();
    let mut d = [0u8; 32];
    d.copy_from_slice(&out);
    d
}

/// Escrow-release payload digest inputs (canonical byte layout).
pub fn escrow_release_payload(
    burn_nullifier: &[u8; 32],
    dest_commitment: &[u8; 32],
    amount_zat: u64,
    asset_id: &[u8; 32],
) -> Vec<u8> {
    let mut v = Vec::with_capacity(32 + 32 + 8 + 32);
    v.extend_from_slice(burn_nullifier);
    v.extend_from_slice(dest_commitment);
    v.extend_from_slice(&amount_zat.to_le_bytes());
    v.extend_from_slice(asset_id);
    v
}

pub fn escrow_release_digest(
    burn_nullifier: &[u8; 32],
    dest_commitment: &[u8; 32],
    amount_zat: u64,
    asset_id: &[u8; 32],
) -> [u8; 32] {
    domain_digest(
        DOMAIN_ESCROW_RELEASE_V1,
        &escrow_release_payload(burn_nullifier, dest_commitment, amount_zat, asset_id),
    )
}

/// One committee member's secret share (lab: full secp256k1 signing key).
#[derive(Clone)]
pub struct MemberShare {
    pub index: u16,
    signing_key: SigningKey,
}

impl MemberShare {
    pub fn from_signing_key(index: u16, signing_key: SigningKey) -> Self {
        Self { index, signing_key }
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        *self.signing_key.verifying_key()
    }

    pub fn public_key_sec1(&self) -> Vec<u8> {
        self.signing_key.verifying_key().to_sec1_bytes().to_vec()
    }

    /// Partial-sign a pre-hashed 32-byte digest (caller applies domain separation).
    pub fn partial_sign_digest(&self, digest32: &[u8; 32]) -> Result<PartialSignature> {
        let sig: Signature = self
            .signing_key
            .sign_prehash(digest32)
            .map_err(|e| ThresholdError::Crypto(e.to_string()))?;
        Ok(PartialSignature {
            member_index: self.index,
            signature: sig.to_bytes().to_vec(),
        })
    }
}

/// Published roster: threshold + ordered member verifying keys (SEC1 compressed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitteeRoster {
    pub threshold: u16,
    /// Index i corresponds to member index i (0..n-1).
    pub member_pks_sec1: Vec<Vec<u8>>,
}

impl CommitteeRoster {
    pub fn n(&self) -> u16 {
        self.member_pks_sec1.len() as u16
    }

    pub fn validate_params(&self) -> Result<()> {
        let n = self.n();
        let t = self.threshold;
        if t == 0 || n == 0 || t > n {
            return Err(ThresholdError::InvalidParams { t, n });
        }
        for pk in &self.member_pks_sec1 {
            VerifyingKey::from_sec1_bytes(pk)
                .map_err(|e| ThresholdError::Crypto(format!("member pk: {e}")))?;
        }
        Ok(())
    }

    /// Encode committee public material for Custody::public_key / wire.
    ///
    /// Layout: `TC01` ‖ t_le16 ‖ n_le16 ‖ concat(SEC1 pks)
    pub fn encode_public(&self) -> Result<Vec<u8>> {
        self.validate_params()?;
        let n = self.n();
        let mut out = Vec::with_capacity(
            4 + 2 + 2 + self.member_pks_sec1.iter().map(|p| p.len()).sum::<usize>(),
        );
        out.extend_from_slice(COMMITTEE_PK_MAGIC);
        out.extend_from_slice(&self.threshold.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        for pk in &self.member_pks_sec1 {
            out.extend_from_slice(pk);
        }
        Ok(out)
    }

    pub fn decode_public(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 || &bytes[0..4] != COMMITTEE_PK_MAGIC {
            return Err(ThresholdError::MalformedCommitteePk);
        }
        let t = u16::from_le_bytes([bytes[4], bytes[5]]);
        let n = u16::from_le_bytes([bytes[6], bytes[7]]);
        let mut rest = &bytes[8..];
        let mut member_pks_sec1 = Vec::with_capacity(n as usize);
        for _ in 0..n {
            // Compressed SEC1 = 33 bytes
            if rest.len() < 33 {
                return Err(ThresholdError::MalformedCommitteePk);
            }
            member_pks_sec1.push(rest[..33].to_vec());
            rest = &rest[33..];
        }
        if !rest.is_empty() {
            return Err(ThresholdError::MalformedCommitteePk);
        }
        let roster = Self {
            threshold: t,
            member_pks_sec1,
        };
        roster.validate_params()?;
        Ok(roster)
    }

    pub fn verifying_key_at(&self, index: u16) -> Result<VerifyingKey> {
        let pk = self
            .member_pks_sec1
            .get(index as usize)
            .ok_or(ThresholdError::BadMemberIndex)?;
        VerifyingKey::from_sec1_bytes(pk).map_err(|e| ThresholdError::Crypto(e.to_string()))
    }
}

/// Lab DKG: generate n independent keys; threshold t of n must sign.
#[derive(Clone)]
pub struct LabCommittee {
    pub roster: CommitteeRoster,
    pub shares: Vec<MemberShare>,
    /// `roster.encode_public()` cached (`TC01` wire). Kept for callers written
    /// against the interim stub, which exposed this field.
    pub encoded_public: Vec<u8>,
}

impl LabCommittee {
    /// Generate a fresh lab committee (OsRng).
    #[cfg(feature = "lab")]
    pub fn generate(threshold: u16, n: u16) -> Result<Self> {
        if threshold == 0 || n == 0 || threshold > n {
            return Err(ThresholdError::InvalidParams { t: threshold, n });
        }
        let mut shares = Vec::with_capacity(n as usize);
        let mut member_pks_sec1 = Vec::with_capacity(n as usize);
        for i in 0..n {
            let sk = random_signing_key();
            let share = MemberShare::from_signing_key(i, sk);
            member_pks_sec1.push(share.public_key_sec1());
            shares.push(share);
        }
        let roster = CommitteeRoster {
            threshold,
            member_pks_sec1,
        };
        roster.validate_params()?;
        let encoded_public = roster.encode_public()?;
        Ok(Self {
            roster,
            shares,
            encoded_public,
        })
    }

    pub fn share(&self, index: u16) -> Result<&MemberShare> {
        self.shares
            .get(index as usize)
            .ok_or(ThresholdError::BadMemberIndex)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartialSignature {
    pub member_index: u16,
    /// 64-byte compact ECDSA signature.
    pub signature: Vec<u8>,
}

/// Combined threshold signature (encoded for wire / receipts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombinedSignature {
    pub partials: Vec<PartialSignature>,
}

impl CombinedSignature {
    /// Layout: `ver_u8` ‖ `count_u8` ‖ repeated (`index_u16_le` ‖ `sig_64`)
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + self.partials.len() * (2 + 64));
        out.push(COMBINED_SIG_VERSION);
        out.push(self.partials.len() as u8);
        for p in &self.partials {
            out.extend_from_slice(&p.member_index.to_le_bytes());
            // pad/truncate to 64
            let mut sig = p.signature.clone();
            sig.resize(64, 0);
            out.extend_from_slice(&sig[..64]);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 || bytes[0] != COMBINED_SIG_VERSION {
            return Err(ThresholdError::MalformedCombined);
        }
        let count = bytes[1] as usize;
        let mut rest = &bytes[2..];
        let mut partials = Vec::with_capacity(count);
        for _ in 0..count {
            if rest.len() < 2 + 64 {
                return Err(ThresholdError::MalformedCombined);
            }
            let index = u16::from_le_bytes([rest[0], rest[1]]);
            let signature = rest[2..2 + 64].to_vec();
            rest = &rest[2 + 64..];
            partials.push(PartialSignature {
                member_index: index,
                signature,
            });
        }
        if !rest.is_empty() {
            return Err(ThresholdError::MalformedCombined);
        }
        Ok(Self { partials })
    }
}

/// Combine partials: require ≥ t distinct valid signatures under roster.
pub fn combine(
    roster: &CommitteeRoster,
    digest32: &[u8; 32],
    partials: &[PartialSignature],
) -> Result<CombinedSignature> {
    roster.validate_params()?;
    let mut seen = BTreeSet::new();
    let mut accepted = Vec::new();

    for p in partials {
        if p.member_index >= roster.n() {
            return Err(ThresholdError::BadMemberIndex);
        }
        if !seen.insert(p.member_index) {
            return Err(ThresholdError::DuplicateMember);
        }
        let vk = roster.verifying_key_at(p.member_index)?;
        let sig = Signature::from_slice(&p.signature)
            .map_err(|_| ThresholdError::InvalidPartial(p.member_index))?;
        vk.verify_prehash(digest32, &sig)
            .map_err(|_| ThresholdError::InvalidPartial(p.member_index))?;
        accepted.push(p.clone());
    }

    let have = accepted.len() as u16;
    if have < roster.threshold {
        return Err(ThresholdError::BelowThreshold {
            have,
            need: roster.threshold,
        });
    }

    // Deterministic order by member index
    accepted.sort_by_key(|p| p.member_index);
    Ok(CombinedSignature { partials: accepted })
}

/// Verify combined signature against roster + digest.
pub fn verify(
    roster: &CommitteeRoster,
    digest32: &[u8; 32],
    combined: &CombinedSignature,
) -> Result<()> {
    roster.validate_params()?;
    let mut seen = BTreeSet::new();
    let mut ok = 0u16;
    for p in &combined.partials {
        if p.member_index >= roster.n() {
            return Err(ThresholdError::BadMemberIndex);
        }
        if !seen.insert(p.member_index) {
            return Err(ThresholdError::DuplicateMember);
        }
        let vk = roster.verifying_key_at(p.member_index)?;
        let sig = Signature::from_slice(&p.signature)
            .map_err(|_| ThresholdError::InvalidPartial(p.member_index))?;
        if vk.verify_prehash(digest32, &sig).is_ok() {
            ok += 1;
        } else {
            return Err(ThresholdError::InvalidPartial(p.member_index));
        }
    }
    if ok < roster.threshold {
        return Err(ThresholdError::BelowThreshold {
            have: ok,
            need: roster.threshold,
        });
    }
    Ok(())
}

/// Convenience: first `threshold` shares of a lab committee sign + combine.
pub fn lab_sign_with_threshold(
    committee: &LabCommittee,
    digest32: &[u8; 32],
) -> Result<CombinedSignature> {
    let t = committee.roster.threshold as usize;
    let mut partials = Vec::with_capacity(t);
    for i in 0..t {
        let share = committee.share(i as u16)?;
        partials.push(share.partial_sign_digest(digest32)?);
    }
    combine(&committee.roster, digest32, &partials)
}

/// Verify using encoded committee public key + encoded combined sig (Custody wire).
pub fn verify_encoded(committee_pk: &[u8], digest32: &[u8; 32], combined_sig: &[u8]) -> Result<()> {
    let roster = CommitteeRoster::decode_public(committee_pk)?;
    let combined = CombinedSignature::decode(combined_sig)?;
    verify(&roster, digest32, &combined)
}

pub fn is_committee_public_key(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == COMMITTEE_PK_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_dkg_happy_threshold_sign_verify() {
        let c = LabCommittee::generate(2, 3).unwrap();
        let digest = domain_digest(DOMAIN_ESCROW_RELEASE_V1, b"payload-demo");
        let combined = lab_sign_with_threshold(&c, &digest).unwrap();
        verify(&c.roster, &digest, &combined).unwrap();
        // Round-trip wire encoding
        let pk = c.roster.encode_public().unwrap();
        let sig = combined.encode();
        verify_encoded(&pk, &digest, &sig).unwrap();
        assert!(is_committee_public_key(&pk));
    }

    #[test]
    fn below_threshold_fails() {
        let c = LabCommittee::generate(2, 3).unwrap();
        let digest = domain_digest(DOMAIN_VE_V1, b"ve-body");
        let only_one = c.share(0).unwrap().partial_sign_digest(&digest).unwrap();
        let err = combine(&c.roster, &digest, &[only_one]).unwrap_err();
        assert_eq!(err, ThresholdError::BelowThreshold { have: 1, need: 2 });
    }

    #[test]
    fn invalid_partial_rejected() {
        let c = LabCommittee::generate(2, 3).unwrap();
        let digest = domain_digest(DOMAIN_VE_V1, b"a");
        let mut p0 = c.share(0).unwrap().partial_sign_digest(&digest).unwrap();
        let p1 = c.share(1).unwrap().partial_sign_digest(&digest).unwrap();
        // Corrupt p0 signature bytes
        p0.signature[0] ^= 0xff;
        let err = combine(&c.roster, &digest, &[p0, p1]).unwrap_err();
        assert!(matches!(err, ThresholdError::InvalidPartial(0)));
    }

    #[test]
    fn share_public_consistency_with_roster() {
        let c = LabCommittee::generate(3, 5).unwrap();
        for i in 0..5u16 {
            let share_pk = c.share(i).unwrap().public_key_sec1();
            assert_eq!(share_pk, c.roster.member_pks_sec1[i as usize]);
        }
        c.roster.validate_params().unwrap();
    }

    #[test]
    fn wrong_digest_fails_verify() {
        let c = LabCommittee::generate(2, 3).unwrap();
        let d1 = domain_digest(DOMAIN_VE_V1, b"one");
        let d2 = domain_digest(DOMAIN_VE_V1, b"two");
        let combined = lab_sign_with_threshold(&c, &d1).unwrap();
        assert!(verify(&c.roster, &d2, &combined).is_err());
    }

    #[test]
    fn escrow_release_digest_domain_separated() {
        let nf = [1u8; 32];
        let dest = [2u8; 32];
        let asset = [3u8; 32];
        let d = escrow_release_digest(&nf, &dest, 99, &asset);
        let d2 = domain_digest(
            DOMAIN_ESCROW_RELEASE_V1,
            &escrow_release_payload(&nf, &dest, 99, &asset),
        );
        assert_eq!(d, d2);
        let other = domain_digest(
            DOMAIN_VE_V1,
            &escrow_release_payload(&nf, &dest, 99, &asset),
        );
        assert_ne!(d, other);
    }

    #[test]
    fn invalid_params_rejected() {
        assert!(LabCommittee::generate(0, 3).is_err());
        assert!(LabCommittee::generate(4, 3).is_err());
    }

    #[test]
    fn duplicate_member_partials_fail() {
        let c = LabCommittee::generate(2, 3).unwrap();
        let digest = domain_digest(DOMAIN_VE_V1, b"dup");
        let p0 = c.share(0).unwrap().partial_sign_digest(&digest).unwrap();
        let err = combine(&c.roster, &digest, &[p0.clone(), p0]).unwrap_err();
        assert_eq!(err, ThresholdError::DuplicateMember);
    }
}
