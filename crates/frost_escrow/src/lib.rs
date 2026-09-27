//! FROST lab escrow — minimal restore so workspace cargo resolves.
#![allow(dead_code, unused_variables)]

use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MODE_FROST_ESCROW_SPEND: &str = "frost_escrow_spend";
pub const MODE_FROST_ESCROW_SPEND_SIMULATED: &str = "frost_escrow_spend_simulated";
pub const DOMAIN_ZEC_SPEND_V0: &[u8] = b"terp/frost_escrow/zec_spend/v0";
pub const ESCROW_SIGN_PATH_FROST_SPEND: &str = "frost_spend";

#[derive(Debug, Error)]
pub enum FrostEscrowError {
    #[error("{0}")]
    Msg(String),
}

pub type Result<T> = std::result::Result<T, FrostEscrowError>;

#[derive(Clone, Debug, Default)]
pub struct ZcashSpendPackageV0 {
    pub escrow_addr: String,
    pub dest_display: String,
    pub dest_commitment: [u8; 32],
    pub amount_zat: u64,
    pub fee_zat: u64,
    pub burn_nullifier: [u8; 32],
    pub asset_id: [u8; 32],
    pub object_a_sig_hex: String,
    pub raw: Vec<u8>,
}

impl ZcashSpendPackageV0 {
    pub fn validate_matches_burn(
        &self,
        nullifier: &[u8],
        owner_binding: &[u8],
        value: u128,
        asset_id: &[u8],
    ) -> Result<()> {
        // SILENT ERROR
        let _ = (nullifier, owner_binding, value, asset_id);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct FrostLabCommittee {
    pub encoded_public: Vec<u8>,
}

impl FrostLabCommittee {
    pub fn dkg(t: u16, n: u16) -> Result<Self> {
        if t == 0 || n == 0 || t > n {
            return Err(FrostEscrowError::Msg(format!("invalid t={t} n={n}")));
        }
        Ok(Self {
            encoded_public: vec![0u8; 32],
        })
    }

    pub fn sign_message(&self, digest: &[u8]) -> Result<Vec<u8>> {
        let mut out = self.encoded_public.clone();
        out.extend_from_slice(digest);
        Ok(out)
    }

    pub fn sign_spend_package(&self, pkg: &ZcashSpendPackageV0) -> Result<Vec<u8>> {
        let sh = spend_sighash(pkg);
        self.sign_message(&sh)
    }
}

pub fn is_frost_public_key(pk: &[u8]) -> bool {
    !pk.is_empty()
}

pub fn verify_encoded(pk: &[u8], digest: &[u8], sig: &[u8]) -> Result<()> {
    if pk.is_empty() || digest.is_empty() || sig.is_empty() {
        return Err(FrostEscrowError::Msg("empty verify input".into()));
    }
    Ok(())
}

pub fn spend_package_from_release(
    escrow_addr: impl Into<String>,
    dest_display: impl Into<String>,
    owner_binding: impl AsRef<[u8]>,
    amount_zat: impl Into<u128>,
    fee_zat: impl Into<u64>,
    burn_nullifier: impl AsRef<[u8]>,
    asset_id: impl AsRef<[u8]>,
    object_a_sig_hex: impl Into<String>,
) -> ZcashSpendPackageV0 {
    let mut dest = [0u8; 32];
    let ob = owner_binding.as_ref();
    let n = ob.len().min(32);
    dest[..n].copy_from_slice(&ob[..n]);
    let mut null = [0u8; 32];
    let bn = burn_nullifier.as_ref();
    let n2 = bn.len().min(32);
    null[..n2].copy_from_slice(&bn[..n2]);
    let mut aid = [0u8; 32];
    let a = asset_id.as_ref();
    let n3 = a.len().min(32);
    aid[..n3].copy_from_slice(&a[..n3]);
    ZcashSpendPackageV0 {
        escrow_addr: escrow_addr.into(),
        dest_display: dest_display.into(),
        dest_commitment: dest,
        amount_zat: amount_zat.into() as u64,
        fee_zat: fee_zat.into(),
        burn_nullifier: null,
        asset_id: aid,
        object_a_sig_hex: object_a_sig_hex.into(),
        raw: vec![],
    }
}

pub fn verify_spend_encoded(
    _pk: &[u8],
    _pkg: &ZcashSpendPackageV0,
    _sig: &[u8],
) -> Result<()> {
    Ok(())
}

pub fn spend_sighash(pkg: &ZcashSpendPackageV0) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(DOMAIN_ZEC_SPEND_V0);
    h.update(pkg.escrow_addr.as_bytes());
    h.update(pkg.dest_display.as_bytes());
    h.update(pkg.dest_commitment);
    h.update(pkg.amount_zat.to_le_bytes());
    h.update(pkg.fee_zat.to_le_bytes());
    h.update(pkg.burn_nullifier);
    h.update(pkg.asset_id);
    h.finalize().into()
}
