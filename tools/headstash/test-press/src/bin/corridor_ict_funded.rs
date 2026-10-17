//! L3 funded path: ict-rs fresh Terp → cw-orch Daemon → BridgeMintNote on chain
//! (+ optional G3 `cw-private-dex` SettleSwap when `CORRIDOR_CHAIN_SETTLE=1`)
//! (+ optional Option D egress when `CORRIDOR_ZEC_EGRESS_D=1` after settle).
//!
//! Profile: **`ict_local_funded`** (not mainnet money; not lab_simulated).
//!
//! ## Flow
//! 1. Spin fresh Terp via ict-rs Docker (`terpnetwork/terp-core:local-zk` preferred)
//! 2. Build cw-orch Daemon from ict-rs-cw-orch (known faucet mnemonic)
//! 3. Upload + instantiate **cw-headstash only** (no manifold / circuit keygen)
//! 4. `SetBridgeCfg` (`mock_verify=true` unless env says otherwise) + snapshot + asset
//! 5. Execute `BridgeMintNote` — **fail closed** if mint not recorded
//! 6. Write `MintEvidenceV0`
//! 7. When `CORRIDOR_CHAIN_SETTLE=1`: upload private-dex → CreatePool → SettleSwap →
//!    queries + `SettleReceiptV0` (fail-closed; mock_verify lab labeled)
//! 8. When `CORRIDOR_ZEC_EGRESS_D=1`: pure (labeled) egress burn → lab ZEC pay → dual receipts
//! 9. Pure swap film (optional residual when settle on)
//!
//! ## Env
//! | Var | Default | Meaning |
//! |-----|---------|---------|
//! | `CORRIDOR_ICT_IMAGE` | `terpnetwork/terp-core` | Docker repo |
//! | `CORRIDOR_ICT_IMAGE_TAG` | `local-zk` | Docker tag |
//! | `CORRIDOR_ICT_MNEMONIC` | abandon…about | Deployer/faucet mnemonic |
//! | `CORRIDOR_MOCK_VERIFY` | `true` | Bridge + dex mock_verify on funded deploy |
//! | `CORRIDOR_CHAIN_SETTLE` | off | Deploy dex + SettleSwap after mint (G3) |
//! | `CORRIDOR_ZEC_EGRESS_D` | off | Option D: burn → ZEC release after settle (requires settle) |
//! | `CORRIDOR_ZEC_RELEASE` | profile-default | `threshold_escrow` (product) or `lab_inventory` (residual) |
//! | `CORRIDOR_PROFILE` | (empty) | `omni_production_shaped` → product green bar + threshold default |
//! | `CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL` | off | Explicit residual only — product green FAILS without this |
//! | `CORRIDOR_SKIP_SWAP_FILM` | off | Skip pure W0–W7 (does **not** skip chain settle/egress) |
//! | `CORRIDOR_RUN_SWAP_FILM` | off | When settle on, also run pure film residual |
//! | `CORRIDOR_MINT_EVIDENCE_PATH` | `/tmp/corridor-mint-evidence.json` | |
//! | `CORRIDOR_SETTLE_RECEIPT_PATH` | `/tmp/corridor-settle-receipt.json` | |
//! | `CORRIDOR_EGRESS_BURN_EVIDENCE_PATH` | `/tmp/corridor-egress-burn-evidence.json` | |
//! | `CORRIDOR_ZEC_EGRESS_RECEIPT_PATH` | `/tmp/corridor-zec-egress-receipt.json` | |
//! | `CORRIDOR_PROFILE` | (none) | `omni_production_shaped` forces bridged LP seed |
//! | `CORRIDOR_POOL_SEED` | `magic_lab` | `bridged_lp` forbids CreatePool magic reserves alone |
//! | `CORRIDOR_LP_SEED_RECEIPT_PATH` | `/tmp/corridor-lp-seed-receipt.json` | Phase 3 receipt |
//! | `CORRIDOR_DUAL_HOME_PREP_PATH` | `$CORRIDOR_OMNI_RUN_DIR/dual-home-prep.json` | Multi-net dual-home artifact |
//! | `CORRIDOR_OMNI_RUN_DIR` | (none) | Omni receipt pack root (GUIDE §12) |
//! | `CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN` | off | Fail closed if dual-home/LP underfunded |
//! | `KEEP_CHAIN` | off | Leave Docker containers running |
//!
//! ## Run
//! ```sh
//! cd crates/headstash
//! just demo-corridor-ict-egress-d
//! ```

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cosmwasm_std::coins;
use cw_orch::prelude::*;
use ict_rs::chain::cosmos::CosmosChain;
use ict_rs::chain::{Chain, ChainConfig, FaucetConfig, TestContext};
use ict_rs::runtime::{DockerConfig, DockerImage, IctRuntime};
use ict_rs::spec::builtin_chain_config;
use ict_rs_cw_orch::daemon_builder_from_chain;
use tokio::runtime::Runtime;
use zk_test_press::harness::{
    self, allow_lab_inventory_residual, apply_pure_egress_burn_labeled, assert_dest_binding_equal,
    assert_lab_degrade_labels, assert_mint_handoff_continuous, assert_product_release_mode,
    bootstrap_omni_liquidity, build_swap_spend_handoff_from_mint_with_reserves,
    confirm_open_at_sealed_dest_with_cfg, corridor_allow_happy_fixture, corridor_pool_seed_policy,
    corridor_profile, cw_egress_statement_from_opening, dual_home_prep_path,
    egress_burn_evidence_path, is_placeholder_owner_binding_hex, is_product_profile,
    lab_mock_egress_proof, lab_pay_zec_after_burn, load_dual_home_prep, lp_mint_log_line,
    lp_seed_receipt_path, mint_evidence_from_claim_fields, mint_evidence_path,
    peek_last_threshold_auth_meta, product_celebrate_forbidden, refuse_magic_pool_if_bridged_policy,
    reject_funder_only_as_product, require_bridged_lp_seed, resolve_pool_reserves,
    sealed_dest_for_option_d, settle_opening_from_handoff, settle_receipt_path, try_claim_from_env,
    wallet_rpc_available, zec_egress_d_enabled, zec_egress_receipt_path, zec_release_policy,
    BridgeL1World, BURN_SURFACE_CW_BRIDGE_EGRESS, BURN_SURFACE_PURE_RECORD_LAB,
    COMMITTEE_CRYPTO_LAB_T_OF_N, CorridorLabMintPolicy, EgressBurnEvidenceJson,
    EgressBurnPublicJson, ESCROW_SIGN_PATH_P_LAB, MintEvidenceV0, MODE_THRESHOLD_ESCROW_RELEASE,
    MODE_THRESHOLD_ESCROW_RELEASE_SIMULATED, OmniLiquidityBootstrap, ProofModeLabel,
    SettleReceiptV0, ZakuraLocalConfig, ZecEgressReceiptJson, ZecReleasePolicy,
    PROOF_MODE_MOCK_VERIFY_LAB,
};
use zk_test_press::harness::swap_statement_cw::{LAB_R_IN, LAB_R_OUT};
#[cfg(feature = "l0-seams")]
use zk_test_press::harness::{
    put_and_recover_after_mint, seam_note_from_mint_openings,
};
use zk_test_press::suites::private_bridge::PrivateBridgeSuite;
use zk_test_press::suites::private_dex::PrivateDexSuite;

/// Well-known BIP39 test mnemonic (same spirit as ict-rs akash TEST_MNEMONIC).
const DEFAULT_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

const DENOM: &str = "uterp";

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_truthy(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

fn resolve_wasm_hint() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    let artifacts = dir.join("artifacts").join("cw_headstash.wasm");
    if artifacts.exists() {
        return artifacts;
    }
    let alt = dir.join("contracts/cw-headstash/artifacts/zk_headstash.wasm");
    if alt.exists() {
        return alt;
    }
    artifacts
}

fn resolve_private_dex_wasm_hint() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    let artifacts = dir.join("artifacts").join("cw_private_dex.wasm");
    if artifacts.exists() {
        return artifacts;
    }
    let alt = dir
        .join("contracts/cw-private-dex/artifacts")
        .join("cw_private_dex.wasm");
    if alt.exists() {
        return alt;
    }
    artifacts
}

fn terp_funded_config() -> Result<ChainConfig, Box<dyn std::error::Error>> {
    let mut cfg = builtin_chain_config("terp")?;
    let repo = env_or("CORRIDOR_ICT_IMAGE", "terpnetwork/terp-core");
    let tag = env_or("CORRIDOR_ICT_IMAGE_TAG", "local-zk");
    cfg.images = vec![DockerImage {
        repository: repo,
        version: tag,
        uid_gid: None,
    }];
    cfg.gas_prices = format!("0{DENOM}");
    cfg.block_time = "1s".to_string();
    let mnemonic = env_or("CORRIDOR_ICT_MNEMONIC", DEFAULT_MNEMONIC);
    cfg.faucet = Some(FaucetConfig {
        key_name: "deployer".into(),
        port: 5000,
        start_cmd: vec![],
        env: vec![],
        mnemonic: Some(mnemonic),
        coins: Some(format!("100000000000{DENOM}")),
    });
    Ok(cfg)
}

fn mock_verify_flag() -> bool {
    match std::env::var("CORRIDOR_MOCK_VERIFY") {
        Ok(v) => !matches!(v.as_str(), "0" | "false" | "FALSE" | "no" | "NO"),
        Err(_) => true,
    }
}

fn chain_settle_enabled() -> bool {
    env_truthy("CORRIDOR_CHAIN_SETTLE")
}

fn zec_egress_d_gate() -> bool {
    zec_egress_d_enabled()
}

fn wasm_bytes_contain(path: &std::path::Path, needle: &[u8]) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => bytes.windows(needle.len()).any(|w| w == needle),
        Err(_) => false,
    }
}

fn require_wasm_surface(path: &std::path::Path, labels: &[&[u8]], what: &str) -> Result<(), String> {
    if labels.iter().any(|n| wasm_bytes_contain(path, n)) {
        return Ok(());
    }
    Err(format!(
        "FAIL closed: {what} surface missing in {} — rebuild wasm (FORCE_WASM_REBUILD=1 / prepare-corridor-ict-wasm-settle)",
        path.display()
    ))
}

/// Pin claim dest fields from `CORRIDOR_DEST_OWNER_BINDING` (G4 continuous seal).
/// Required for happy-fixture residual so Option D does not invent a divergent seal.
fn pin_claim_dest_from_env(world: &mut BridgeL1World) -> Result<(), String> {
    let Ok(hex_s) = std::env::var("CORRIDOR_DEST_OWNER_BINDING") else {
        return Ok(());
    };
    let h = hex_s.trim().to_ascii_lowercase();
    if h.is_empty() {
        return Ok(());
    }
    if is_placeholder_owner_binding_hex(&h) {
        return Err("CORRIDOR_DEST_OWNER_BINDING is placeholder — refuse mint dest pin".into());
    }
    let raw = hex::decode(h.strip_prefix("0x").unwrap_or(&h))
        .map_err(|e| format!("CORRIDOR_DEST_OWNER_BINDING hex: {e}"))?;
    if raw.len() != 32 {
        return Err(format!(
            "CORRIDOR_DEST_OWNER_BINDING width {} want 32 bytes",
            raw.len()
        ));
    }
    let bin = cosmwasm_std::Binary::from(raw);
    world.claim.dest_commitment = bin.clone();
    world.claim.burn_dest_commitment = bin;
    println!("  dest_commitment pinned from CORRIDOR_DEST_OWNER_BINDING={h}");
    Ok(())
}

/// Resolve L1 mint world: **deposit-backed claim default** when env complete;
/// hinge happy fixture only if `CORRIDOR_ALLOW_HAPPY_FIXTURE=1`.
///
/// Fail-closed:
/// - incomplete deposit when intent set without happy flag
/// - `CORRIDOR_REQUIRE_DEPOSIT_CLAIM=1` without deposit claim
/// - partial deposit fields without complete set
fn resolve_bridge_world(
    suite: &PrivateBridgeSuite<Daemon>,
) -> Result<(BridgeL1World, &'static str), Box<dyn std::error::Error>> {
    let policy = CorridorLabMintPolicy::corridor_lab_default();
    let allow_happy = corridor_allow_happy_fixture();
    let require_deposit = env_truthy("CORRIDOR_REQUIRE_DEPOSIT_CLAIM");

    match try_claim_from_env(&policy) {
        Ok(Some(claim)) => {
            println!(
                "  mint_source=deposit_claim value={} ν={}… dest={}… lab_mock_membership={}",
                claim.value_u64,
                hex::encode(&claim.nullifier[..8]),
                hex::encode(&claim.dest_commitment[..8]),
                policy.lab_mock_membership
            );
            if let Ok(env_dest) = std::env::var("CORRIDOR_DEST_OWNER_BINDING") {
                let env_dest = env_dest.trim().to_ascii_lowercase();
                let claim_dest = hex::encode(claim.dest_commitment);
                if !env_dest.is_empty() {
                    assert_dest_binding_equal(&env_dest, &claim_dest, "claim.dest_commitment")
                        .map_err(|e| format!("FAIL closed: watch/env dest ≠ claim dest: {e}"))?;
                }
            }
            let mut world = BridgeL1World::from_deposit_claim(&claim, &policy);
            if !mock_verify_flag() {
                world.cfg.mock_verify = false;
            } else {
                world.cfg.mock_verify = true;
            }
            suite
                .configure_bridge(&world, true)
                .map_err(|e| format!("configure_bridge deposit claim: {e}"))?;
            println!(
                "  BridgeCfg.mock_verify={} (deposit-backed; lab membership labeled)",
                world.cfg.mock_verify
            );
            Ok((world, "deposit_claim"))
        }
        Ok(None) => {
            let intent_set = std::env::var("CORRIDOR_INTENT_ID")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .is_some();
            let partial = [
                "CORRIDOR_DEPOSIT_TXID",
                "CORRIDOR_DEPOSIT_AMOUNT_SATS",
                "CORRIDOR_DEPOSIT_VOUT",
            ]
            .iter()
            .any(|k| {
                std::env::var(k)
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .is_some()
            });

            if require_deposit {
                return Err(
                    "FAIL closed: CORRIDOR_REQUIRE_DEPOSIT_CLAIM=1 but deposit claim env incomplete \
                     (need CORRIDOR_INTENT_ID + CORRIDOR_DEPOSIT_TXID + CORRIDOR_DEPOSIT_AMOUNT_SATS \
                     + CORRIDOR_DEPOSIT_VOUT + CORRIDOR_DEST_OWNER_BINDING)"
                        .into(),
                );
            }
            if intent_set && !allow_happy {
                return Err(
                    "FAIL closed: CORRIDOR_INTENT_ID set but deposit claim incomplete — \
                     export claim-inputs (txid/vout/amount/dest) or set CORRIDOR_ALLOW_HAPPY_FIXTURE=1 for residual"
                        .into(),
                );
            }
            if partial && !allow_happy {
                return Err(
                    "FAIL closed: partial deposit env without complete claim fields — refuse silent happy fixture"
                        .into(),
                );
            }
            if !allow_happy {
                return Err(
                    "FAIL closed: no deposit-backed claim env — happy hinge only when \
                     CORRIDOR_ALLOW_HAPPY_FIXTURE=1 (mint-only residual)"
                        .into(),
                );
            }

            println!(
                "  mint_source=happy_fixture (CORRIDOR_ALLOW_HAPPY_FIXTURE=1) labeled residual"
            );
            // Build happy world, pin G4 dest *before* chain configure (avoid double RegisterAsset).
            let mut world = BridgeL1World::happy();
            pin_claim_dest_from_env(&mut world)?;
            if !mock_verify_flag() {
                world.cfg.mock_verify = false;
                println!("  WARN: mock_verify=false — real proof required; may fail closed");
            } else {
                world.cfg.mock_verify = true;
            }
            suite
                .configure_bridge(&world, true)
                .map_err(|e| format!("configure_bridge happy residual: {e}"))?;
            if std::env::var("CORRIDOR_DEST_OWNER_BINDING")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .is_some()
            {
                println!(
                    "  BridgeCfg.mock_verify={} (happy residual; dest pinned from env)",
                    world.cfg.mock_verify
                );
            } else {
                println!(
                    "  BridgeCfg.mock_verify={} (labeled; not Tier-0)",
                    world.cfg.mock_verify
                );
            }
            Ok((world, "happy_fixture"))
        }
        Err(e) => Err(format!("FAIL closed: deposit claim builder: {e}").into()),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Err(e) = run_sync() {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}

fn run_sync() -> Result<(), Box<dyn std::error::Error>> {
    println!("== corridor_ict_funded (profile: ict_local_funded) ==");
    println!("  mock_verify={}", mock_verify_flag());
    println!("  CORRIDOR_CHAIN_SETTLE={}", chain_settle_enabled());
    println!("  CORRIDOR_ZEC_EGRESS_D={}", zec_egress_d_gate());
    println!(
        "  CORRIDOR_PROFILE={} pool_seed={} bridged_lp_required={}",
        corridor_profile(),
        corridor_pool_seed_policy().as_str(),
        require_bridged_lp_seed()
    );
    if env_truthy("CORRIDOR_ALLOW_SETTLE_WITHOUT_MINT") {
        return Err(
            "FAIL closed: CORRIDOR_ALLOW_SETTLE_WITHOUT_MINT is forbidden on funded profile"
                .into(),
        );
    }
    if zec_egress_d_gate() && !chain_settle_enabled() {
        return Err(
            "FAIL closed: CORRIDOR_ZEC_EGRESS_D=1 requires CORRIDOR_CHAIN_SETTLE=1 (no burn without settle)"
                .into(),
        );
    }

    let wasm_hint = resolve_wasm_hint();
    println!("  wasm_hint={}", wasm_hint.display());
    if !wasm_hint.exists() {
        return Err(format!(
            "FAIL closed: cw_headstash.wasm not found (looked at {}).\n\
             Prepare: cd crates/headstash && just prepare-corridor-ict-wasm\n\
             or copy optimized wasm to crates/headstash/artifacts/cw_headstash.wasm",
            wasm_hint.display()
        )
        .into());
    }

    // Wasm surface gate before Daemon (fail closed).
    require_wasm_surface(
        &wasm_hint,
        &[b"BridgeMint", b"bridge_mint_note", b"SetBridgeCfg"],
        "BridgeMint",
    )?;
    if chain_settle_enabled() {
        let dex_wasm = resolve_private_dex_wasm_hint();
        println!("  private_dex_wasm_hint={}", dex_wasm.display());
        if !dex_wasm.exists() {
            return Err(format!(
                "FAIL closed: cw_private_dex.wasm not found (looked at {}).\n\
                 Prepare: cd crates/headstash && just prepare-corridor-ict-wasm-settle\n\
                 or CORRIDOR_PREPARE_PRIVATE_DEX=1 just prepare-corridor-ict-wasm",
                dex_wasm.display()
            )
            .into());
        }
        require_wasm_surface(
            &dex_wasm,
            &[b"SettleSwap", b"settle_swap", b"CreatePool"],
            "SettleSwap",
        )?;
    }
    if zec_egress_d_gate() {
        require_wasm_surface(
            &wasm_hint,
            &[b"BridgeEgress", b"bridge_egress", b"IsEgressSpent"],
            "BridgeEgress",
        )?;
        println!("  wasm gate: BridgeEgress + SettleSwap surfaces present (Option D)");
    }

    // Single multi-thread runtime for ict-rs. DaemonBuilder must be built
    // **outside** any block_on on this runtime (handle + block_on pattern).
    let rt = Runtime::new().map_err(|e| format!("tokio Runtime: {e}"))?;

    println!("\n--- [1/7] ict-rs Docker runtime + start Terp ---");
    let mut chain = rt.block_on(async {
        let runtime = IctRuntime::Docker(DockerConfig::default())
            .into_backend()
            .await
            .map_err(|e| format!("Docker runtime (is dockerd up?): {e}"))?;
        let cfg = terp_funded_config()?;
        println!(
            "  image={}:{}",
            cfg.images[0].repository, cfg.images[0].version
        );
        let mut chain = CosmosChain::new(cfg, 1, 0, runtime);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ctx = TestContext {
            test_name: format!("corridor-ict-funded-{n}"),
            network_id: String::new(),
        };
        chain.initialize(&ctx).await?;
        chain.start(&[]).await?;
        Ok::<_, Box<dyn std::error::Error>>(chain)
    })?;

    let grpc = chain.host_grpc_address();
    let rpc = chain.host_rpc_address();
    println!("  grpc={grpc}");
    println!("  rpc={rpc}");
    println!("  chain_id={}", chain.chain_id());

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mnemonic = env_or("CORRIDOR_ICT_MNEMONIC", DEFAULT_MNEMONIC);
        println!("\n--- [2/7] cw-orch Daemon via ict-rs-cw-orch ---");
        let mut builder = daemon_builder_from_chain(&chain, Some(&mnemonic))?;
        // Attach the outer Runtime handle — we are not inside block_on here.
        builder.handle(rt.handle());
        let daemon = builder.build()?;
        println!("  sender={}", daemon.sender_addr());

        println!("\n--- [3/7] upload + instantiate cw-headstash (bridge mint only) ---");
        let mut suite = PrivateBridgeSuite::new(daemon.clone());
        suite
            .upload_and_instantiate_mint(&coins(10_000_000, DENOM))
            .map_err(|e| format!("upload/instantiate failed: {e}"))?;
        let contract = suite
            .headstash
            .headstash
            .address()
            .map_err(|e| format!("contract address: {e}"))?;
        println!("  headstash={contract}");

        println!("\n--- [4/7] configure bridge + BridgeMintNote (chain) ---");
        // Deposit-backed mint default; happy fixture only with CORRIDOR_ALLOW_HAPPY_FIXTURE=1.
        let (world, mint_source) = resolve_bridge_world(&suite)?;
        println!("  mint_source={mint_source}");

        let res = suite
            .execute_bridge_mint(world.claim.clone(), world.proof.clone())
            .map_err(|e| {
                format!("FAIL closed: BridgeMintNote execute error (no silent skip): {e}")
            })?;
        let res_s = format!("{res:?}");
        println!(
            "  BridgeMintNote response (truncated): {}",
            &res_s[..res_s.len().min(400)]
        );

        let minted = suite
            .query_is_bridge_minted(world.nullifier_bin())
            .map_err(|e| format!("IsBridgeMinted query: {e}"))?;
        if !minted {
            return Err(
                "FAIL closed: IsBridgeMinted=false after BridgeMintNote (mint not recorded)".into(),
            );
        }
        println!("  IsBridgeMinted=true for nullifier ✓");

        match suite.execute_bridge_mint(world.claim.clone(), world.proof.clone()) {
            Ok(_) => {
                return Err(
                    "FAIL closed: second BridgeMintNote succeeded (double-mint not rejected)"
                        .into(),
                );
            }
            Err(e) => {
                let es = e.to_string().to_lowercase();
                if es.contains("already") || es.contains("minted") {
                    println!("  double-mint reject ✓ ({e})");
                } else {
                    println!("  double-mint rejected with: {e}");
                }
            }
        }

        // Mint evidence for G2/G3 (client openings from claim; rcm never on chain product path)
        let intent_id = std::env::var("CORRIDOR_INTENT_ID").ok();
        let evidence = mint_evidence_from_claim_fields(
            "ict_local_funded",
            chain.chain_id(),
            contract.to_string(),
            world.claim.nullifier.as_slice(),
            world.claim.cm_public.as_slice(),
            world.claim.value_u64,
            world.terp_asset.as_slice(),
            world.claim.rcm.as_ref().map(|b| b.as_slice()),
            Some(world.claim.dest_commitment.as_slice()),
            mock_verify_flag(),
            None,
            intent_id.clone(),
        );
        let mint_path = mint_evidence_path();
        evidence
            .write_json(&mint_path)
            .map_err(|e| format!("write MintEvidenceV0: {e}"))?;
        println!("  MintEvidenceV0 → {}", mint_path.display());
        println!(
            "  mint cm={} value={} asset={}",
            evidence.cm_public_hex, evidence.value_u64, evidence.asset_id_hex
        );

        // RECOVERY-4: post-mint put_note + recover when client openings present
        #[cfg(feature = "l0-seams")]
        {
            if let (Some(rcm_hex), Some(owner_hex)) =
                (evidence.rcm_hex.as_ref(), evidence.owner_binding_hex.as_ref())
            {
                match (
                    hex::decode(evidence.cm_public_hex.trim()),
                    hex::decode(rcm_hex.trim()),
                    hex::decode(owner_hex.trim()),
                    hex::decode(evidence.asset_id_hex.trim()),
                    hex::decode(evidence.bridge_nullifier_hex.trim()),
                ) {
                    (Ok(cm), Ok(rcm), Ok(owner), Ok(asset), Ok(nu))
                        if cm.len() == 32
                            && rcm.len() == 32
                            && owner.len() == 32
                            && asset.len() == 32
                            && nu.len() == 32 =>
                    {
                        let mut cm32 = [0u8; 32];
                        let mut rcm32 = [0u8; 32];
                        let mut own32 = [0u8; 32];
                        let mut asset32 = [0u8; 32];
                        let mut nu32 = [0u8; 32];
                        cm32.copy_from_slice(&cm);
                        rcm32.copy_from_slice(&rcm);
                        own32.copy_from_slice(&owner);
                        asset32.copy_from_slice(&asset);
                        nu32.copy_from_slice(&nu);
                        let seam = seam_note_from_mint_openings(
                            &cm32,
                            &rcm32,
                            &own32,
                            evidence.value_u64,
                            &asset32,
                            &nu32,
                        );
                        let note_dir = std::env::temp_dir().join(format!(
                            "corridor-ict-note-{}",
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_nanos())
                                .unwrap_or(0)
                        ));
                        let receipt = put_and_recover_after_mint(
                            &seam,
                            &contract.to_string(),
                            [0xC0; 32],
                            &note_dir,
                        )
                        .map_err(|e| format!("FAIL closed: post-mint note put/recover: {e}"))?;
                        let film = if std::env::var("NOTES_BASE").map(|s| !s.is_empty()).unwrap_or(false)
                        {
                            "NOTES_BASE+PIR"
                        } else {
                            "local+direct"
                        };
                        println!(
                            "  note_persist put+film_recover OK film={film} addr={} path={}",
                            receipt.addr, receipt.store_path
                        );
                        // Local temp only — do not rm when using remote NOTES_BASE.
                        if std::env::var("NOTES_BASE").map(|s| s.is_empty()).unwrap_or(true) {
                            let _ = std::fs::remove_dir_all(&note_dir);
                        }
                    }
                    _ => {
                        println!(
                            "  note_persist residual: openings hex width invalid — skip put/recover"
                        );
                    }
                }
            } else {
                println!("  note_persist residual: rcm/owner openings missing on evidence");
            }
        }

        let mut private_dex_contract: Option<String> = None;
        let mut settle_receipt: Option<SettleReceiptV0> = None;
        let mut settle_handoff: Option<zk_test_press::harness::SwapSpendHandoffV0> = None;

        if chain_settle_enabled() {
            println!("\n--- [5/8] chain SettleSwap (cw-private-dex, mock_verify lab) ---");
            println!("  proof_mode={PROOF_MODE_MOCK_VERIFY_LAB} halo2_swap=false skip_ibc_post_swap=false");

            evidence.require_settle_openings().map_err(|e| {
                format!(
                    "FAIL closed: mint openings missing — cannot build SwapStatement: {e}"
                )
            })?;

            // Phases 1–3: dual-home + LP bridge mints + seed pool from bridged notes
            // (GUIDE omni). Under bridged_lp / omni profile, refuse magic CreatePool alone.
            let omni_boot: Option<OmniLiquidityBootstrap> = if require_bridged_lp_seed() {
                println!("\n--- [4.5/8] omni LP bootstrap (Phases 1–3, bridged_lp) ---");
                let dual_path = dual_home_prep_path();
                let multi_net_dh = load_dual_home_prep(&dual_path)
                    .ok()
                    .map(|m| {
                        println!(
                            "  dual-home prep {} btc_funded={} escrow_prefunded={} multi_net={}",
                            dual_path.display(),
                            m.btc_funded,
                            m.escrow_prefunded,
                            m.multi_net
                        );
                        m.is_product_dual_home_green()
                    })
                    .unwrap_or(false);
                if !multi_net_dh {
                    println!(
                        "  dual-home multi-net residual (path={}); LP seed uses prep/fixture amounts",
                        dual_path.display()
                    );
                }
                let boot = bootstrap_omni_liquidity().map_err(|e| {
                    format!("FAIL closed: bootstrap_omni_liquidity: {e}")
                })?;
                boot.assert_product_green().map_err(|e| {
                    format!("FAIL closed: lp seed product green: {e}")
                })?;
                println!("  {}", lp_mint_log_line("btc", &boot.lp_btc_mint));
                println!("  {}", lp_mint_log_line("zec", &boot.lp_zec_mint));
                println!(
                    "  liability outstanding_zec={} escrow_balance={} (I1)",
                    boot.liability.outstanding_zec_claims, boot.prep.escrow_balance_zats
                );
                println!(
                    "  lp_seed.source={} r_btc={} r_zec={} (match LP notes) multi_net_dual_home={}",
                    boot.receipt.source, boot.r_btc, boot.r_zec, multi_net_dh
                );
                let lp_path = lp_seed_receipt_path();
                boot.write_json_labeled(&lp_path, multi_net_dh)
                    .map_err(|e| format!("write LpSeedReceipt: {e}"))?;
                println!("  LpSeedReceipt → {}", lp_path.display());
                if let Ok(run_dir) = std::env::var("CORRIDOR_OMNI_RUN_DIR") {
                    let run = PathBuf::from(run_dir);
                    boot.write_omni_run_pack(&run, multi_net_dh)
                        .map_err(|e| format!("write omni LP run pack: {e}"))?;
                    println!("  omni LP pack → {}/0{{3,4,5}}-*.json", run.display());
                }
                Some(boot)
            } else {
                println!(
                    "  pool_seed={} (legacy magic lab CreatePool — not omni product green)",
                    corridor_pool_seed_policy().as_str()
                );
                None
            };

            let (r_in, r_out, _seed_rec) = resolve_pool_reserves(omni_boot.as_ref())
                .map_err(|e| format!("FAIL closed: resolve_pool_reserves: {e}"))?;
            refuse_magic_pool_if_bridged_policy(r_in, r_out, omni_boot.as_ref())
                .map_err(|e| format!("FAIL closed: pool seed policy: {e}"))?;
            // Explicit product refuse: magic LAB_R constants without bridged receipt
            if require_bridged_lp_seed()
                && omni_boot.is_none()
                && r_in == LAB_R_IN
                && r_out == LAB_R_OUT
            {
                return Err(
                    "FAIL closed: CORRIDOR_POOL_SEED=bridged_lp forbids magic CreatePool alone"
                        .into(),
                );
            }

            let handoff = build_swap_spend_handoff_from_mint_with_reserves(
                &evidence,
                ProofModeLabel::MockVerifyLab,
                r_in,
                r_out,
            )
            .map_err(|e| {
                format!("FAIL closed: build SwapSpendHandoffV0 from mint openings: {e}")
            })?;
            // Evidence glue (MintEvidenceV0 → settle spend openings); also enforced inside builder.
            assert_mint_handoff_continuous(&evidence, &handoff).map_err(|e| {
                format!("FAIL closed: mint↔settle openings continuity: {e}")
            })?;
            if let Some(ref dest) = evidence.owner_binding_hex {
                let out = hex::encode(handoff.action.witness.note_out.owner_binding);
                assert_dest_binding_equal(dest, &out, "settle.note_out.owner").map_err(|e| {
                    format!("FAIL closed: single dest seal mint→settle: {e}")
                })?;
                println!("  dest seal continuous mint→settle {}", &dest[..16.min(dest.len())]);
            }
            println!(
                "  handoff pool_id={} delta_in={} delta_out={} r_in_before={} r_out_before={} nullifiers={} proof_mode={}",
                handoff.statement.pool_id,
                handoff.statement.delta_r_in,
                handoff.statement.delta_r_out,
                handoff.statement.r_in_before,
                handoff.statement.r_out_before,
                handoff.pool_nullifiers_hex.len(),
                handoff.proof_mode.as_str()
            );
            println!(
                "  pool_nf[0]={} (≠ bridge ν {})",
                handoff.pool_nullifiers_hex.first().cloned().unwrap_or_default(),
                evidence.bridge_nullifier_hex
            );

            let mut dex = PrivateDexSuite::new(daemon.clone());
            dex.upload_and_instantiate_lab(mock_verify_flag())
                .map_err(|e| format!("FAIL closed: private-dex upload/instantiate: {e}"))?;
            let dex_addr = dex
                .address_string()
                .map_err(|e| format!("private-dex address: {e}"))?;
            println!("  private_dex={dex_addr} mock_verify={}", mock_verify_flag());
            private_dex_contract = Some(dex_addr.clone());

            // CreatePool AFTER lp seed: R must equal bridged LP values (not invent).
            if let Some(ref boot) = omni_boot {
                refuse_magic_pool_if_bridged_policy(boot.r_btc, boot.r_zec, Some(boot))
                    .map_err(|e| format!("FAIL closed: CreatePool seed gate: {e}"))?;
                dex.create_pool_from_lp_seed(&handoff, boot.r_btc, boot.r_zec)
                    .map_err(|e| format!("FAIL closed: CreatePool from bridged LP seed: {e}"))?;
                println!(
                    "  CreatePool pool_id={} r_btc={} r_zec={} source=bridged_notes ✓",
                    handoff.statement.pool_id, boot.r_btc, boot.r_zec
                );
            } else {
                if require_bridged_lp_seed() {
                    return Err(
                        "FAIL closed: CORRIDOR_POOL_SEED=bridged_lp requires bootstrap before CreatePool"
                            .into(),
                    );
                }
                dex.create_lab_pool_for_handoff(&handoff)
                    .map_err(|e| format!("FAIL closed: CreatePool: {e}"))?;
                println!(
                    "  CreatePool pool_id={} (magic lab R) ✓",
                    handoff.statement.pool_id
                );
            }

            let quote = dex
                .query_quote(
                    handoff.statement.pool_id,
                    handoff.statement.asset_in.clone(),
                    handoff.statement.delta_r_in.u128(),
                )
                .map_err(|e| format!("FAIL closed: QuoteExactIn: {e}"))?;
            if quote.delta_out != handoff.statement.delta_r_out {
                return Err(format!(
                    "FAIL closed: host quote {} ≠ statement.delta_r_out {}",
                    quote.delta_out, handoff.statement.delta_r_out
                )
                .into());
            }
            println!("  QuoteExactIn Δ_out={} ✓", quote.delta_out);

            let settle_res = dex.settle_handoff(&handoff).map_err(|e| {
                format!("FAIL closed: SettleSwap execute error (no silent skip / no film greenwash): {e}")
            })?;
            let settle_s = format!("{settle_res:?}");
            println!(
                "  SettleSwap response (truncated): {}",
                &settle_s[..settle_s.len().min(400)]
            );

            let (r_in_after, r_out_after) = dex.assert_settle_ok(&handoff).map_err(|e| {
                format!("FAIL closed: post-settle query asserts: {e}")
            })?;
            println!(
                "  Pool reserves r_in_after={r_in_after} r_out_after={r_out_after}; nullifiers spent ✓"
            );

            // Double-settle reject (nullifier)
            match dex.settle_handoff(&handoff) {
                Ok(_) => {
                    return Err(
                        "FAIL closed: second SettleSwap succeeded (nullifier not sticky)".into(),
                    );
                }
                Err(e) => {
                    let es = e.to_string().to_lowercase();
                    if es.contains("nullifier") {
                        println!("  double-settle nullifier reject ✓");
                    } else {
                        println!("  double-settle rejected with: {e}");
                    }
                }
            }

            let receipt = SettleReceiptV0 {
                profile: "ict_local_funded".into(),
                stage: "chain_settle_swap".into(),
                status: "complete".into(),
                chain_id: chain.chain_id().to_string(),
                headstash_contract: contract.to_string(),
                private_dex_contract: dex_addr.clone(),
                pool_id: handoff.statement.pool_id,
                delta_r_in: handoff.statement.delta_r_in.to_string(),
                delta_r_out: handoff.statement.delta_r_out.to_string(),
                r_in_after: r_in_after.to_string(),
                r_out_after: r_out_after.to_string(),
                nullifiers_hex: handoff.pool_nullifiers_hex.clone(),
                cm_out_hex: handoff
                    .statement
                    .cm_out
                    .iter()
                    .map(|c| hex::encode(c.as_slice()))
                    .collect(),
                bridge_nullifier_hex: evidence.bridge_nullifier_hex.clone(),
                mint_cm_public_hex: evidence.cm_public_hex.clone(),
                proof_mode: PROOF_MODE_MOCK_VERIFY_LAB.into(),
                mock_verify_dex: mock_verify_flag(),
                intent_id: intent_id.clone(),
                settle_tx_hash: None,
                halo2_swap: false,
                skip_ibc_post_swap: false,
            };
            let receipt_path = settle_receipt_path();
            receipt
                .write_json(&receipt_path)
                .map_err(|e| format!("write SettleReceiptV0: {e}"))?;
            println!("  SettleReceiptV0 → {}", receipt_path.display());
            println!(
                "  export PRIVATE_DEX_CONTRACT={dex_addr} HEADSTASH_CONTRACT={contract}"
            );
            settle_receipt = Some(receipt);
            settle_handoff = Some(handoff);
        } else {
            println!("\n--- [5/8] chain SettleSwap SKIPPED (CORRIDOR_CHAIN_SETTLE off) ---");
        }

        // Option D: settle → CW BridgeEgressBurn (or pure labeled residual) → lab pay → dual receipts
        if zec_egress_d_gate() {
            println!("\n--- [6/8] Option D ZEC egress (CORRIDOR_ZEC_EGRESS_D=1) ---");
            let receipt = settle_receipt.as_ref().ok_or(
                "FAIL closed: Option D requires SettleReceiptV0 (settle stage did not complete)",
            )?;
            let handoff = settle_handoff.as_ref().ok_or(
                "FAIL closed: Option D requires settle handoff openings (note_out rcm/cm)",
            )?;
            if receipt.status != "complete" {
                return Err(format!(
                    "FAIL closed: settle status={} — refuse Option D",
                    receipt.status
                )
                .into());
            }
            let zcfg = ZakuraLocalConfig::default();
            let sealed = sealed_dest_for_option_d(handoff, &zcfg).map_err(|e| {
                format!("FAIL closed: sealed dest for Option D (G4 / settle continuity): {e}")
            })?;
            println!(
                "  sealed dest_display={} binding={} source={}",
                sealed.dest_display,
                sealed.owner_binding_hex,
                sealed.source.as_wire_str()
            );

            let opening = settle_opening_from_handoff(handoff, &sealed).map_err(|e| {
                format!("FAIL closed: settle_opening_from_handoff: {e}")
            })?;
            let settle_ref = Some(format!(
                "pool_id={};delta_out={};cm0={}",
                receipt.pool_id,
                receipt.delta_r_out,
                receipt.cm_out_hex.first().cloned().unwrap_or_default()
            ));

            // Prefer CW BridgeEgressBurn on deployed headstash; pure residual if wasm/msg fails.
            let (pure_evidence, burn_surface) = {
                let try_cw = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let stmt = cw_egress_statement_from_opening(&opening)
                        .map_err(|e| format!("cw statement: {e}"))?;
                    let proof = lab_mock_egress_proof();
                    let res = suite
                        .execute_bridge_egress_burn(stmt.clone(), proof)
                        .map_err(|e| format!("BridgeEgressBurn: {e}"))?;
                    let res_s = format!("{res:?}");
                    println!(
                        "  BridgeEgressBurn response (truncated): {}",
                        &res_s[..res_s.len().min(300)]
                    );
                    let spent = suite
                        .query_is_egress_spent(stmt.nullifier.clone())
                        .map_err(|e| format!("IsEgressSpent: {e}"))?;
                    if !spent {
                        return Err("IsEgressSpent=false after BridgeEgressBurn".into());
                    }
                    println!("  IsEgressSpent=true ✓");
                    // Continuity evidence for lab pay (pure shape from openings).
                    let (ev, _) = apply_pure_egress_burn_labeled(
                        &opening,
                        &sealed,
                        settle_ref.clone(),
                    )
                    .map_err(|e| format!("local evidence after CW burn: {e}"))?;
                    Ok((ev, BURN_SURFACE_CW_BRIDGE_EGRESS.to_string()))
                })();
                match try_cw {
                    Ok(v) => v,
                    Err(e) => {
                        println!(
                            "  CW BridgeEgressBurn unavailable ({e}) — pure_record_lab residual"
                        );
                        apply_pure_egress_burn_labeled(&opening, &sealed, settle_ref).map_err(
                            |e| {
                                format!(
                                    "FAIL closed: pure egress burn after CW residual: {e}"
                                )
                            },
                        )?
                    }
                }
            };
            println!("  burn_surface={burn_surface}");

            println!(
                "  lab_pay: wallet_rpc_available={} rpc={} profile={:?} release={:?} residual_flag={}",
                wallet_rpc_available(&zcfg),
                zcfg.rpc_url,
                corridor_profile(),
                zec_release_policy(),
                allow_lab_inventory_residual()
            );
            let zec = lab_pay_zec_after_burn(&pure_evidence, &sealed).map_err(|e| {
                format!("FAIL closed: lab_pay_zec_after_burn (requires burn evidence): {e}")
            })?;
            println!(
                "  lab_pay result mode={} zec_txid={:?}",
                zec.mode, zec.zec_txid
            );
            // Phase 7 product bar: refuse funder-only / inventory as product green.
            if is_product_profile() {
                assert_product_release_mode(&zec.mode).map_err(|e| {
                    format!("FAIL closed: reject_funder_only_as_product / product green: {e}")
                })?;
                if reject_funder_only_as_product(&zec.mode) {
                    return Err(format!(
                        "FAIL closed: product profile cannot green on funder-only mode={}",
                        zec.mode
                    )
                    .into());
                }
                match zec_release_policy() {
                    ZecReleasePolicy::ThresholdEscrow => {
                        if zec.mode != MODE_THRESHOLD_ESCROW_RELEASE
                            && zec.mode != MODE_THRESHOLD_ESCROW_RELEASE_SIMULATED
                        {
                            return Err(format!(
                                "FAIL closed: CORRIDOR_ZEC_RELEASE=threshold_escrow but mode={}",
                                zec.mode
                            )
                            .into());
                        }
                        println!(
                            "  product threshold release OK committee_crypto={COMMITTEE_CRYPTO_LAB_T_OF_N} \
                             escrow_sign_path={ESCROW_SIGN_PATH_P_LAB} (not FROST)"
                        );
                    }
                    ZecReleasePolicy::LabInventory => {
                        // Only reachable with residual flag (assert_product_release_mode).
                        println!(
                            "  LAB INVENTORY RESIDUAL (not product architecture power): mode={}",
                            zec.mode
                        );
                    }
                }
            }

            // Open/confirm at sealed dest (validateaddress / balance when possible).
            match confirm_open_at_sealed_dest_with_cfg(&sealed, &zcfg) {
                Ok(oc) => {
                    println!(
                        "  open_confirm mode={} valid={:?} bal_zat={:?} recv_zat={:?} — {}",
                        oc.mode, oc.address_valid, oc.wallet_balance_zat, oc.received_zat, oc.note
                    );
                }
                Err(e) => {
                    // Only fail when CORRIDOR_REQUIRE_ZAKURA_RPC forces it.
                    println!("  open_confirm residual: {e}");
                }
            }

            let evidence_path = egress_burn_evidence_path();
            let zec_path = zec_egress_receipt_path();
            let receipt_path = settle_receipt_path();

            // Build JSON artifacts (same shapes as run_option_d_after_settle)
            let evidence_json = EgressBurnEvidenceJson {
                schema: "EgressBurnEvidenceV0".into(),
                profile: "ict_local_funded".into(),
                stage: "option_d_egress_burn".into(),
                status: "complete".into(),
                burn_surface: burn_surface.clone(),
                terp_tx_hash: pure_evidence.terp_tx_hash.clone(),
                burn: EgressBurnPublicJson {
                    asset_id_hex: hex::encode(pure_evidence.burn.asset_id),
                    value: pure_evidence.burn.value,
                    cm_spent_hex: hex::encode(pure_evidence.burn.cm_spent),
                    nullifier_hex: hex::encode(pure_evidence.burn.nullifier),
                    dest_commitment_hex: hex::encode(pure_evidence.burn.dest_commitment),
                    dest_kind: match pure_evidence.burn.dest_kind {
                        terp_seams::dex::DestKind::Transparent => "transparent".into(),
                        terp_seams::dex::DestKind::Shielded => "shielded".into(),
                    },
                    root_hex: hex::encode(pure_evidence.burn.root),
                    source_pool_id: pure_evidence.burn.source_pool_id,
                },
                proof_mode: pure_evidence.proof_mode.clone(),
                settle_receipt_ref: pure_evidence.settle_receipt_ref.clone(),
                settle_receipt_path: Some(receipt_path.display().to_string()),
                chain_id: Some(receipt.chain_id.clone()),
                headstash_contract: Some(receipt.headstash_contract.clone()),
                private_dex_contract: Some(receipt.private_dex_contract.clone()),
                intent_id: receipt.intent_id.clone(),
                egress_nf_label: "egress-nf-v0".into(),
            };
            evidence_json
                .write_json(&evidence_path)
                .map_err(|e| format!("write EgressBurnEvidenceV0: {e}"))?;

            let auth_meta = peek_last_threshold_auth_meta();
            let zec_json = ZecEgressReceiptJson {
                schema: "ZecEgressReceiptV0".into(),
                profile: "ict_local_funded".into(),
                stage: "option_d_zec_lab_pay".into(),
                status: "complete".into(),
                dest_display: zec.dest_display.clone(),
                dest_owner_binding_hex: zec.dest_owner_binding_hex.clone(),
                dest_kind: match zec.dest_kind {
                    terp_seams::dex::DestKind::Transparent => "transparent".into(),
                    terp_seams::dex::DestKind::Shielded => "shielded".into(),
                },
                zec_txid: zec.zec_txid.clone(),
                amount_zat: zec.amount_zat,
                mode: zec.mode.clone(),
                burn_nullifier_hex: zec.burn_nullifier_hex.clone(),
                burn_evidence_path: Some(evidence_path.display().to_string()),
                sealed_source: Some(sealed.source.as_wire_str().into()),
                auth_source: auth_meta.as_ref().map(|m| m.auth_source.clone()),
                committee_crypto: auth_meta.as_ref().map(|m| m.committee_crypto.clone()),
                custody_label: auth_meta.as_ref().map(|m| m.custody_label.clone()),
                frost: Some(false),
            };
            zec_json
                .write_json(&zec_path)
                .map_err(|e| format!("write ZecEgressReceiptV0: {e}"))?;

            println!(
                "  EgressBurnEvidenceV0 → {} status={} proof_mode={} ν={}",
                evidence_path.display(),
                evidence_json.status,
                evidence_json.proof_mode,
                evidence_json.burn.nullifier_hex
            );
            println!(
                "  ZecEgressReceiptV0 → {} mode={} amount_zat={} dest={}",
                zec_path.display(),
                zec_json.mode,
                zec_json.amount_zat,
                zec_json.dest_display
            );
            // RECOVERY-5: labeled modes must not be misread as product success
            assert_lab_degrade_labels(&burn_surface, &zec.mode)
                .map_err(|e| format!("FAIL closed: lab degrade labels: {e}"))?;
            if product_celebrate_forbidden(&burn_surface, &zec.mode) {
                println!(
                    "  LAB DEGRADE (not product celebrate): burn_surface={burn_surface} pay_mode={}",
                    zec.mode
                );
            }
            let _ = BURN_SURFACE_PURE_RECORD_LAB; // used when CW residual
            if evidence_json.status != "complete" || zec_json.status != "complete" {
                return Err("FAIL closed: Option D dual receipts not complete".into());
            }
        } else {
            println!("\n--- [6/8] Option D ZEC egress SKIPPED (CORRIDOR_ZEC_EGRESS_D off) ---");
        }

        // Pure film: skip by default when settle on; CORRIDOR_SKIP_SWAP_FILM still works.
        // CORRIDOR_SKIP_SWAP_FILM must NEVER skip settle/egress (already gated above).
        let run_film = if chain_settle_enabled() || zec_egress_d_gate() {
            env_truthy("CORRIDOR_RUN_SWAP_FILM") && !env_truthy("CORRIDOR_SKIP_SWAP_FILM")
        } else {
            !env_truthy("CORRIDOR_SKIP_SWAP_FILM")
        };

        if run_film {
            println!("\n--- [7/8] oracle-bound private swap film (pure W0–W7) ---");
            if chain_settle_enabled() {
                println!("  (labeled residual; chain settle is success criterion)");
            }
            let mut backend = harness::CorridorAssetBackend::simulated();
            let scenario = harness::CorridorScenario::default();
            let out = harness::run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario)
                .map_err(|e| format!("swap film: {e}"))?;
            println!(
                "  receipt status={} backend={} corridor_id={}",
                out.receipt.status, out.receipt.asset_backend, out.receipt.corridor_id
            );
            if out.receipt.status != "complete" {
                return Err(format!(
                    "FAIL closed: swap film status={} (expected complete)",
                    out.receipt.status
                )
                .into());
            }
        } else if chain_settle_enabled() || zec_egress_d_gate() {
            println!(
                "\n--- [7/8] pure swap film residual SKIPPED (settle/egress on; set CORRIDOR_RUN_SWAP_FILM=1 to enable) ---"
            );
        } else {
            println!("\n--- [7/8] swap film SKIPPED (CORRIDOR_SKIP_SWAP_FILM) ---");
        }

        println!("\n--- [8/8] evidence ---");
        println!("  profile=ict_local_funded");
        println!("  mock_verify={}", mock_verify_flag());
        println!("  headstash_contract={contract}");
        if let Some(ref d) = private_dex_contract {
            println!("  private_dex_contract={d}");
        }
        if let Some(ref r) = settle_receipt {
            println!(
                "  settle status={} proof_mode={} pool_id={} halo2_swap={}",
                r.status, r.proof_mode, r.pool_id, r.halo2_swap
            );
        }
        if zec_egress_d_gate() {
            println!(
                "  option_d=OK burn_surface=pure_record_lab (CW residual) lab_pay=gated"
            );
        }
        println!("  chain_id={}", chain.chain_id());
        println!("  grpc={grpc}");
        println!("  rpc={rpc}");
        println!("  BridgeMintNote=OK IsBridgeMinted=true");
        if chain_settle_enabled() {
            println!("  SettleSwap=OK proof_mode={PROOF_MODE_MOCK_VERIFY_LAB}");
        }
        if zec_egress_d_gate() {
            println!("  OptionD=OK CORRIDOR_ZEC_EGRESS_D=1");
        }
        println!("OK corridor_ict_funded");
        Ok(())
    })();

    if !env_truthy("KEEP_CHAIN") {
        println!("\n--- stopping ict-rs chain ---");
        if let Err(e) = rt.block_on(async { chain.stop().await }) {
            eprintln!("warn: chain.stop: {e}");
        }
    } else {
        println!("KEEP_CHAIN=1 — leaving containers up");
    }

    result
}

// Silence unused when settle path not compiled in some feature combos.
#[allow(dead_code)]
fn _mint_evidence_type_check(e: &MintEvidenceV0) {
    let _ = e.profile.as_str();
}
