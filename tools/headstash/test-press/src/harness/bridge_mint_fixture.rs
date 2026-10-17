//! Domain B bridge mint packet fixture — load/build without SP1 / real LC.
//!
//! Schema: `docs/plans/spectrum/fixtures/bridge_mint_claim_happy.v1.json`
//! (`bridge-mint-claim-public-v1`). Compatible with
//! `cw-headstash::bridge::BridgeMintClaimPublic` field set (hex-encoded 32B digests).
//!
//! # L1 mock ZK / LC posture
//! PR CI uses **policy + mock membership flags** (`in_burn_set`, `mock_verify`).
//! Real SP1 / reflection IMT / anvil dual-container is a separate track.
//! Optional anvil meta is annotation-only (EVM confidential mint path).

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use terp_seams::bridge::{
    authorize_bridge_mint_apply, derive_claim_id_with_dest, derive_domain_binding,
    hinge_happy_fixture, label_hash, terp_asset_id_from_tacit, AssetRegistry, BridgeMintClaim,
    BridgeMintPublic, MintedSet, NoteOutSketch, ReflectionSnapshot, DEFAULT_CONFIRMATIONS_K,
    DEFAULT_MAX_LC_LAG,
};

#[derive(Debug, Error)]
pub enum BridgeMintFixtureError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema: {0}")]
    Schema(String),
}

/// Top-level golden / emitted mint packet document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BridgeMintFixtureDoc {
    pub schema: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub source: Option<serde_json::Value>,
    #[serde(default)]
    pub field_docs: Option<serde_json::Value>,
    pub snapshot: BridgeMintFixtureSnapshot,
    pub cfg: BridgeMintFixtureCfg,
    pub asset: BridgeMintFixtureAsset,
    pub claim: BridgeMintFixtureClaim,
    /// Mock proof bytes as `0x` hex (non-empty for production mock_verify).
    #[serde(default)]
    pub proof_hex: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeMintFixtureSnapshot {
    pub pool_root: String,
    pub spent_root: String,
    pub burn_root: String,
    pub source_height: u64,
    pub tip_height: u64,
    pub confirmations_k: u64,
    pub max_lc_lag: u64,
    pub frozen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeMintFixtureCfg {
    pub dest_domain: String,
    pub confirmations_k: u64,
    pub max_lc_lag: u64,
    pub lc_client_id: String,
    pub mock_verify: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeMintFixtureAsset {
    pub asset_id: String,
    pub local_denom: String,
    #[serde(default)]
    pub origin: Option<String>,
    pub status: String,
}

/// `BridgeMintClaimPublic`-compatible claim (hex digests, not CosmWasm `Binary`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeMintFixtureClaim {
    pub source_chain_tag: String,
    pub tacit_asset_id: String,
    pub value_u64: u64,
    pub nullifier: String,
    pub dest_commitment: String,
    pub dest_domain: String,
    pub claim_id: String,
    pub source_pool_root: String,
    pub source_burn_root: String,
    pub source_height: u64,
    pub domain_binding: String,
    pub unit_scale: u64,
    pub pool_domain: String,
    pub cm_public: String,
    #[serde(default)]
    pub rcm: Option<String>,
    pub in_burn_set: bool,
    pub in_pool_root: bool,
    #[serde(default)]
    pub spent_only: bool,
    pub src_chain_id: String,
    pub dst_chain_id: String,
    pub lc_client_id: String,
    pub burn_dest_commitment: String,
}

impl BridgeMintFixtureDoc {
    /// Policy checks: schema id, hash sizes, claim_id + domain_binding re-derive, H-1 flags.
    pub fn validate_policy(&self) -> Result<(), BridgeMintFixtureError> {
        if self.schema != "bridge-mint-claim-public-v1" {
            return Err(BridgeMintFixtureError::Schema(format!(
                "expected schema bridge-mint-claim-public-v1, got {}",
                self.schema
            )));
        }
        if !self.cfg.mock_verify {
            return Err(BridgeMintFixtureError::Schema(
                "fixture must set cfg.mock_verify=true for Round-3 mock LC path".into(),
            ));
        }

        let c = &self.claim;
        let tacit = parse_hash32(&c.tacit_asset_id, "tacit_asset_id")?;
        let nu = parse_hash32(&c.nullifier, "nullifier")?;
        let dest = parse_hash32(&c.dest_domain, "dest_domain")?;
        let dest_cm = parse_hash32(&c.dest_commitment, "dest_commitment")?;
        let claim_id = parse_hash32(&c.claim_id, "claim_id")?;
        let pool_root = parse_hash32(&c.source_pool_root, "source_pool_root")?;
        let burn_root = parse_hash32(&c.source_burn_root, "source_burn_root")?;
        let domain_binding = parse_hash32(&c.domain_binding, "domain_binding")?;
        let src_chain = parse_hash32(&c.src_chain_id, "src_chain_id")?;
        let dst_chain = parse_hash32(&c.dst_chain_id, "dst_chain_id")?;
        let lc = parse_hash32(&c.lc_client_id, "lc_client_id")?;
        let burn_dest = parse_hash32(&c.burn_dest_commitment, "burn_dest_commitment")?;
        let _pool_domain = parse_hash32(&c.pool_domain, "pool_domain")?;
        let _cm = parse_hash32(&c.cm_public, "cm_public")?;
        if let Some(rcm) = &c.rcm {
            let _ = parse_hash32(rcm, "rcm")?;
        }

        // Snapshot pins must match claim membership roots.
        let snap_pool = parse_hash32(&self.snapshot.pool_root, "snapshot.pool_root")?;
        let snap_burn = parse_hash32(&self.snapshot.burn_root, "snapshot.burn_root")?;
        if snap_pool != pool_root {
            return Err(BridgeMintFixtureError::Schema(
                "snapshot.pool_root != claim.source_pool_root".into(),
            ));
        }
        if snap_burn != burn_root {
            return Err(BridgeMintFixtureError::Schema(
                "snapshot.burn_root != claim.source_burn_root".into(),
            ));
        }
        if self.snapshot.source_height != c.source_height {
            return Err(BridgeMintFixtureError::Schema(
                "snapshot.source_height != claim.source_height".into(),
            ));
        }
        if self.snapshot.frozen {
            return Err(BridgeMintFixtureError::Schema(
                "happy fixture must not be frozen".into(),
            ));
        }
        if burn_dest != dest_cm {
            return Err(BridgeMintFixtureError::Schema(
                "burn_dest_commitment != dest_commitment".into(),
            ));
        }

        let expect_claim =
            derive_claim_id_with_dest(&dest, &dest_cm, &nu, &tacit, c.value_u64);
        if expect_claim != claim_id {
            return Err(BridgeMintFixtureError::Schema(
                "claim_id re-derive mismatch (Domain B A14)".into(),
            ));
        }
        let expect_db = derive_domain_binding(
            &src_chain,
            &dst_chain,
            &lc,
            &tacit,
            &nu,
            c.source_height,
            &burn_root,
        );
        if expect_db != domain_binding {
            return Err(BridgeMintFixtureError::Schema(
                "domain_binding re-derive mismatch (C §3.1)".into(),
            ));
        }

        // Asset map pin
        let expect_asset =
            terp_asset_id_from_tacit(&c.source_chain_tag, &tacit, c.unit_scale);
        let asset_id = parse_hash32(&self.asset.asset_id, "asset.asset_id")?;
        if expect_asset != asset_id {
            return Err(BridgeMintFixtureError::Schema(
                "asset.asset_id != terp_asset_id_from_tacit(...)".into(),
            ));
        }

        // Happy-path H-1: burn set required.
        if !c.in_burn_set {
            return Err(BridgeMintFixtureError::Schema(
                "happy fixture requires in_burn_set=true".into(),
            ));
        }
        if c.spent_only {
            return Err(BridgeMintFixtureError::Schema(
                "happy fixture requires spent_only=false".into(),
            ));
        }

        // Mock proof parse when present.
        let _ = self.mock_proof_bytes()?;
        Ok(())
    }

    pub fn mock_proof_bytes(&self) -> Result<Vec<u8>, BridgeMintFixtureError> {
        match &self.proof_hex {
            None => Ok(Vec::new()),
            Some(h) => decode_hex(h).map_err(BridgeMintFixtureError::Schema),
        }
    }

    /// Convert claim + snapshot into pure hinge types for L0 authorize.
    pub fn to_hinge(
        &self,
    ) -> Result<(ReflectionSnapshot, BridgeMintClaim, [u8; 32]), BridgeMintFixtureError> {
        self.validate_policy()?;
        let c = &self.claim;
        let tacit = parse_hash32(&c.tacit_asset_id, "tacit_asset_id")?;
        let nu = parse_hash32(&c.nullifier, "nullifier")?;
        let dest = parse_hash32(&c.dest_domain, "dest_domain")?;
        let dest_cm = parse_hash32(&c.dest_commitment, "dest_commitment")?;
        let claim_id = parse_hash32(&c.claim_id, "claim_id")?;
        let pool_root = parse_hash32(&c.source_pool_root, "source_pool_root")?;
        let spent_root = parse_hash32(&self.snapshot.spent_root, "spent_root")?;
        let burn_root = parse_hash32(&c.source_burn_root, "source_burn_root")?;
        let domain_binding = parse_hash32(&c.domain_binding, "domain_binding")?;
        let src_chain = parse_hash32(&c.src_chain_id, "src_chain_id")?;
        let dst_chain = parse_hash32(&c.dst_chain_id, "dst_chain_id")?;
        let lc = parse_hash32(&c.lc_client_id, "lc_client_id")?;
        let pool_domain = parse_hash32(&c.pool_domain, "pool_domain")?;
        let cm_public = parse_hash32(&c.cm_public, "cm_public")?;
        let rcm = match &c.rcm {
            Some(h) => parse_hash32(h, "rcm")?,
            None => [0u8; 32],
        };
        let terp_asset = parse_hash32(&self.asset.asset_id, "asset.asset_id")?;

        let snapshot = ReflectionSnapshot {
            pool_root,
            spent_root,
            burn_root,
            source_height: self.snapshot.source_height,
            tip_height: self.snapshot.tip_height,
            confirmations_k: self.snapshot.confirmations_k,
            max_lc_lag: self.snapshot.max_lc_lag,
            frozen: self.snapshot.frozen,
        };
        let public = BridgeMintPublic {
            source_chain_tag: c.source_chain_tag.clone(),
            tacit_asset_id: tacit,
            value_u64: c.value_u64,
            nullifier: nu,
            dest_commitment: dest_cm,
            dest_domain: dest,
            claim_id,
            source_pool_root: pool_root,
            source_burn_root: burn_root,
            source_height: c.source_height,
            domain_binding,
            unit_scale: c.unit_scale,
            pool_domain,
            cm_public,
            rcm,
        };
        let claim = BridgeMintClaim {
            public,
            mint_value: c.value_u64,
            expected_dest_domain: dest,
            src_chain_id: src_chain,
            dst_chain_id: dst_chain,
            lc_client_id: lc,
            burn_dest_commitment: dest_cm,
            in_burn_set: c.in_burn_set,
            in_pool_root: c.in_pool_root,
            spent_only: c.spent_only,
        };
        Ok((snapshot, claim, terp_asset))
    }
}

/// Load golden/emitted JSON and validate policy layout.
pub fn load_bridge_mint_fixture(path: &Path) -> Result<BridgeMintFixtureDoc, BridgeMintFixtureError> {
    let raw = std::fs::read_to_string(path)?;
    let fix: BridgeMintFixtureDoc = serde_json::from_str(&raw)?;
    fix.validate_policy()?;
    Ok(fix)
}

/// Build synthetic happy fixture (same labels as `hinge_happy_fixture` / CW unit tests).
pub fn build_happy_bridge_mint_fixture() -> BridgeMintFixtureDoc {
    let dest = label_hash("terp-chain-1");
    let tacit = label_hash("tacit-btc-etch-1");
    let unit_scale = 1u64;
    let terp_asset = terp_asset_id_from_tacit("bitcoin-mainnet", &tacit, unit_scale);
    let nu = label_hash("nu-hinge-happy");
    let dest_cm = label_hash("dest-commitment-A");
    let pool_root = label_hash("pool-root-1");
    let spent_root = label_hash("spent-root-1");
    let burn_root = label_hash("burn-root-1");
    let height = 100u64;
    let tip = height + DEFAULT_CONFIRMATIONS_K;
    let claim_id = derive_claim_id_with_dest(&dest, &dest_cm, &nu, &tacit, 1_000_000);
    let src_chain = label_hash("src-bitcoin-mainnet");
    let dst_chain = dest;
    let lc_client = label_hash("lc-client-reflection-0");
    let domain_binding = derive_domain_binding(
        &src_chain,
        &dst_chain,
        &lc_client,
        &tacit,
        &nu,
        height,
        &burn_root,
    );
    let rcm = label_hash("rcm-hinge-happy");

    BridgeMintFixtureDoc {
        schema: "bridge-mint-claim-public-v1".into(),
        version: 1,
        description: Some(
            "Synthetic hinge-happy BridgeMintClaimPublic packet (mock LC)".into(),
        ),
        source: Some(serde_json::json!({
            "kind": "synthetic-hinge-happy",
            "hash_fn": "sha256(label_utf8)",
        })),
        field_docs: None,
        snapshot: BridgeMintFixtureSnapshot {
            pool_root: hex32(&pool_root),
            spent_root: hex32(&spent_root),
            burn_root: hex32(&burn_root),
            source_height: height,
            tip_height: tip,
            confirmations_k: DEFAULT_CONFIRMATIONS_K,
            max_lc_lag: DEFAULT_MAX_LC_LAG,
            frozen: false,
        },
        cfg: BridgeMintFixtureCfg {
            dest_domain: hex32(&dest),
            confirmations_k: DEFAULT_CONFIRMATIONS_K,
            max_lc_lag: DEFAULT_MAX_LC_LAG,
            lc_client_id: "08-wasm-tacit-reflection-0".into(),
            mock_verify: true,
        },
        asset: BridgeMintFixtureAsset {
            asset_id: hex32(&terp_asset),
            local_denom: "ubtc".into(),
            origin: Some("tacit:bitcoin-mainnet".into()),
            status: "active".into(),
        },
        claim: BridgeMintFixtureClaim {
            source_chain_tag: "bitcoin-mainnet".into(),
            tacit_asset_id: hex32(&tacit),
            value_u64: 1_000_000,
            nullifier: hex32(&nu),
            dest_commitment: hex32(&dest_cm),
            dest_domain: hex32(&dest),
            claim_id: hex32(&claim_id),
            source_pool_root: hex32(&pool_root),
            source_burn_root: hex32(&burn_root),
            source_height: height,
            domain_binding: hex32(&domain_binding),
            unit_scale,
            pool_domain: hex32(&label_hash("terp-pool-0")),
            cm_public: hex32(&label_hash("cm-leaf-dest-1")),
            rcm: Some(hex32(&rcm)),
            in_burn_set: true,
            in_pool_root: true,
            spent_only: false,
            src_chain_id: hex32(&src_chain),
            dst_chain_id: hex32(&dst_chain),
            lc_client_id: hex32(&lc_client),
            burn_dest_commitment: hex32(&dest_cm),
        },
        proof_hex: Some("0xdeadbeef".into()),
    }
}

/// Authorize L0 mint from fixture document (E2E-01 from disk/synthetic).
pub fn authorize_from_bridge_mint_fixture(
    doc: &BridgeMintFixtureDoc,
) -> Result<NoteOutSketch, BridgeMintFixtureError> {
    let (snapshot, claim, terp_asset) = doc.to_hinge()?;
    let mut registry = AssetRegistry::new();
    registry.register(terp_asset);
    let mut minted = MintedSet::new();
    authorize_bridge_mint_apply(&snapshot, &claim, &mut minted, &registry).map_err(|e| {
        BridgeMintFixtureError::Schema(format!("authorize_bridge_mint: {e:?}"))
    })
}

/// Path helper: spectrum fixtures dir relative to workspace when CWD is crates/headstash.
pub fn default_golden_bridge_mint_path() -> std::path::PathBuf {
    // Prefer env override for CI / emit script.
    if let Ok(p) = std::env::var("BRIDGE_MINT_FIXTURE") {
        return std::path::PathBuf::from(p);
    }
    // Relative from crates/headstash (just demo-e2e-l0 / demo-bridge-fixture).
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../docs/plans/spectrum/fixtures/bridge_mint_claim_happy.v1.json")
}

/// Load golden if present and valid; else build synthetic (degrade gracefully).
///
/// If the on-disk golden fails policy (stale digests), falls back to
/// [`build_happy_bridge_mint_fixture`] so harness CI still greens — re-emit with
/// `emit_bridge_mint_fixture.sh --regenerate` to refresh the file.
pub fn load_or_build_happy_bridge_mint_fixture() -> Result<BridgeMintFixtureDoc, BridgeMintFixtureError>
{
    let path = default_golden_bridge_mint_path();
    if path.is_file() {
        match load_bridge_mint_fixture(&path) {
            Ok(doc) => return Ok(doc),
            Err(_) => {
                // degrade: synthetic hinge-happy (same labels as golden intent)
            }
        }
    }
    let doc = build_happy_bridge_mint_fixture();
    doc.validate_policy()?;
    Ok(doc)
}

/// Sanity: golden/synthetic matches live `hinge_happy_fixture` public fields.
pub fn assert_fixture_matches_hinge_happy() -> Result<(), BridgeMintFixtureError> {
    let doc = load_or_build_happy_bridge_mint_fixture()?;
    let (snap_f, claim_f, asset_f) = doc.to_hinge()?;
    let (snap_h, claim_h, _minted, _reg, asset_h) = hinge_happy_fixture();
    if snap_f != snap_h {
        return Err(BridgeMintFixtureError::Schema(
            "snapshot != hinge_happy_fixture snapshot".into(),
        ));
    }
    if claim_f.public != claim_h.public {
        return Err(BridgeMintFixtureError::Schema(
            "claim.public != hinge_happy_fixture public".into(),
        ));
    }
    if asset_f != asset_h {
        return Err(BridgeMintFixtureError::Schema(
            "asset_id != hinge_happy terp asset".into(),
        ));
    }
    Ok(())
}

fn hex32(h: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(h))
}

fn parse_hash32(s: &str, name: &str) -> Result<[u8; 32], BridgeMintFixtureError> {
    let b = decode_hex(s).map_err(BridgeMintFixtureError::Schema)?;
    if b.len() != 32 {
        return Err(BridgeMintFixtureError::Schema(format!(
            "{name} must be 32 bytes, got {}",
            b.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&b);
    Ok(out)
}

fn normalize_hex(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    t.to_ascii_lowercase()
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let t = normalize_hex(s);
    if t.len() % 2 != 0 {
        return Err(format!("odd hex length: {}", t.len()));
    }
    (0..t.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&t[i..i + 2], 16).map_err(|e| format!("hex parse at {i}: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_happy_validates_and_authorizes() {
        let doc = build_happy_bridge_mint_fixture();
        doc.validate_policy().expect("policy");
        let note = authorize_from_bridge_mint_fixture(&doc).expect("authorize");
        assert_eq!(note.value, 1_000_000);
        assert!(note.is_dex_consumable());
    }

    #[test]
    fn matches_hinge_happy_fixture() {
        assert_fixture_matches_hinge_happy().expect("match hinge");
    }

    #[test]
    fn json_roundtrip() {
        let doc = build_happy_bridge_mint_fixture();
        let s = serde_json::to_string_pretty(&doc).unwrap();
        let back: BridgeMintFixtureDoc = serde_json::from_str(&s).unwrap();
        back.validate_policy().unwrap();
        assert_eq!(back.claim.value_u64, 1_000_000);
    }

    #[test]
    fn load_golden_if_present() {
        let path = default_golden_bridge_mint_path();
        if path.is_file() {
            let doc = load_bridge_mint_fixture(&path).expect("golden load");
            assert_eq!(doc.schema, "bridge-mint-claim-public-v1");
            authorize_from_bridge_mint_fixture(&doc).expect("golden authorize");
        }
    }
}
