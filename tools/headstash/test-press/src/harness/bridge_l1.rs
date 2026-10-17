//! L1 bridge fixture builders: pure / contract Binary world for cw-orch Mock.
//!
//! Gated by `interface` (pulls `cw-headstash`). Converts hinge labels and
//! Domain B JSON (`bridge_mint_fixture`) into [`cw_headstash::bridge`] CosmWasm
//! types used by `ExecuteMsg::BridgeMintNote`.
//!
//! SSOT: ROUND2-BRIDGE + `terp_seams::bridge::hinge_happy_fixture`.

use cosmwasm_std::Binary;
use cw_headstash::bridge::{
    happy_bridge_mint_world, mock_bridge_proof_bytes, AssetEntry, AssetStatus, BridgeCfg,
    BridgeMintClaimPublic, Hash32, ReflectionSnapshot, DEFAULT_CONFIRMATIONS_K, DEFAULT_MAX_LC_LAG,
};
use terp_seams::bridge::{label_hash, terp_asset_id_from_tacit};

use super::bridge_mint_fixture::BridgeMintFixtureDoc;
use super::claim_from_deposit::{BridgeMintClaimPure, CorridorLabMintPolicy};

/// Full happy L1 mint packet + corridor config (mock_verify=true).
#[derive(Clone, Debug)]
pub struct BridgeL1World {
    pub snapshot: ReflectionSnapshot,
    pub cfg: BridgeCfg,
    pub claim: BridgeMintClaimPublic,
    pub terp_asset: Hash32,
    pub asset: AssetEntry,
    pub proof: Binary,
}

fn parse_hex32(s: &str, name: &str) -> Result<Hash32, String> {
    let h = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(h).map_err(|e| format!("{name}: hex decode {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("{name}: expected 32 bytes, got {}", bytes.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn bin32(h: Hash32) -> Binary {
    Binary::from(h.to_vec())
}

impl BridgeL1World {
    /// Shared happy world (same labels as pure hinge + contract unit tests).
    pub fn happy() -> Self {
        let (snapshot, cfg, claim, terp_asset, asset) = happy_bridge_mint_world();
        Self {
            snapshot,
            cfg,
            claim,
            terp_asset,
            asset,
            proof: mock_bridge_proof_bytes(),
        }
    }

    /// Convert Domain B JSON fixture → CosmWasm Binary world for execute path.
    pub fn from_fixture_doc(doc: &BridgeMintFixtureDoc) -> Result<Self, String> {
        doc.validate_policy().map_err(|e| e.to_string())?;
        let snap = &doc.snapshot;
        let cfg_f = &doc.cfg;
        let c = &doc.claim;
        let a = &doc.asset;

        let terp_asset = parse_hex32(&a.asset_id, "asset.asset_id")?;
        let snapshot = ReflectionSnapshot {
            pool_root: bin32(parse_hex32(&snap.pool_root, "pool_root")?),
            spent_root: bin32(parse_hex32(&snap.spent_root, "spent_root")?),
            burn_root: bin32(parse_hex32(&snap.burn_root, "burn_root")?),
            source_height: snap.source_height,
            tip_height: snap.tip_height,
            confirmations_k: snap.confirmations_k,
            max_lc_lag: snap.max_lc_lag,
            frozen: snap.frozen,
        };
        let cfg = BridgeCfg {
            dest_domain: bin32(parse_hex32(&cfg_f.dest_domain, "cfg.dest_domain")?),
            confirmations_k: cfg_f.confirmations_k,
            max_lc_lag: cfg_f.max_lc_lag,
            lc_client_id: cfg_f.lc_client_id.clone(),
            mock_verify: cfg_f.mock_verify,
            egress_zkid: None,
        };
        let rcm = match &c.rcm {
            Some(h) => Some(bin32(parse_hex32(h, "rcm")?)),
            None => None,
        };
        let claim = BridgeMintClaimPublic {
            source_chain_tag: c.source_chain_tag.clone(),
            tacit_asset_id: bin32(parse_hex32(&c.tacit_asset_id, "tacit_asset_id")?),
            value_u64: c.value_u64,
            nullifier: bin32(parse_hex32(&c.nullifier, "nullifier")?),
            dest_commitment: bin32(parse_hex32(&c.dest_commitment, "dest_commitment")?),
            dest_domain: bin32(parse_hex32(&c.dest_domain, "dest_domain")?),
            claim_id: bin32(parse_hex32(&c.claim_id, "claim_id")?),
            source_pool_root: bin32(parse_hex32(&c.source_pool_root, "source_pool_root")?),
            source_burn_root: bin32(parse_hex32(&c.source_burn_root, "source_burn_root")?),
            source_height: c.source_height,
            domain_binding: bin32(parse_hex32(&c.domain_binding, "domain_binding")?),
            unit_scale: c.unit_scale,
            pool_domain: bin32(parse_hex32(&c.pool_domain, "pool_domain")?),
            cm_public: bin32(parse_hex32(&c.cm_public, "cm_public")?),
            rcm,
            in_burn_set: c.in_burn_set,
            in_pool_root: c.in_pool_root,
            spent_only: c.spent_only,
            src_chain_id: bin32(parse_hex32(&c.src_chain_id, "src_chain_id")?),
            dst_chain_id: bin32(parse_hex32(&c.dst_chain_id, "dst_chain_id")?),
            lc_client_id: bin32(parse_hex32(&c.lc_client_id, "lc_client_id")?),
            burn_dest_commitment: bin32(parse_hex32(
                &c.burn_dest_commitment,
                "burn_dest_commitment",
            )?),
        };
        let status = match a.status.to_lowercase().as_str() {
            "active" | "" => AssetStatus::Active,
            "paused" => AssetStatus::Paused,
            "frozen" => AssetStatus::Frozen,
            other => return Err(format!("unknown asset status {other}")),
        };
        let asset = AssetEntry {
            asset_id: bin32(terp_asset),
            local_denom: a.local_denom.clone(),
            origin: a.origin.clone(),
            status,
        };
        let proof = match &doc.proof_hex {
            Some(h) => {
                let raw = h.strip_prefix("0x").unwrap_or(h);
                let b = hex::decode(raw).map_err(|e| format!("proof_hex: {e}"))?;
                if b.is_empty() {
                    mock_bridge_proof_bytes()
                } else {
                    Binary::from(b)
                }
            }
            None => mock_bridge_proof_bytes(),
        };
        Ok(Self {
            snapshot,
            cfg,
            claim,
            terp_asset,
            asset,
            proof,
        })
    }

    /// Claim mutated for H-1 spent-only reject.
    pub fn with_spent_only(mut self) -> Self {
        self.claim.in_burn_set = false;
        self.claim.spent_only = true;
        self
    }

    /// Nullifier Binary for `IsBridgeMinted` queries.
    pub fn nullifier_bin(&self) -> Binary {
        self.claim.nullifier.clone()
    }

    /// Build L1 world from deposit-backed pure claim + lab policy.
    ///
    /// Identity (ν, value, dest) come from the claim; roots/snapshot/cfg match
    /// policy (defaults align with happy hinge so `configure_bridge` still works).
    /// Lab membership flags on claim are labeled mock under `mock_verify`.
    pub fn from_deposit_claim(claim: &BridgeMintClaimPure, policy: &CorridorLabMintPolicy) -> Self {
        let unit_scale = policy.unit_scale;
        let terp_asset = terp_asset_id_from_tacit(
            &policy.source_chain_tag,
            &policy.tacit_asset_id,
            unit_scale,
        );
        let spent_root = label_hash("spent-root-1");
        let height = claim.source_height;
        let tip = height + DEFAULT_CONFIRMATIONS_K;
        let snapshot = ReflectionSnapshot {
            pool_root: bin32(claim.source_pool_root),
            spent_root: bin32(spent_root),
            burn_root: bin32(claim.source_burn_root),
            source_height: height,
            tip_height: tip,
            confirmations_k: DEFAULT_CONFIRMATIONS_K,
            max_lc_lag: DEFAULT_MAX_LC_LAG,
            frozen: false,
        };
        let cfg = BridgeCfg {
            dest_domain: bin32(claim.dest_domain),
            confirmations_k: DEFAULT_CONFIRMATIONS_K,
            max_lc_lag: DEFAULT_MAX_LC_LAG,
            lc_client_id: "08-wasm-tacit-reflection-0".into(),
            // Labeled lab default — not production verify (D7 residual).
            mock_verify: true,
            egress_zkid: None,
        };
        let rcm = claim.rcm.map(bin32);
        let cw_claim = BridgeMintClaimPublic {
            source_chain_tag: claim.source_chain_tag.clone(),
            tacit_asset_id: bin32(claim.tacit_asset_id),
            value_u64: claim.value_u64,
            nullifier: bin32(claim.nullifier),
            dest_commitment: bin32(claim.dest_commitment),
            dest_domain: bin32(claim.dest_domain),
            claim_id: bin32(claim.claim_id),
            source_pool_root: bin32(claim.source_pool_root),
            source_burn_root: bin32(claim.source_burn_root),
            source_height: claim.source_height,
            domain_binding: bin32(claim.domain_binding),
            unit_scale: claim.unit_scale,
            pool_domain: bin32(claim.pool_domain),
            cm_public: bin32(claim.cm_public),
            rcm,
            in_burn_set: claim.in_burn_set,
            in_pool_root: claim.in_pool_root,
            spent_only: claim.spent_only,
            src_chain_id: bin32(claim.src_chain_id),
            dst_chain_id: bin32(claim.dst_chain_id),
            lc_client_id: bin32(claim.lc_client_id),
            burn_dest_commitment: bin32(claim.burn_dest_commitment),
        };
        let origin = format!("tacit:{}", policy.source_chain_tag);
        let asset = AssetEntry {
            asset_id: bin32(terp_asset),
            local_denom: "ubtc".into(),
            origin: Some(origin),
            status: AssetStatus::Active,
        };
        Self {
            snapshot,
            cfg,
            claim: cw_claim,
            terp_asset,
            asset,
            proof: mock_bridge_proof_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::build_happy_bridge_mint_fixture;

    #[test]
    fn happy_world_shapes() {
        let w = BridgeL1World::happy();
        assert_eq!(w.claim.nullifier.as_slice().len(), 32);
        assert_eq!(w.claim.value_u64, 1_000_000);
        assert!(w.cfg.mock_verify);
        assert!(!w.proof.is_empty());
        assert_eq!(w.asset.local_denom, "ubtc");
    }

    #[test]
    fn from_fixture_doc_matches_happy() {
        let doc = build_happy_bridge_mint_fixture();
        let from_doc = BridgeL1World::from_fixture_doc(&doc).expect("convert");
        let direct = BridgeL1World::happy();
        assert_eq!(from_doc.claim.nullifier, direct.claim.nullifier);
        assert_eq!(from_doc.claim.value_u64, direct.claim.value_u64);
        assert_eq!(from_doc.claim.claim_id, direct.claim.claim_id);
        assert_eq!(from_doc.snapshot.burn_root, direct.snapshot.burn_root);
        assert!(from_doc.cfg.mock_verify);
    }
}
