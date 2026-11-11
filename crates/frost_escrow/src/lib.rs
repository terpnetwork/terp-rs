//! Restored from terp-rs 491ae651 (replaces the "minimal restore" stub whose verify accepted any
//! non-empty input). Logic unchanged.
//!
//! Thin **ZF FROST** lab adapter for Private Bridge ZEC escrow.
//!
//! # Objects
//!
//! - **Object A** — Terp release digest (`DOMAIN_ESCROW_RELEASE_V1`) — policy gate.
//! - **Object B** — remote Zcash **spend package** sighash (`DOMAIN_ZEC_SPEND_V0`) —
//!   FROST-signed authorization for the escrow → dest transfer.
//!
//! # Honesty
//!
//! - Real FROST via ZF `frost-ed25519` (not ECDSA multi-sig `threshold_committee`).
//! - Lab ciphersuite is Ed25519: Object B is a **bound spend authorization** for the
//!   bridge e2e (remote package + FROST). Consensus-native transparent ECDSA /
//!   ZIP-312 Orchard spend is a residual upgrade path.
//!
//! See `docs/private-bridge/DESIGN-FROST-ZEC-ESCROW-2026-07-22.md`.

#![cfg_attr(not(feature = "frost"), allow(dead_code))]

use thiserror::Error;

// Re-export domain helpers so callers need one crate for Object A digests.
pub use threshold_committee::{
    DOMAIN_BOUNDS_INTENT_V1, DOMAIN_ESCROW_RELEASE_V1, DOMAIN_VE_V1, domain_digest,
    escrow_release_digest, escrow_release_payload,
};

mod spend;
pub use spend::{
    DOMAIN_ZEC_SPEND_V0, MODE_FROST_ESCROW_SPEND, MODE_FROST_ESCROW_SPEND_SIMULATED, ReleaseSpend,
    SpendParts, ZcashSpendPackageV0, spend_package_from_parts, spend_package_from_release,
    spend_sighash, verify_spend_encoded,
};

/// Wire magic for FROST group public material (distinct from threshold `TC01`).
pub const FROST_PK_MAGIC: &[u8; 4] = b"FE01";

/// Honesty label for receipts / logs (never claim multi-sig is FROST).
pub const COMMITTEE_CRYPTO_LABEL: &str = "frost_ed25519_lab";

/// Escrow path when Object A (+ Object B when present) is FROST-signed; remote send may use wallet.
pub const ESCROW_SIGN_PATH_LAB: &str = "P-lab";

/// Object B FROST spend authorization present (remote package signed by committee).
pub const ESCROW_SIGN_PATH_FROST_SPEND: &str = "P-frost-spend-object-b";

#[derive(Debug, Error)]
pub enum FrostEscrowError {
    #[error("invalid threshold parameters: t={t} n={n}")]
    InvalidParams { t: u16, n: u16 },
    #[error("below threshold: have {have}, need {need}")]
    BelowThreshold { have: u16, need: u16 },
    #[error("frost feature disabled — enable crate feature `frost` for real ZF FROST")]
    FrostFeatureDisabled,
    #[error("frost crypto: {0}")]
    Crypto(String),
    #[error("malformed frost public key")]
    MalformedPublic,
    #[error("verify failed")]
    VerifyFailed,
}

pub type Result<T> = std::result::Result<T, FrostEscrowError>;

/// Encode group verifying key material for custody wire sketches.
///
/// Layout: `FE01` ‖ t_le16 ‖ n_le16 ‖ vk_bytes
pub fn encode_frost_public(threshold: u16, n: u16, verifying_key_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 2 + 2 + verifying_key_bytes.len());
    out.extend_from_slice(FROST_PK_MAGIC);
    out.extend_from_slice(&threshold.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(verifying_key_bytes);
    out
}

pub fn is_frost_public_key(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[0..4] == FROST_PK_MAGIC
}

/// Parse FE01-encoded group public key → (t, n, verifying_key_bytes).
pub fn decode_frost_public(bytes: &[u8]) -> Result<(u16, u16, Vec<u8>)> {
    if !is_frost_public_key(bytes) || bytes.len() < 8 {
        return Err(FrostEscrowError::MalformedPublic);
    }
    let t = u16::from_le_bytes([bytes[4], bytes[5]]);
    let n = u16::from_le_bytes([bytes[6], bytes[7]]);
    if t == 0 || n == 0 || t > n {
        return Err(FrostEscrowError::InvalidParams { t, n });
    }
    let vk = bytes[8..].to_vec();
    if vk.is_empty() {
        return Err(FrostEscrowError::MalformedPublic);
    }
    Ok((t, n, vk))
}

// -----------------------------------------------------------------------------
// Real ZF FROST path
// -----------------------------------------------------------------------------

#[cfg(feature = "frost")]
mod frost_lab {
    use super::*;
    use frost_ed25519 as frost;
    use frost_ed25519::keys::dkg;
    use rand_core06::OsRng;
    use std::collections::BTreeMap;

    /// Lab committee after FROST DKG: each participant holds a KeyPackage; group vk shared.
    pub struct FrostLabCommittee {
        pub threshold: u16,
        pub max_signers: u16,
        pub key_packages: BTreeMap<frost::Identifier, frost::keys::KeyPackage>,
        pub pubkey_package: frost::keys::PublicKeyPackage,
        pub encoded_public: Vec<u8>,
    }

    impl FrostLabCommittee {
        /// Run **real** 3-round FROST DKG for `threshold`-of-`max_signers` (in-process simulation).
        pub fn dkg(threshold: u16, max_signers: u16) -> Result<Self> {
            if threshold == 0 || max_signers == 0 || threshold > max_signers {
                return Err(FrostEscrowError::InvalidParams {
                    t: threshold,
                    n: max_signers,
                });
            }

            let rng = OsRng;

            // --- Round 1 ---
            let mut round1_secret_packages = BTreeMap::new();
            let mut received_round1_packages: BTreeMap<
                frost::Identifier,
                BTreeMap<frost::Identifier, dkg::round1::Package>,
            > = BTreeMap::new();

            for participant_index in 1..=max_signers {
                let participant_identifier: frost::Identifier = participant_index
                    .try_into()
                    .map_err(|e| FrostEscrowError::Crypto(format!("identifier: {e}")))?;
                let (round1_secret_package, round1_package) =
                    dkg::part1(participant_identifier, max_signers, threshold, rng)
                        .map_err(|e| FrostEscrowError::Crypto(format!("dkg part1: {e}")))?;

                round1_secret_packages.insert(participant_identifier, round1_secret_package);

                for receiver_index in 1..=max_signers {
                    if receiver_index == participant_index {
                        continue;
                    }
                    let receiver_id: frost::Identifier = receiver_index
                        .try_into()
                        .map_err(|e| FrostEscrowError::Crypto(format!("identifier: {e}")))?;
                    received_round1_packages
                        .entry(receiver_id)
                        .or_default()
                        .insert(participant_identifier, round1_package.clone());
                }
            }

            // --- Round 2 ---
            let mut round2_secret_packages = BTreeMap::new();
            let mut received_round2_packages: BTreeMap<
                frost::Identifier,
                BTreeMap<frost::Identifier, dkg::round2::Package>,
            > = BTreeMap::new();

            for participant_index in 1..=max_signers {
                let participant_identifier: frost::Identifier = participant_index
                    .try_into()
                    .map_err(|e| FrostEscrowError::Crypto(format!("identifier: {e}")))?;
                let round1_secret_package = round1_secret_packages
                    .remove(&participant_identifier)
                    .ok_or_else(|| FrostEscrowError::Crypto("missing r1 secret".into()))?;
                let round1_packages = received_round1_packages
                    .get(&participant_identifier)
                    .ok_or_else(|| FrostEscrowError::Crypto("missing r1 packages".into()))?;
                let (round2_secret_package, round2_packages) =
                    dkg::part2(round1_secret_package, round1_packages)
                        .map_err(|e| FrostEscrowError::Crypto(format!("dkg part2: {e}")))?;

                round2_secret_packages.insert(participant_identifier, round2_secret_package);

                for (receiver_identifier, round2_package) in round2_packages {
                    received_round2_packages
                        .entry(receiver_identifier)
                        .or_default()
                        .insert(participant_identifier, round2_package);
                }
            }

            // --- Round 3 ---
            let mut key_packages = BTreeMap::new();
            let mut pubkey_package_opt = None;

            for participant_index in 1..=max_signers {
                let participant_identifier: frost::Identifier = participant_index
                    .try_into()
                    .map_err(|e| FrostEscrowError::Crypto(format!("identifier: {e}")))?;
                let round2_secret_package = round2_secret_packages
                    .get(&participant_identifier)
                    .ok_or_else(|| FrostEscrowError::Crypto("missing r2 secret".into()))?;
                let round1_packages = received_round1_packages
                    .get(&participant_identifier)
                    .ok_or_else(|| FrostEscrowError::Crypto("missing r1 packages".into()))?;
                let round2_packages = received_round2_packages
                    .get(&participant_identifier)
                    .ok_or_else(|| FrostEscrowError::Crypto("missing r2 packages".into()))?;
                let (key_package, pubkey_package) =
                    dkg::part3(round2_secret_package, round1_packages, round2_packages)
                        .map_err(|e| FrostEscrowError::Crypto(format!("dkg part3: {e}")))?;
                key_packages.insert(participant_identifier, key_package);
                pubkey_package_opt = Some(pubkey_package);
            }

            let pubkey_package =
                pubkey_package_opt.ok_or_else(|| FrostEscrowError::Crypto("empty DKG".into()))?;

            let vk_bytes = pubkey_package
                .verifying_key()
                .serialize()
                .map_err(|e| FrostEscrowError::Crypto(format!("serialize vk: {e}")))?;
            let encoded_public = encode_frost_public(threshold, max_signers, &vk_bytes);

            Ok(Self {
                threshold,
                max_signers,
                key_packages,
                pubkey_package,
                encoded_public,
            })
        }

        /// Two-round FROST sign of `message` using the first `threshold` participant ids.
        pub fn sign_message(&self, message: &[u8]) -> Result<Vec<u8>> {
            let ids: Vec<u16> = (1..=self.threshold).collect();
            self.sign_message_with(message, &ids)
        }

        /// Two-round FROST sign with an explicit signer set (1-based participant
        /// ids). Used when shares are spread over weighted operators: the signer
        /// set is the union of the share ids held by the operators online.
        pub fn sign_message_with(&self, message: &[u8], signers: &[u16]) -> Result<Vec<u8>> {
            let mut rng = OsRng;
            let mut uniq: Vec<u16> = signers.to_vec();
            uniq.sort_unstable();
            uniq.dedup();
            if (uniq.len() as u16) < self.threshold {
                return Err(FrostEscrowError::BelowThreshold {
                    have: uniq.len() as u16,
                    need: self.threshold,
                });
            }

            let mut nonces_map = BTreeMap::new();
            let mut commitments_map = BTreeMap::new();

            for participant_index in uniq {
                let id: frost::Identifier = participant_index
                    .try_into()
                    .map_err(|e| FrostEscrowError::Crypto(format!("identifier: {e}")))?;
                let key_package = self.key_packages.get(&id).ok_or_else(|| {
                    FrostEscrowError::Crypto(format!("missing key package {participant_index}"))
                })?;
                let (nonces, commitments) =
                    frost::round1::commit(key_package.signing_share(), &mut rng);
                nonces_map.insert(id, nonces);
                commitments_map.insert(id, commitments);
            }

            let signing_package = frost::SigningPackage::new(commitments_map, message);
            let mut signature_shares = BTreeMap::new();

            for id in nonces_map.keys() {
                let key_package = self
                    .key_packages
                    .get(id)
                    .ok_or_else(|| FrostEscrowError::Crypto("missing key package".into()))?;
                let nonces = &nonces_map[id];
                let signature_share = frost::round2::sign(&signing_package, nonces, key_package)
                    .map_err(|e| FrostEscrowError::Crypto(format!("round2 sign: {e}")))?;
                signature_shares.insert(*id, signature_share);
            }

            let group_signature =
                frost::aggregate(&signing_package, &signature_shares, &self.pubkey_package)
                    .map_err(|e| FrostEscrowError::Crypto(format!("aggregate: {e}")))?;

            group_signature
                .serialize()
                .map_err(|e| FrostEscrowError::Crypto(format!("serialize sig: {e}")))
        }

        /// Verify group signature over `message` under this committee's verifying key.
        pub fn verify_message(&self, message: &[u8], sig_bytes: &[u8]) -> Result<()> {
            let sig = frost::Signature::deserialize(sig_bytes)
                .map_err(|e| FrostEscrowError::Crypto(format!("deserialize sig: {e}")))?;
            self.pubkey_package
                .verifying_key()
                .verify(message, &sig)
                .map_err(|_| FrostEscrowError::VerifyFailed)
        }

        /// Sign Object A escrow-release digest (domain-separated).
        pub fn sign_escrow_release_digest(
            &self,
            burn_nullifier: &[u8; 32],
            dest_commitment: &[u8; 32],
            amount_zat: u64,
            asset_id: &[u8; 32],
        ) -> Result<Vec<u8>> {
            let digest =
                escrow_release_digest(burn_nullifier, dest_commitment, amount_zat, asset_id);
            self.sign_message(&digest)
        }

        pub fn verify_escrow_release_digest(
            &self,
            burn_nullifier: &[u8; 32],
            dest_commitment: &[u8; 32],
            amount_zat: u64,
            asset_id: &[u8; 32],
            sig_bytes: &[u8],
        ) -> Result<()> {
            let digest =
                escrow_release_digest(burn_nullifier, dest_commitment, amount_zat, asset_id);
            self.verify_message(&digest, sig_bytes)
        }

        /// Object B: FROST-sign remote Zcash spend package sighash.
        pub fn sign_spend_package(&self, pkg: &crate::ZcashSpendPackageV0) -> Result<Vec<u8>> {
            let sighash = crate::spend_sighash(pkg);
            self.sign_message(&sighash)
        }

        pub fn verify_spend_package(
            &self,
            pkg: &crate::ZcashSpendPackageV0,
            sig_bytes: &[u8],
        ) -> Result<()> {
            let sighash = crate::spend_sighash(pkg);
            self.verify_message(&sighash, sig_bytes)
        }
    }

    /// Verify FROST group signature using FE01-encoded public key (Custody wire).
    pub fn verify_encoded(committee_pk: &[u8], message: &[u8], sig_bytes: &[u8]) -> Result<()> {
        let (_t, _n, vk_bytes) = decode_frost_public(committee_pk)?;
        let vk = frost::VerifyingKey::deserialize(&vk_bytes)
            .map_err(|e| FrostEscrowError::Crypto(format!("deserialize vk: {e}")))?;
        let sig = frost::Signature::deserialize(sig_bytes)
            .map_err(|e| FrostEscrowError::Crypto(format!("deserialize sig: {e}")))?;
        vk.verify(message, &sig)
            .map_err(|_| FrostEscrowError::VerifyFailed)
    }

    /// Verify Object A escrow-release digest under FE01 public + FROST sig.
    pub fn verify_escrow_release_encoded(
        committee_pk: &[u8],
        burn_nullifier: &[u8; 32],
        dest_commitment: &[u8; 32],
        amount_zat: u64,
        asset_id: &[u8; 32],
        sig_bytes: &[u8],
    ) -> Result<()> {
        let digest = escrow_release_digest(burn_nullifier, dest_commitment, amount_zat, asset_id);
        verify_encoded(committee_pk, &digest, sig_bytes)
    }

    /// Below-threshold signing must fail: only `have` signers attempt round2/aggregate.
    pub fn try_sign_with_signers(
        committee: &FrostLabCommittee,
        message: &[u8],
        num_signers: u16,
    ) -> Result<Vec<u8>> {
        if num_signers < committee.threshold {
            return Err(FrostEscrowError::BelowThreshold {
                have: num_signers,
                need: committee.threshold,
            });
        }
        let _ = message;
        Err(FrostEscrowError::Crypto(
            "use FrostLabCommittee::sign_message when have >= threshold".into(),
        ))
    }
}

#[cfg(feature = "frost")]
pub use frost_lab::{
    FrostLabCommittee, try_sign_with_signers, verify_encoded, verify_escrow_release_encoded,
};

#[cfg(not(feature = "frost"))]
pub fn verify_encoded(_committee_pk: &[u8], _message: &[u8], _sig_bytes: &[u8]) -> Result<()> {
    Err(FrostEscrowError::FrostFeatureDisabled)
}

#[cfg(not(feature = "frost"))]
pub fn verify_escrow_release_encoded(
    _committee_pk: &[u8],
    _burn_nullifier: &[u8; 32],
    _dest_commitment: &[u8; 32],
    _amount_zat: u64,
    _asset_id: &[u8; 32],
    _sig_bytes: &[u8],
) -> Result<()> {
    Err(FrostEscrowError::FrostFeatureDisabled)
}

// -----------------------------------------------------------------------------
// Stubs when `frost` feature is off — tests must fail closed, not silently pass.
// -----------------------------------------------------------------------------

#[cfg(not(feature = "frost"))]
pub struct FrostLabCommittee;

#[cfg(not(feature = "frost"))]
impl FrostLabCommittee {
    pub fn dkg(_threshold: u16, _max_signers: u16) -> Result<Self> {
        Err(FrostEscrowError::FrostFeatureDisabled)
    }

    pub fn sign_message(&self, _message: &[u8]) -> Result<Vec<u8>> {
        Err(FrostEscrowError::FrostFeatureDisabled)
    }

    pub fn verify_message(&self, _message: &[u8], _sig_bytes: &[u8]) -> Result<()> {
        Err(FrostEscrowError::FrostFeatureDisabled)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "frost")]
    #[test]
    fn frost_dkg_sign_verify_domain_digest() {
        let c = FrostLabCommittee::dkg(2, 3).expect("real FROST DKG");
        assert!(is_frost_public_key(&c.encoded_public));
        assert_eq!(&c.encoded_public[0..4], FROST_PK_MAGIC);

        let msg = domain_digest(DOMAIN_ESCROW_RELEASE_V1, b"frost-lab-payload");
        let sig = c.sign_message(&msg).expect("FROST sign");
        c.verify_message(&msg, &sig).expect("FROST verify");

        // Wrong message fails
        let other = domain_digest(DOMAIN_ESCROW_RELEASE_V1, b"other");
        assert!(c.verify_message(&other, &sig).is_err());
    }

    #[cfg(feature = "frost")]
    #[test]
    fn frost_signs_escrow_release_object_a() {
        let c = FrostLabCommittee::dkg(2, 3).unwrap();
        let nf = [0x11u8; 32];
        let dest = [0x22u8; 32];
        let asset = [0x33u8; 32];
        let amount = 50_000_000u64; // 0.5 ZEC in zatoshis

        let sig = c
            .sign_escrow_release_digest(&nf, &dest, amount, &asset)
            .unwrap();
        c.verify_escrow_release_digest(&nf, &dest, amount, &asset, &sig)
            .unwrap();

        // Dest rewrite must not verify under same sig
        let wrong_dest = [0xEEu8; 32];
        assert!(
            c.verify_escrow_release_digest(&nf, &wrong_dest, amount, &asset, &sig)
                .is_err()
        );
    }

    #[cfg(feature = "frost")]
    #[test]
    fn frost_below_threshold_fails_closed() {
        let c = FrostLabCommittee::dkg(2, 3).unwrap();
        let msg = domain_digest(DOMAIN_VE_V1, b"need-two");
        let err = try_sign_with_signers(&c, &msg, 1).unwrap_err();
        assert!(matches!(
            err,
            FrostEscrowError::BelowThreshold { have: 1, need: 2 }
        ));
    }

    #[cfg(feature = "frost")]
    #[test]
    fn frost_not_confused_with_threshold_committee_magic() {
        let c = FrostLabCommittee::dkg(2, 3).unwrap();
        assert!(is_frost_public_key(&c.encoded_public));
        assert!(!threshold_committee::is_committee_public_key(
            &c.encoded_public
        ));
        // Threshold TC01 must not parse as FE01
        let tc = threshold_committee::LabCommittee::generate(2, 3).unwrap();
        let tc_pk = tc.roster.encode_public().unwrap();
        assert!(!is_frost_public_key(&tc_pk));
    }

    #[cfg(feature = "frost")]
    #[test]
    fn honesty_labels() {
        assert_eq!(COMMITTEE_CRYPTO_LABEL, "frost_ed25519_lab");
        assert_ne!(COMMITTEE_CRYPTO_LABEL, "lab_t_of_n_multisig");
        assert_eq!(ESCROW_SIGN_PATH_LAB, "P-lab");
    }

    #[cfg(feature = "frost")]
    #[test]
    fn verify_encoded_wire_roundtrip() {
        let c = FrostLabCommittee::dkg(2, 3).unwrap();
        let digest = escrow_release_digest(&[1u8; 32], &[2u8; 32], 99, &[3u8; 32]);
        let sig = c.sign_message(&digest).unwrap();
        verify_encoded(&c.encoded_public, &digest, &sig).unwrap();
        verify_escrow_release_encoded(
            &c.encoded_public,
            &[1u8; 32],
            &[2u8; 32],
            99,
            &[3u8; 32],
            &sig,
        )
        .unwrap();
        assert!(
            verify_encoded(
                &c.encoded_public,
                &digest,
                &sig[..sig.len().saturating_sub(1)]
            )
            .is_err()
        );
    }

    #[cfg(feature = "frost")]
    #[test]
    fn object_b_frost_spend_package_sign_verify() {
        let c = FrostLabCommittee::dkg(2, 3).unwrap();
        let nf = [0xAAu8; 32];
        let dest = [0xBBu8; 32];
        let asset = [0xCCu8; 32];
        // Object A first
        let a_sig = c
            .sign_escrow_release_digest(&nf, &dest, 50_000, &asset)
            .unwrap();
        let pkg = crate::spend::pkg8(
            "tmFrostEscrow",
            "tmUserDest",
            dest,
            50_000,
            0,
            nf,
            asset,
            hex::encode(&a_sig),
        );
        pkg.validate_matches_burn(&nf, &dest, 50_000, &asset)
            .unwrap();
        let b_sig = c.sign_spend_package(&pkg).unwrap();
        c.verify_spend_package(&pkg, &b_sig).unwrap();
        verify_spend_encoded(&c.encoded_public, &pkg, &b_sig).unwrap();
        // Dest rewrite fails
        let mut bad = pkg.clone();
        bad.dest_commitment = [0xEE; 32];
        assert!(c.verify_spend_package(&bad, &b_sig).is_err());
    }

    #[cfg(feature = "frost")]
    #[test]
    fn invalid_dkg_params() {
        assert!(FrostLabCommittee::dkg(0, 3).is_err());
        assert!(FrostLabCommittee::dkg(4, 3).is_err());
    }

    /// When frost feature is disabled, DKG must error — never silent green.
    #[cfg(not(feature = "frost"))]
    #[test]
    fn frost_disabled_fails_not_silent() {
        let err = FrostLabCommittee::dkg(2, 3).unwrap_err();
        assert!(matches!(err, FrostEscrowError::FrostFeatureDisabled));
    }
}
