//! Mint / settle receipt artifacts for corridor funded harness (G3).
//!
//! SSOT shapes: `DESIGN-HARNESS-SETTLE.md` (`MintEvidenceV0`, `SettleReceiptV0`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Minimal evidence packet written after BridgeMintNote (file or in-process).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MintEvidenceV0 {
    pub profile: String,
    pub chain_id: String,
    pub headstash_contract: String,
    /// Bridge *ingress* nullifier (IsBridgeMinted) — NOT pool-spend ν.
    pub bridge_nullifier_hex: String,
    pub cm_public_hex: String,
    pub value_u64: u64,
    pub asset_id_hex: String,
    /// Client-held openings required for G2 spend (rcm never on chain).
    pub rcm_hex: Option<String>,
    pub owner_binding_hex: Option<String>,
    pub mock_verify_bridge: bool,
    pub mint_tx_hash: Option<String>,
    pub intent_id: Option<String>,
}

/// Post-settle automation / host receipt (write JSON artifact).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleReceiptV0 {
    pub profile: String,
    pub stage: String,
    pub status: String,
    pub chain_id: String,
    pub headstash_contract: String,
    pub private_dex_contract: String,
    pub pool_id: u64,
    pub delta_r_in: String,
    pub delta_r_out: String,
    pub r_in_after: String,
    pub r_out_after: String,
    pub nullifiers_hex: Vec<String>,
    pub cm_out_hex: Vec<String>,
    pub bridge_nullifier_hex: String,
    pub mint_cm_public_hex: String,
    pub proof_mode: String,
    pub mock_verify_dex: bool,
    pub intent_id: Option<String>,
    pub settle_tx_hash: Option<String>,
    pub halo2_swap: bool,
    pub skip_ibc_post_swap: bool,
}

/// Lab proof mode labels (receipt / logs).
pub const PROOF_MODE_MOCK_VERIFY_LAB: &str = "mock_verify_lab";
pub const PROOF_MODE_ZK_API: &str = "zk_api";

#[derive(Clone, Debug, thiserror::Error)]
pub enum MintEvidenceError {
    #[error("mint openings missing: {0}")]
    OpeningsMissing(String),
    #[error("invalid hex field {field}: {detail}")]
    InvalidHex { field: String, detail: String },
    #[error("io: {0}")]
    Io(String),
    #[error("json: {0}")]
    Json(String),
}

impl MintEvidenceV0 {
    /// Fail-closed gate: rcm + owner_binding + non-empty cm/asset/value required for settle.
    pub fn require_settle_openings(&self) -> Result<(), MintEvidenceError> {
        if self.value_u64 == 0 {
            return Err(MintEvidenceError::OpeningsMissing("value_u64=0".into()));
        }
        if self.cm_public_hex.trim().is_empty() {
            return Err(MintEvidenceError::OpeningsMissing("cm_public_hex empty".into()));
        }
        if self.asset_id_hex.trim().is_empty() {
            return Err(MintEvidenceError::OpeningsMissing("asset_id_hex empty".into()));
        }
        match &self.rcm_hex {
            None => {
                return Err(MintEvidenceError::OpeningsMissing(
                    "rcm_hex missing — cannot build SwapStatement".into(),
                ));
            }
            Some(h) if h.trim().is_empty() => {
                return Err(MintEvidenceError::OpeningsMissing("rcm_hex empty".into()));
            }
            Some(_) => {}
        }
        match &self.owner_binding_hex {
            None => {
                return Err(MintEvidenceError::OpeningsMissing(
                    "owner_binding_hex missing — cannot build SwapStatement".into(),
                ));
            }
            Some(h) if h.trim().is_empty() => {
                return Err(MintEvidenceError::OpeningsMissing(
                    "owner_binding_hex empty".into(),
                ));
            }
            Some(_) => {}
        }
        // Width checks
        decode_hex32(&self.bridge_nullifier_hex, "bridge_nullifier_hex")?;
        decode_hex32(&self.cm_public_hex, "cm_public_hex")?;
        decode_hex32(&self.asset_id_hex, "asset_id_hex")?;
        decode_hex32(self.rcm_hex.as_ref().unwrap(), "rcm_hex")?;
        decode_hex32(self.owner_binding_hex.as_ref().unwrap(), "owner_binding_hex")?;
        Ok(())
    }

    pub fn write_json(&self, path: &Path) -> Result<(), MintEvidenceError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| MintEvidenceError::Io(e.to_string()))?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| MintEvidenceError::Json(e.to_string()))?;
        fs::write(path, s).map_err(|e| MintEvidenceError::Io(e.to_string()))
    }

    pub fn read_json(path: &Path) -> Result<Self, MintEvidenceError> {
        let s = fs::read_to_string(path).map_err(|e| MintEvidenceError::Io(e.to_string()))?;
        serde_json::from_str(&s).map_err(|e| MintEvidenceError::Json(e.to_string()))
    }
}

impl SettleReceiptV0 {
    pub fn write_json(&self, path: &Path) -> Result<(), MintEvidenceError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| MintEvidenceError::Io(e.to_string()))?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| MintEvidenceError::Json(e.to_string()))?;
        fs::write(path, s).map_err(|e| MintEvidenceError::Io(e.to_string()))
    }

    pub fn read_json(path: &Path) -> Result<Self, MintEvidenceError> {
        let s = fs::read_to_string(path).map_err(|e| MintEvidenceError::Io(e.to_string()))?;
        serde_json::from_str(&s).map_err(|e| MintEvidenceError::Json(e.to_string()))
    }
}

/// Default mint evidence path (`CORRIDOR_MINT_EVIDENCE_PATH` or `/tmp/...`).
pub fn mint_evidence_path() -> PathBuf {
    std::env::var("CORRIDOR_MINT_EVIDENCE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/corridor-mint-evidence.json"))
}

/// Default settle receipt path.
pub fn settle_receipt_path() -> PathBuf {
    std::env::var("CORRIDOR_SETTLE_RECEIPT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/corridor-settle-receipt.json"))
}

pub fn decode_hex32(s: &str, field: &str) -> Result<[u8; 32], MintEvidenceError> {
    let h = s.strip_prefix("0x").unwrap_or(s).trim();
    let bytes = hex::decode(h).map_err(|e| MintEvidenceError::InvalidHex {
        field: field.into(),
        detail: e.to_string(),
    })?;
    if bytes.len() != 32 {
        return Err(MintEvidenceError::InvalidHex {
            field: field.into(),
            detail: format!("expected 32 bytes, got {}", bytes.len()),
        });
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn hex32(b: &[u8]) -> String {
    hex::encode(b)
}

/// Build mint evidence from claim openings after a successful BridgeMintNote.
pub fn mint_evidence_from_claim_fields(
    profile: impl Into<String>,
    chain_id: impl Into<String>,
    headstash_contract: impl Into<String>,
    bridge_nullifier: &[u8],
    cm_public: &[u8],
    value_u64: u64,
    asset_id: &[u8],
    rcm: Option<&[u8]>,
    owner_binding: Option<&[u8]>,
    mock_verify_bridge: bool,
    mint_tx_hash: Option<String>,
    intent_id: Option<String>,
) -> MintEvidenceV0 {
    MintEvidenceV0 {
        profile: profile.into(),
        chain_id: chain_id.into(),
        headstash_contract: headstash_contract.into(),
        bridge_nullifier_hex: hex32(bridge_nullifier),
        cm_public_hex: hex32(cm_public),
        value_u64,
        asset_id_hex: hex32(asset_id),
        rcm_hex: rcm.map(hex32),
        owner_binding_hex: owner_binding.map(hex32),
        mock_verify_bridge,
        mint_tx_hash,
        intent_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_evidence(with_openings: bool) -> MintEvidenceV0 {
        MintEvidenceV0 {
            profile: "ict_local_funded".into(),
            chain_id: "test-1".into(),
            headstash_contract: "terp1hs".into(),
            bridge_nullifier_hex: hex::encode([1u8; 32]),
            cm_public_hex: hex::encode([2u8; 32]),
            value_u64: 1_000,
            asset_id_hex: hex::encode([3u8; 32]),
            rcm_hex: if with_openings {
                Some(hex::encode([4u8; 32]))
            } else {
                None
            },
            owner_binding_hex: if with_openings {
                Some(hex::encode([5u8; 32]))
            } else {
                None
            },
            mock_verify_bridge: true,
            mint_tx_hash: None,
            intent_id: Some("intent-1".into()),
        }
    }

    #[test]
    fn require_openings_fail_closed() {
        let e = sample_evidence(false);
        let err = e.require_settle_openings().unwrap_err();
        assert!(err.to_string().contains("rcm"));
    }

    #[test]
    fn require_openings_ok() {
        sample_evidence(true).require_settle_openings().unwrap();
    }

    #[test]
    fn roundtrip_mint_and_receipt_json() {
        let dir = std::env::temp_dir().join(format!(
            "corridor-evidence-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mint_path = dir.join("mint.json");
        let receipt_path = dir.join("receipt.json");

        let mint = sample_evidence(true);
        mint.write_json(&mint_path).unwrap();
        let mint2 = MintEvidenceV0::read_json(&mint_path).unwrap();
        assert_eq!(mint, mint2);

        let receipt = SettleReceiptV0 {
            profile: "ict_local_funded".into(),
            stage: "chain_settle_swap".into(),
            status: "complete".into(),
            chain_id: "test-1".into(),
            headstash_contract: "terp1hs".into(),
            private_dex_contract: "terp1dex".into(),
            pool_id: 1,
            delta_r_in: "1000".into(),
            delta_r_out: "1994".into(),
            r_in_after: "1001000".into(),
            r_out_after: "1998006".into(),
            nullifiers_hex: vec![hex::encode([9u8; 32])],
            cm_out_hex: vec![hex::encode([8u8; 32])],
            bridge_nullifier_hex: mint.bridge_nullifier_hex.clone(),
            mint_cm_public_hex: mint.cm_public_hex.clone(),
            proof_mode: PROOF_MODE_MOCK_VERIFY_LAB.into(),
            mock_verify_dex: true,
            intent_id: mint.intent_id.clone(),
            settle_tx_hash: None,
            halo2_swap: false,
            skip_ibc_post_swap: false,
        };
        receipt.write_json(&receipt_path).unwrap();
        let r2 = SettleReceiptV0::read_json(&receipt_path).unwrap();
        assert_eq!(r2.status, "complete");
        assert_eq!(r2.proof_mode, PROOF_MODE_MOCK_VERIFY_LAB);
        assert!(!r2.halo2_swap);
        let _ = fs::remove_dir_all(&dir);
    }
}
