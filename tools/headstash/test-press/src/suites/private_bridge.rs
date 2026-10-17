//! Compose **private bridge** (+ optional swap) suite around **one** mint: `cw-headstash`.
//!
//! Design SSOT: `docs/plans/spectrum/E2E-HARNESS-PLAN.md`  
//! Clarity: `CLARITY-cw-headstash-router-and-asset-registry.md` (no parallel mint wasm)  
//! Case IDs: E2E-01..E2E-20
//!
//! # Layers
//! - **L0:** pure `bridge_auth_seams` via [`crate::harness`] (always on CI via `demo-e2e-l0`)
//! - **L1:** cw-orch Mock / multi-test via [`super::headstash::HeadstashSuite`] + real
//!   `ExecuteMsg::{SetBridgeCfg,RegisterAsset,SetReflectionSnapshot,BridgeMintNote}`
//! - **L2:** optional Tacit Anvil RPC handle (string URL; ops deferred)
//! - **L3:** same suite types against `Daemon` from `ict-rs-cw-orch`
//!   (`feature = "ict-daemon"`, binary `corridor_ict_funded` / `just demo-corridor-ict`)
//! - **L4:** optional mock LC tip / freeze flags (no Tier-0 claim)
//!
//! # L1 mock ZK posture
//! PR CI uses **policy + mock verify** for claim/mint paths (`BridgeCfg.mock_verify`).
//! Real H1 composite prove is a **separate track** (`just demo-h1` / nightly).
//! Do not label mock-ZK as Tier-0.

use std::path::PathBuf;
use std::string::String;

use cosmwasm_std::{Binary, Coin};
use cw_headstash::bridge::{AssetStatus, BridgeMintClaimPublic};
use cw_headstash::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use cw_headstash::tokenfactory::{HeadstashTokenObject, TokenStrategy};
use cw_headstash::wavs::generate_test_wavs_proof;
use cw_orch::prelude::*;

use super::headstash::{HeadstashDeployData, HeadstashSuite};
use crate::harness::{
    self, authorize_from_bridge_mint_fixture, build_happy_bridge_mint_fixture,
    build_policy_claim_fixture, load_bridge_mint_fixture, load_claim_fixture,
    load_or_build_happy_bridge_mint_fixture, BridgeL1World, BridgeMintFixtureDoc, ClaimFixture,
    L0Error,
};

/// Optional mock light-client hinge for L4 wiring tests.
///
/// Not a real Crosslink / reflection client. Labels must stay **mock / demo**
/// (compose matrix suite C9 — no Tier-0 while soundness depends on this).
#[derive(Clone, Debug, Default)]
pub struct LcMockConfig {
    /// Latest accepted foreign height / tip.
    pub tip_height: u64,
    /// Required confirmations before mint (Domain B / C spirit).
    pub confirmations_required: u64,
    /// When true, hinge rejects all mint gates (`E_LC_FROZEN`).
    pub frozen: bool,
    /// Optional max residual lag after K (aligns with `ReflectionSnapshot::max_lc_lag`).
    pub max_lc_lag: Option<u64>,
}

/// Deploy / runtime options for the compose suite.
#[derive(Clone, Debug)]
pub struct PrivateBridgeDeployData {
    /// Underlying headstash + manifold deploy payload.
    pub headstash: HeadstashDeployData,
    /// Directory of claim / mint-packet / swap JSON fixtures (DEMO-PATH + Domain B).
    pub fixtures_dir: Option<PathBuf>,
    /// Host-mapped Anvil / Tacit RPC (e.g. `http://127.0.0.1:8545`).
    pub tacit_rpc: Option<String>,
    /// Mock LC hinge; `None` skips L4 gates (claim-only paths).
    pub lc_mock: Option<LcMockConfig>,
    /// Optional post-mint note persist (`notes_base` or local data_dir).
    pub notes_persist: Option<harness::NotesPersistConfig>,
}

/// Compose suite: Headstash deploy surface + optional Tacit RPC + LC mock.
///
/// Bridge mint + claim route through **the same** `cw-headstash` code_id
/// (CLARITY). No freestanding `cw-bridge-mint` product.
pub struct PrivateBridgeSuite<Chain: ZkCwEnv> {
    pub headstash: HeadstashSuite<Chain>,
    pub tacit_rpc: Option<String>,
    pub fixtures_dir: Option<PathBuf>,
    pub lc_mock: Option<LcMockConfig>,
    /// When set: client encrypts SEAM and writes notes after mint (product loop).
    pub notes_persist: Option<harness::NotesPersistConfig>,
}

fn l0_to_cw(e: L0Error) -> CwOrchError {
    CwOrchError::StdErr(e.0)
}

fn err_contains(err: &CwOrchError, needles: &[&str]) -> bool {
    let s = err.to_string().to_lowercase();
    needles.iter().any(|n| s.contains(&n.to_lowercase()))
}

impl<Chain: ZkCwEnv> PrivateBridgeSuite<Chain> {
    /// Un-deployed suite from a chain handle (L1 Mock or Daemon).
    pub fn new(chain: Chain) -> Self
    where
        Chain: Clone,
    {
        Self {
            headstash: HeadstashSuite::new(chain),
            tacit_rpc: None,
            fixtures_dir: None,
            lc_mock: None,
            notes_persist: None,
        }
    }

    pub fn with_tacit_rpc(mut self, rpc: impl Into<String>) -> Self {
        self.tacit_rpc = Some(rpc.into());
        self
    }

    pub fn with_fixtures_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.fixtures_dir = Some(dir.into());
        self
    }

    pub fn with_lc_mock(mut self, cfg: LcMockConfig) -> Self {
        self.lc_mock = Some(cfg);
        self
    }

    /// Wire post-mint private note persistence (hash-market SSOT client side).
    pub fn with_notes_persist(mut self, cfg: harness::NotesPersistConfig) -> Self {
        self.notes_persist = Some(cfg);
        self
    }

    // -------------------------------------------------------------------------
    // L0 pure re-exports (no CW bridge mint msg required)
    // -------------------------------------------------------------------------

    /// E2E-01 min-layer: policy bridge mint happy via `authorize_bridge_mint`.
    pub fn assert_policy_bridge_mint_happy(&self) -> Result<(), CwOrchError> {
        harness::assert_policy_bridge_mint_happy()
            .map(|_| ())
            .map_err(l0_to_cw)
    }

    /// E2E-03 / H-1: spent-only ν never mints.
    pub fn assert_h1_spent_only_reject(&self) -> Result<(), CwOrchError> {
        harness::assert_h1_spent_only_reject().map_err(l0_to_cw)
    }

    /// Alias for E2E-03 naming in ROUND1 sketch.
    pub fn assert_ordinary_spend_not_burn(&self) -> Result<(), CwOrchError> {
        self.assert_h1_spent_only_reject()
    }

    /// E2E-02: double mint same ν rejects.
    pub fn assert_double_mint_reject(&self) -> Result<(), CwOrchError> {
        harness::assert_double_mint_reject().map_err(l0_to_cw)
    }

    /// Thin compose: sketch → SEAM 382-byte layout (+ optional seam_note_out decode).
    pub fn compose_bridge_mint_to_seam(&self) -> Result<(), CwOrchError> {
        harness::compose_bridge_mint_to_seam_bytes()
            .map(|_| ())
            .map_err(l0_to_cw)
    }

    // -------------------------------------------------------------------------
    // Claim fixture (DEMO-PATH A2) — policy only; mock ZK on L1
    // -------------------------------------------------------------------------

    /// Load claim fixture JSON (DEMO-PATH A2) without prove.
    pub fn load_claim_fixture(
        &self,
        fixture_path: &std::path::Path,
    ) -> Result<ClaimFixture, CwOrchError> {
        load_claim_fixture(fixture_path).map_err(|e| CwOrchError::StdErr(e.to_string()))
    }

    /// Build synthetic policy fixture (Poseidon root + 168-byte instance layout).
    pub fn build_claim_fixture(&self, leaf_index: u64, value: u64) -> ClaimFixture {
        build_policy_claim_fixture(leaf_index, value)
    }

    /// E2E-08 policy path: load/build claim fixture and validate layout.
    ///
    /// Does **not** call real H1 prove. Full `ProcessHeadstash` submit remains
    /// separate (claim path ≠ bridge mint).
    pub fn load_claim_fixture_and_process(
        &self,
        fixture_path: &std::path::Path,
    ) -> Result<ClaimFixture, CwOrchError> {
        let fix = self.load_claim_fixture(fixture_path)?;
        let _mock = fix
            .mock_proof_bytes()
            .map_err(|e| CwOrchError::StdErr(e.to_string()))?;
        Ok(fix)
    }

    /// Load Domain B `bridge-mint-claim-public-v1` JSON (hex digests, mock LC).
    pub fn load_bridge_mint_fixture(
        &self,
        fixture_path: &std::path::Path,
    ) -> Result<BridgeMintFixtureDoc, CwOrchError> {
        load_bridge_mint_fixture(fixture_path).map_err(|e| CwOrchError::StdErr(e.to_string()))
    }

    /// Synthetic hinge-happy mint packet (matches golden labels / BridgeL1World).
    pub fn build_bridge_mint_fixture(&self) -> BridgeMintFixtureDoc {
        build_happy_bridge_mint_fixture()
    }

    /// E2E-01 L0(+L4 mock) from Domain B mint-packet fixture.
    ///
    /// Prefer [`Self::e2e_bridge_mint_happy`] for L1 CW multi-test. This path loads
    /// golden JSON (or synthetic degrade), validates claim_id / domain_binding,
    /// then pure `authorize_bridge_mint`.
    pub fn e2e_burn_to_mint_fixture(
        &self,
        fixture_path: &std::path::Path,
    ) -> Result<(), CwOrchError> {
        let doc = if fixture_path.is_file() {
            self.load_bridge_mint_fixture(fixture_path)?
        } else {
            load_or_build_happy_bridge_mint_fixture()
                .map_err(|e| CwOrchError::StdErr(e.to_string()))?
        };
        if let Some(cfg) = self.lc_mock.as_ref() {
            let inclusion = doc.snapshot.source_height;
            let now = cfg
                .tip_height
                .max(inclusion + cfg.confirmations_required);
            if !self.lc_mock_allows_mint(inclusion, now) {
                return Err(CwOrchError::StdErr(
                    "lc_mock_allows_mint rejected (frozen / immature / tip)".into(),
                ));
            }
        }
        authorize_from_bridge_mint_fixture(&doc)
            .map(|_| ())
            .map_err(|e| CwOrchError::StdErr(e.to_string()))
    }

    /// E2E-10/11: swap fixture (pure seams until CosmWasm DEX lands).
    pub fn apply_swap_fixture(&self, _fixture_path: &std::path::Path) -> Result<(), CwOrchError> {
        Err(CwOrchError::StdErr(
            "apply_swap_fixture: delegate L0 private_dex_seams via just demo-e2e-l0; CW DEX TBD"
                .into(),
        ))
    }

    /// DEMO CashApp→ZEC corridor W0–W7 L0 (Simulated backend). See
    /// [`crate::harness::cashapp_zec_corridor`].
    pub fn e2e_cashapp_zec_corridor_happy(&self) -> Result<harness::CorridorReceiptV0, CwOrchError> {
        let mut backend = harness::CorridorAssetBackend::simulated();
        let scenario = harness::CorridorScenario::default();
        let out = harness::run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario)
            .map_err(|e| CwOrchError::StdErr(e.to_string()))?;
        Ok(out.receipt)
    }

    /// E2E-13: oracle path must not mint balances (pure seams).
    pub fn assert_oracle_cannot_mint(&self) -> Result<(), CwOrchError> {
        Err(CwOrchError::StdErr(
            "assert_oracle_cannot_mint: run private_dex_seams via demo-e2e-l0 (E2E-13 L0)"
                .into(),
        ))
    }

    /// L4 helper: whether mock hinge would accept a burn at `inclusion_height`.
    pub fn lc_mock_allows_mint(&self, inclusion_height: u64, now_height: u64) -> bool {
        let Some(cfg) = self.lc_mock.as_ref() else {
            return true;
        };
        if cfg.frozen {
            return false;
        }
        if now_height < inclusion_height {
            return false;
        }
        let conf_ok = now_height.saturating_sub(inclusion_height) >= cfg.confirmations_required
            && inclusion_height <= cfg.tip_height;
        if !conf_ok {
            return false;
        }
        if let Some(max_lag) = cfg.max_lc_lag {
            let residual = now_height
                .saturating_sub(inclusion_height.saturating_add(cfg.confirmations_required));
            if residual > max_lag {
                return false;
            }
        }
        true
    }

    // -------------------------------------------------------------------------
    // L1 real execute on deployed `cw-headstash` (Mock / multi-test)
    // -------------------------------------------------------------------------

    /// Default instantiate payload for bridge L1 (ExistingFungible + real WAVS PoP).
    ///
    /// Caller must send ≥1 unit of `denom` with instantiate (or prefund contract).
    pub fn bridge_l1_instantiate_msg(denom: &str) -> InstantiateMsg {
        InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: Some("bridge-l1".into()),
            token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject::new(
                denom.into(),
            )),
            wavs: generate_test_wavs_proof(1),
            circuit_id: None,
        }
    }

    /// Upload **only** `cw-headstash` (no manifold / no circuit) and instantiate.
    ///
    /// Preferred L1 path for bridge mint: avoids circuit keygen and manifold.
    /// Still one mint product (`cw-headstash` code_id) per CLARITY.
    ///
    /// `funds` must include the ExistingFungible denom (e.g. `uterp`).
    pub fn upload_and_instantiate_mint(&mut self, funds: &[Coin]) -> Result<(), CwOrchError> {
        self.headstash.headstash.upload()?;
        let msg = Self::bridge_l1_instantiate_msg(
            funds
                .first()
                .map(|c| c.denom.as_str())
                .unwrap_or("uterp"),
        );
        self.headstash
            .headstash
            .instantiate(&msg, None, funds)?;
        Ok(())
    }

    /// Owner: `SetBridgeCfg` + `RegisterAsset` + `SetReflectionSnapshot` from happy world.
    pub fn configure_bridge_happy(&self) -> Result<BridgeL1World, CwOrchError> {
        let world = BridgeL1World::happy();
        self.configure_bridge(&world, true)?;
        Ok(world)
    }

    /// Configure + mint from Domain B JSON / synthetic fixture (Binary convert).
    pub fn e2e_bridge_mint_from_fixture(
        &self,
        fixture_path: &std::path::Path,
    ) -> Result<(), CwOrchError> {
        let doc = if fixture_path.is_file() {
            self.load_bridge_mint_fixture(fixture_path)?
        } else {
            load_or_build_happy_bridge_mint_fixture()
                .map_err(|e| CwOrchError::StdErr(e.to_string()))?
        };
        let world = BridgeL1World::from_fixture_doc(&doc)
            .map_err(|e| CwOrchError::StdErr(e))?;
        self.configure_bridge(&world, true)?;
        self.execute_bridge_mint(world.claim.clone(), world.proof.clone())?;
        let minted = self.query_is_bridge_minted(world.nullifier_bin())?;
        if !minted {
            return Err(CwOrchError::StdErr(
                "e2e_bridge_mint_from_fixture: not minted after execute".into(),
            ));
        }
        Ok(())
    }

    /// Owner configure corridor. When `register_asset` is false, asset is omitted
    /// (for unregistered-asset reject cases).
    pub fn configure_bridge(
        &self,
        world: &BridgeL1World,
        register_asset: bool,
    ) -> Result<(), CwOrchError> {
        let hs = &self.headstash.headstash;
        // Prefer raw ExecuteMsg (ExecuteFns also generated; raw is explicit for optional fields).
        hs.execute(
            &ExecuteMsg::SetBridgeCfg {
                cfg: world.cfg.clone(),
            },
            &[],
        )?;
        hs.execute(
            &ExecuteMsg::SetReflectionSnapshot {
                snapshot: world.snapshot.clone(),
            },
            &[],
        )?;
        if register_asset {
            hs.execute(
                &ExecuteMsg::RegisterAsset {
                    asset_id: world.asset.asset_id.clone(),
                    local_denom: world.asset.local_denom.clone(),
                    origin: world.asset.origin.clone(),
                    status: Some(AssetStatus::Active),
                },
                &[],
            )?;
        }
        Ok(())
    }

    /// Execute `BridgeMintNote` with claim + mock proof.
    pub fn execute_bridge_mint(
        &self,
        claim: BridgeMintClaimPublic,
        proof: Binary,
    ) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.headstash.headstash.execute(
            &ExecuteMsg::BridgeMintNote { claim, proof },
            &[],
        )
    }

    /// Query whether bridge burn ν has minted.
    pub fn query_is_bridge_minted(&self, nullifier: Binary) -> Result<bool, CwOrchError> {
        self.headstash
            .headstash
            .query(&QueryMsg::IsBridgeMinted { nullifier })
    }

    /// Option D: register asset (if missing) then `BridgeEgressBurn` + optional spent query.
    ///
    /// Lab: mock proof `b"corridor-ict-mock-egress-proof-v0"` under `mock_verify`.
    pub fn execute_bridge_egress_burn(
        &self,
        statement: cw_headstash::egress::EgressBurnStatement,
        proof: Binary,
    ) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        // Ensure asset is registered when registry is non-empty (egress UnmappedAsset gate).
        let _ = self.headstash.headstash.execute(
            &ExecuteMsg::RegisterAsset {
                asset_id: statement.asset_id.clone(),
                local_denom: "uzec".into(),
                origin: Some("registry:lab-sim-zec".into()),
                status: Some(AssetStatus::Active),
            },
            &[],
        );
        self.headstash.headstash.execute(
            &ExecuteMsg::BridgeEgressBurn { statement, proof },
            &[],
        )
    }

    /// Query whether egress ν is spent (`IsEgressSpent`).
    pub fn query_is_egress_spent(&self, nullifier: Binary) -> Result<bool, CwOrchError> {
        self.headstash
            .headstash
            .query(&QueryMsg::IsEgressSpent { nullifier })
    }

    /// E2E-01 L1: configure happy corridor + one-shot bridge mint (mock verify).
    ///
    /// Requires prior [`Self::upload_and_instantiate_mint`] (or full `deploy_on`).
    pub fn e2e_bridge_mint_happy(&self) -> Result<(), CwOrchError> {
        let world = self.configure_bridge_happy()?;
        let res = self.execute_bridge_mint(world.claim.clone(), world.proof.clone())?;
        // Prefer attribute checks when response exposes them; string form is portable.
        let s = format!("{res:?}");
        if !s.contains("bridge_mint_note") && !s.to_lowercase().contains("success") {
            // Still accept if query proves mint — multi-test may wrap attrs differently.
            let minted = self.query_is_bridge_minted(world.nullifier_bin())?;
            if !minted {
                return Err(CwOrchError::StdErr(format!(
                    "e2e_bridge_mint_happy: mint not recorded; resp={s}"
                )));
            }
        } else {
            let minted = self.query_is_bridge_minted(world.nullifier_bin())?;
            if !minted {
                return Err(CwOrchError::StdErr(
                    "e2e_bridge_mint_happy: BRIDGE_MINTED missing after execute".into(),
                ));
            }
        }
        Ok(())
    }

    /// L2 product loop: L1 Mock `BridgeMintNote` → client encrypt → put → get → decrypt.
    ///
    /// Requires [`Self::with_notes_persist`] and feature `l0-seams`. Cleartext SEAM is
    /// rebuilt on the **client** from the same pure hinge (rcm never on chain).
    ///
    /// Backend: local HeadstashStore-shaped dir when `notes_base` is None; live
    /// HTTP when `notes_base` is set (needs feature `note-http`).
    #[cfg(feature = "l0-seams")]
    pub fn e2e_l2_bridge_mint_note_persist(
        &self,
    ) -> Result<harness::NotePersistReceipt, CwOrchError> {
        let cfg = self.notes_persist.as_ref().ok_or_else(|| {
            CwOrchError::StdErr(
                "e2e_l2_bridge_mint_note_persist: call with_notes_persist first".into(),
            )
        })?;

        // 1) Chain commit (Mock)
        self.e2e_bridge_mint_happy()?;

        // 2) Client holds plaintext SEAM (same happy hinge as mint world)
        let sketch = harness::compose_bridge_mint_to_seam_bytes().map_err(l0_to_cw)?;
        let note = terp_seams::dex::note::SeamNoteOutV0::from_bytes(&sketch.to_seam_bytes())
            .map_err(|e| CwOrchError::StdErr(format!("seam decode: {e:?}")))?;

        // Prefer contract address as hs_id when config still has a season placeholder.
        let mut cfg = cfg.clone();
        if cfg.hs_id == "season-1" || cfg.hs_id.is_empty() {
            if let Ok(addr) = self.headstash.headstash.address() {
                cfg.hs_id = addr.to_string();
            }
        }

        // 3–4) put_note_after_mint + get/decrypt (product call site)
        harness::l2_smoke_put_get_decrypt(&note, &cfg).map_err(l0_to_cw)
    }

    /// E2E-02 L1: second `BridgeMintNote` same ν → AlreadyMinted.
    pub fn e2e_bridge_double_mint_reject(&self) -> Result<(), CwOrchError> {
        let world = self.configure_bridge_happy()?;
        self.execute_bridge_mint(world.claim.clone(), world.proof.clone())?;
        match self.execute_bridge_mint(world.claim.clone(), world.proof.clone()) {
            Ok(_) => Err(CwOrchError::StdErr(
                "e2e_bridge_double_mint_reject: expected second mint to fail".into(),
            )),
            Err(e) if err_contains(&e, &["already minted", "AlreadyMinted"]) => Ok(()),
            Err(e) => Err(CwOrchError::StdErr(format!(
                "e2e_bridge_double_mint_reject: unexpected err: {e}"
            ))),
        }
    }

    /// E2E-06 L1: corridor without RegisterAsset → UnmappedAsset.
    pub fn e2e_unregistered_asset_reject(&self) -> Result<(), CwOrchError> {
        let world = BridgeL1World::happy();
        self.configure_bridge(&world, false)?;
        match self.execute_bridge_mint(world.claim, world.proof) {
            Ok(_) => Err(CwOrchError::StdErr(
                "e2e_unregistered_asset_reject: expected mint to fail".into(),
            )),
            Err(e) if err_contains(&e, &["unregistered", "unmapped", "UnmappedAsset"]) => Ok(()),
            Err(e) => Err(CwOrchError::StdErr(format!(
                "e2e_unregistered_asset_reject: unexpected err: {e}"
            ))),
        }
    }

    /// E2E-03 L1: spent-only ν → NotInBurnSet (H-1).
    pub fn e2e_h1_spent_only_reject(&self) -> Result<(), CwOrchError> {
        let world = BridgeL1World::happy().with_spent_only();
        self.configure_bridge(&world, true)?;
        match self.execute_bridge_mint(world.claim, world.proof) {
            Ok(_) => Err(CwOrchError::StdErr(
                "e2e_h1_spent_only_reject: expected H-1 reject".into(),
            )),
            Err(e) if err_contains(&e, &["H-1", "burn set", "NotInBurnSet"]) => Ok(()),
            Err(e) => Err(CwOrchError::StdErr(format!(
                "e2e_h1_spent_only_reject: unexpected err: {e}"
            ))),
        }
    }
}

impl<Chain: ZkCwEnv + CircuitUploadable> Deploy<Chain> for PrivateBridgeSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = PrivateBridgeDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let headstash = HeadstashSuite::store_on(chain)?;
        Ok(Self {
            headstash,
            tacit_rpc: None,
            fixtures_dir: None,
            lc_mock: None,
            notes_persist: None,
        })
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        self.headstash.get_contracts_mut()
    }

    fn load_from(_chain: Chain) -> Result<Self, Self::Error> {
        Err(CwOrchError::StdErr(
            "PrivateBridgeSuite::load_from — not implemented (use new/deploy_on)".into(),
        ))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let headstash = HeadstashSuite::deploy_on(chain, data.headstash)?;
        Ok(Self {
            headstash,
            tacit_rpc: data.tacit_rpc,
            fixtures_dir: data.fixtures_dir,
            lc_mock: data.lc_mock,
            notes_persist: data.notes_persist,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::coins;
    use cw_orch::mock::Mock;

    const DENOM: &str = "uterp";

    fn funded_mock() -> Mock {
        let chain = Mock::new("sender");
        let sender = chain.sender_addr();
        chain
            .set_balance(&sender, coins(1_000_000, DENOM))
            .expect("fund sender");
        chain
    }

    fn deployed_suite() -> PrivateBridgeSuite<Mock> {
        let chain = funded_mock();
        let mut suite = PrivateBridgeSuite::new(chain);
        suite
            .upload_and_instantiate_mint(&coins(10_000, DENOM))
            .expect("upload+instantiate cw-headstash");
        suite
    }

    #[test]
    fn l0_suite_methods_on_mock_chain() {
        let chain = Mock::new("sender");
        let suite = PrivateBridgeSuite::new(chain);
        suite
            .assert_policy_bridge_mint_happy()
            .expect("E2E-01 L0");
        suite.assert_h1_spent_only_reject().expect("E2E-03 L0");
        suite.assert_double_mint_reject().expect("E2E-02 L0");
        suite.compose_bridge_mint_to_seam().expect("compose L0");
    }

    #[test]
    fn cashapp_zec_corridor_happy_via_suite() {
        let chain = Mock::new("sender");
        let suite = PrivateBridgeSuite::new(chain);
        let receipt = suite
            .e2e_cashapp_zec_corridor_happy()
            .expect("corridor W0–W7 L0");
        assert_eq!(receipt.status, "complete");
        assert_eq!(receipt.asset_backend, "simulated");
        assert_eq!(receipt.corridor_id, "cashapp-btc-zec-v0");
    }

    #[test]
    fn claim_fixture_policy_build() {
        let chain = Mock::new("sender");
        let suite = PrivateBridgeSuite::new(chain);
        let fix = suite.build_claim_fixture(3, 1_000_000);
        fix.validate_policy().expect("policy");
        assert_eq!(fix.instance_bytes_len, 200);
    }

    #[test]
    fn bridge_mint_fixture_e2e01_from_synthetic() {
        let chain = Mock::new("sender");
        let suite = PrivateBridgeSuite::new(chain);
        let path = std::path::Path::new("/nonexistent-bridge-mint.json");
        suite
            .e2e_burn_to_mint_fixture(path)
            .expect("E2E-01 from synthetic/golden");
        let fix = suite.build_bridge_mint_fixture();
        fix.validate_policy().expect("bridge mint policy");
        assert!(fix.claim.in_burn_set);
    }

    #[test]
    fn lc_mock_lag_gate() {
        let chain = Mock::new("sender");
        let suite = PrivateBridgeSuite::new(chain).with_lc_mock(LcMockConfig {
            tip_height: 200,
            confirmations_required: 6,
            frozen: false,
            max_lc_lag: Some(2),
        });
        assert!(suite.lc_mock_allows_mint(100, 106)); // residual 0
        assert!(!suite.lc_mock_allows_mint(100, 109)); // residual 3 > 2
        assert!(!suite.lc_mock_allows_mint(100, 105)); // immature
    }

    #[test]
    fn l1_e2e_bridge_mint_happy() {
        let suite = deployed_suite();
        suite.e2e_bridge_mint_happy().expect("E2E-01 L1");
    }

    #[test]
    fn l1_e2e_bridge_double_mint_reject() {
        let suite = deployed_suite();
        suite
            .e2e_bridge_double_mint_reject()
            .expect("E2E-02 L1");
    }

    #[test]
    fn l1_e2e_unregistered_asset_reject() {
        let suite = deployed_suite();
        suite
            .e2e_unregistered_asset_reject()
            .expect("E2E-06 L1");
    }

    #[test]
    fn l1_e2e_h1_spent_only_reject() {
        let suite = deployed_suite();
        suite.e2e_h1_spent_only_reject().expect("E2E-03 L1");
    }

    /// L2: Mock BridgeMintNote → put_note_after_mint → get → decrypt (local store).
    #[cfg(feature = "l0-seams")]
    #[test]
    fn l2_e2e_bridge_mint_note_persist_local() {
        use harness::NotesPersistConfig;
        use std::time::{SystemTime, UNIX_EPOCH};

        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pb-l2-notes-{n}"));
        std::fs::create_dir_all(&dir).unwrap();

        let suite = deployed_suite().with_notes_persist(NotesPersistConfig::local_season(
            "season-1",
            [0x11u8; 32],
            dir.clone(),
        ));
        let receipt = suite
            .e2e_l2_bridge_mint_note_persist()
            .expect("L2 mint→persist→decrypt");
        assert!(receipt.addr.starts_with("cm."));
        assert!(
            dir.join("notes")
                .join(&receipt.hs_id)
                .join(format!("{}.json", receipt.addr))
                .is_file(),
            "expected note file under {}",
            dir.display()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
