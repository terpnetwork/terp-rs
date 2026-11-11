//! Object B — FROST-signed **remote Zcash spend package** (escrow → sealed dest).
//!
//! Completes the bridge-to-ZEC demo: after Option D burn + Object A policy gate,
//! the committee FROST-signs a canonical spend package bound to burn ν, dest, amount.
//! Remote side then broadcasts (wallet RPC) only when Object B verifies.
//!
//! Domain: `terp/frost/zec-spend/v0`

use crate::{FrostEscrowError, Result, domain_digest};

/// Domain for Object B spend sighash (≠ Object A release domain).
pub const DOMAIN_ZEC_SPEND_V0: &[u8] = b"terp/frost/zec-spend/v0";

/// Product mode: Object A + Object B FROST + remote ZEC tx (or labeled sim after Object B).
pub const MODE_FROST_ESCROW_SPEND: &str = "frost_escrow_spend";
/// Object B verified; remote wallet send simulated / residual.
pub const MODE_FROST_ESCROW_SPEND_SIMULATED: &str = "frost_escrow_spend_simulated";

/// Canonical remote spend description (transparent-first lab package).
///
/// This is the message the FROST committee authorizes for the Zcash leg.
/// Not a full Zcash wire transaction — the **sighash binding** for bridge e2e.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZcashSpendPackageV0 {
    /// Escrow / source address (FROST wallet identity when known).
    pub escrow_addr: String,
    /// User sealed destination display (t-addr or UA).
    pub dest_display: String,
    /// G4 dest commitment (32B).
    pub dest_commitment: [u8; 32],
    /// Amount to release (zatoshis) — must match burn value.
    pub amount_zat: u64,
    /// Fee zatoshis (lab may be 0).
    pub fee_zat: u64,
    /// Continuity with Option D burn.
    pub burn_nullifier: [u8; 32],
    pub asset_id: [u8; 32],
    /// Optional hex of Object A signature (binds spend to prior auth).
    pub object_a_sig_hex: String,
}

impl ZcashSpendPackageV0 {
    /// Canonical payload for sighash (deterministic, no JSON).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(256);
        // length-prefixed strings
        push_str(&mut v, &self.escrow_addr);
        push_str(&mut v, &self.dest_display);
        v.extend_from_slice(&self.dest_commitment);
        v.extend_from_slice(&self.amount_zat.to_le_bytes());
        v.extend_from_slice(&self.fee_zat.to_le_bytes());
        v.extend_from_slice(&self.burn_nullifier);
        v.extend_from_slice(&self.asset_id);
        push_str(&mut v, &self.object_a_sig_hex);
        v
    }

    pub fn validate_matches_burn(
        &self,
        burn_nullifier: &[u8; 32],
        dest_commitment: &[u8; 32],
        amount_zat: u64,
        asset_id: &[u8; 32],
    ) -> Result<()> {
        if &self.burn_nullifier != burn_nullifier {
            return Err(FrostEscrowError::Crypto(
                "spend package burn ν mismatch".into(),
            ));
        }
        if &self.dest_commitment != dest_commitment {
            return Err(FrostEscrowError::Crypto(
                "spend package dest mismatch".into(),
            ));
        }
        if self.amount_zat != amount_zat || self.amount_zat == 0 {
            return Err(FrostEscrowError::Crypto(
                "spend package amount mismatch".into(),
            ));
        }
        if &self.asset_id != asset_id {
            return Err(FrostEscrowError::Crypto(
                "spend package asset mismatch".into(),
            ));
        }
        if self.dest_display.is_empty() {
            return Err(FrostEscrowError::Crypto("spend package empty dest".into()));
        }
        Ok(())
    }
}

fn push_str(v: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let len = b.len() as u32;
    v.extend_from_slice(&len.to_le_bytes());
    v.extend_from_slice(b);
}

/// Object B sighash: `SHA256(DOMAIN_ZEC_SPEND_V0 ‖ encode_payload)`.
pub fn spend_sighash(pkg: &ZcashSpendPackageV0) -> [u8; 32] {
    domain_digest(DOMAIN_ZEC_SPEND_V0, &pkg.encode_payload())
}

/// Released-burn inputs for [`spend_package_from_release`] (param-struct API kept from
/// 570fc729). Byte fields longer than 32 bytes are truncated; shorter ones are zero-padded.
#[derive(Clone, Debug)]
pub struct ReleaseSpend<'a> {
    pub escrow_addr: String,
    pub dest_display: String,
    pub owner_binding: &'a [u8],
    pub amount_zat: u128,
    pub fee_zat: u64,
    pub burn_nullifier: &'a [u8],
    pub asset_id: &'a [u8],
    pub object_a_sig_hex: String,
}

fn pad32(b: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = b.len().min(32);
    out[..n].copy_from_slice(&b[..n]);
    out
}

/// Build spend package from release fields (bridge e2e helper).
pub fn spend_package_from_release(release: ReleaseSpend<'_>) -> ZcashSpendPackageV0 {
    let amount_zat = u64::try_from(release.amount_zat).unwrap_or(u64::MAX);
    spend_package_from_parts(SpendParts {
        escrow_addr: release.escrow_addr,
        dest_display: release.dest_display,
        dest_commitment: pad32(release.owner_binding),
        amount_zat,
        fee_zat: release.fee_zat,
        burn_nullifier: pad32(release.burn_nullifier),
        asset_id: pad32(release.asset_id),
        object_a_sig_hex: release.object_a_sig_hex,
    })
}

/// Fixed-width fields for [`spend_package_from_parts`] (original 491ae651 argument list).
#[derive(Clone, Debug)]
pub struct SpendParts {
    pub escrow_addr: String,
    pub dest_display: String,
    pub dest_commitment: [u8; 32],
    pub amount_zat: u64,
    pub fee_zat: u64,
    pub burn_nullifier: [u8; 32],
    pub asset_id: [u8; 32],
    pub object_a_sig_hex: String,
}

/// Original 491ae651 constructor, as a param struct.
pub fn spend_package_from_parts(p: SpendParts) -> ZcashSpendPackageV0 {
    ZcashSpendPackageV0 {
        escrow_addr: p.escrow_addr,
        dest_display: p.dest_display,
        dest_commitment: p.dest_commitment,
        amount_zat: p.amount_zat,
        fee_zat: p.fee_zat,
        burn_nullifier: p.burn_nullifier,
        asset_id: p.asset_id,
        object_a_sig_hex: p.object_a_sig_hex,
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn pkg8(
    escrow_addr: &str,
    dest_display: &str,
    dest_commitment: [u8; 32],
    amount_zat: u64,
    fee_zat: u64,
    burn_nullifier: [u8; 32],
    asset_id: [u8; 32],
    object_a_sig_hex: impl Into<String>,
) -> ZcashSpendPackageV0 {
    spend_package_from_parts(SpendParts {
        escrow_addr: escrow_addr.into(),
        dest_display: dest_display.into(),
        dest_commitment,
        amount_zat,
        fee_zat,
        burn_nullifier,
        asset_id,
        object_a_sig_hex: object_a_sig_hex.into(),
    })
}

/// Verify Object B FROST signature under FE01 public key.
#[cfg(feature = "frost")]
pub fn verify_spend_encoded(
    committee_pk: &[u8],
    pkg: &ZcashSpendPackageV0,
    sig_bytes: &[u8],
) -> Result<()> {
    let sighash = spend_sighash(pkg);
    crate::verify_encoded(committee_pk, &sighash, sig_bytes)
}

#[cfg(not(feature = "frost"))]
pub fn verify_spend_encoded(
    _committee_pk: &[u8],
    _pkg: &ZcashSpendPackageV0,
    _sig_bytes: &[u8],
) -> Result<()> {
    Err(FrostEscrowError::FrostFeatureDisabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_sighash_domain_separated_from_object_a() {
        use crate::{DOMAIN_ESCROW_RELEASE_V1, escrow_release_digest};
        let pkg = pkg8(
            "tmEscrow", "tmDest", [2u8; 32], 1000, 0, [1u8; 32], [3u8; 32], "aa",
        );
        let sh = spend_sighash(&pkg);
        let oa = escrow_release_digest(&[1u8; 32], &[2u8; 32], 1000, &[3u8; 32]);
        assert_ne!(sh, oa);
        assert_ne!(DOMAIN_ZEC_SPEND_V0, DOMAIN_ESCROW_RELEASE_V1);
    }

    #[test]
    fn spend_payload_deterministic() {
        let a = pkg8("e", "d", [1u8; 32], 5, 1, [2u8; 32], [3u8; 32], "sig");
        let b = pkg8("e", "d", [1u8; 32], 5, 1, [2u8; 32], [3u8; 32], "sig");
        assert_eq!(a.encode_payload(), b.encode_payload());
        assert_eq!(spend_sighash(&a), spend_sighash(&b));
    }
}
