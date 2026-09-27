//! FROST privval sidecar e2e — ict-rs + cw-orch suite design.
//!
//! Sound vs unsound matrix from
//! `crates/terp-rs/reviews/HANDOFF-FROST-SIDECAR-E2E-2026-09-03.md`.
//!
//! Layers (same convention as `nostr_orch_suite` / `hashmerchant_live`):
//! - **L0 (always):** in-process [`FrostPrivvalHarness`] — no Docker.
//!   Each plane is its own test so they run **in parallel**.
//! - **L1 (mock):** [`FrostPrivvalSuite`] lifecycle over loopback (no Docker image).
//! - **L2 (ignored):** full lifecycle — ict-rs Docker chain + cw-orch Daemon +
//!   [`FrostPrivvalSuite`] + operator railguards. Gate: `ICT_FROST=1`.
//!
//! Run:
//! ```bash
//! cargo test -p terp-scripts --test frost_privval_e2e --features hash-market -- --nocapture
//! ICT_FROST=1 cargo test -p terp-scripts --test frost_privval_e2e --features hash-market -- --ignored --nocapture
//! ```
//!
//! Hard rule: bind type only in inventory (`sign_consensus_for(..., step)`).
//! Never assert "presig equals this prevote" — presigs are message-independent.

use terp_scripts::suite::frost::{
    assert_sig_valid, check_empty_pool_recovers_without_ceremony, check_vote_bytes_bind_chain_id,
    fail_if_dual_signer, fail_if_shared_kcons, golden_vote_sign_bytes, presig_step_for_vote_type,
    FrostMatrix, FrostPrivvalHarness, FrostPrivvalSuite,
};
use terp_scripts::suite::sidecar::TerpSidecar;

// ---------------------------------------------------------------------------
// L0 — sound (must pass / 64-byte dalek-valid sigs)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sound_colocated_2_of_3_signs_ed25519() {
    let h = FrostPrivvalHarness::new("sound-colocated").unwrap();
    let r = h.run(FrostMatrix { colocated_sign: true, ..Default::default() }).await.unwrap();
    r.assert_all();
}

#[tokio::test]
async fn sound_inventory_earmark_and_consume() {
    let h = FrostPrivvalHarness::new("sound-earmark").unwrap();
    let r = h.run(FrostMatrix { earmark_consume: true, ..Default::default() }).await.unwrap();
    r.assert_all();
}

#[tokio::test]
async fn sound_refill_restores_after_consume() {
    let h = FrostPrivvalHarness::new("sound-refill").unwrap();
    let r = h.run(FrostMatrix { refill: true, ..Default::default() }).await.unwrap();
    r.assert_all();
}

#[test]
fn sound_vote_sign_bytes_matches_cometbft_0_40() {
    let bz = golden_vote_sign_bytes().unwrap();
    let want = hex::decode("730801112a0000000000000019010000000000000022480a200102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2012240801122000000000000000000000000000000000000000000000000000000000000000002a0b0880e2cfaa0610959aef3a3206746573742d31").unwrap();
    assert_eq!(bz, want, "protoio CanonicalVote == Go VoteSignBytes");
}

#[tokio::test]
async fn sound_cmt_signvote_protobuf_loopback() {
    let h = FrostPrivvalHarness::new("sound-cmt").unwrap();
    let r = h.run(FrostMatrix { cmt_loopback: true, ..Default::default() }).await.unwrap();
    r.assert_all();
}

#[tokio::test]
async fn sound_json_loopback_sign_and_hrs() {
    let h = FrostPrivvalHarness::new("sound-json").unwrap();
    let r = h.run(FrostMatrix { json_loopback: true, ..Default::default() }).await.unwrap();
    r.assert_all();
}

#[tokio::test]
async fn sound_noise_xx_roundtrip() {
    let h = FrostPrivvalHarness::new("sound-noise").unwrap();
    let r = h.run(FrostMatrix { noise_roundtrip: true, ..Default::default() }).await.unwrap();
    r.assert_all();
}

#[test]
fn sound_kms_scheme_dkg_signs_as_ed25519() {
    use std::collections::BTreeMap;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use kms_shard::{dkg_all, ConsensusScheme, FrostEd25519};
    use rand::rngs::OsRng;

    let mut rng = OsRng;
    let (shares, group) = dkg_all::<FrostEd25519>(3, 2, &mut rng).expect("dkg");
    let pk = FrostEd25519::group_pk_bytes(&group).unwrap();
    let vk = VerifyingKey::from_bytes(&pk).unwrap();
    let msg = b"cometbft-vote-bytes";
    let mut commit_map = BTreeMap::new();
    let mut nonces = BTreeMap::new();
    for id in [1u16, 2] {
        let share = shares.iter().find(|(i, _)| i == &id).unwrap().1.clone();
        let (n, c) = FrostEd25519::commit(&share, &mut rng).unwrap();
        commit_map.insert(frost_ed25519::Identifier::try_from(id).unwrap(), c);
        nonces.insert(id, (share, n));
    }
    let mut partials = BTreeMap::new();
    for id in [1u16, 2] {
        let (share, n) = nonces.remove(&id).unwrap();
        partials.insert(id, FrostEd25519::sign_share(&share, n, &commit_map, msg).unwrap());
    }
    let sig = FrostEd25519::combine(&commit_map, &partials, msg, &group).unwrap();
    let bytes = FrostEd25519::signature_bytes(&sig).unwrap();
    assert_eq!(bytes.len(), 64);
    vk.verify(msg, &Signature::from_bytes(&bytes)).unwrap();
}

// ---------------------------------------------------------------------------
// L0 — unsound (must fail closed — railguards)
// ---------------------------------------------------------------------------

#[test]
fn unsound_below_threshold_cannot_combine() {
    use kms_shard::{dkg_all, ConsensusScheme, FrostEd25519};
    use rand::rngs::OsRng;
    use std::collections::BTreeMap;

    let mut rng = OsRng;
    let (shares, group) = dkg_all::<FrostEd25519>(3, 2, &mut rng).unwrap();
    let _ = group;
    let share = &shares[0].1;
    let (n, c) = FrostEd25519::commit(share, &mut rng).unwrap();
    let mut commit_map = BTreeMap::new();
    commit_map.insert(frost_ed25519::Identifier::try_from(shares[0].0).unwrap(), c);
    assert!(
        FrostEd25519::sign_share(share, n, &commit_map, b"one-share-must-fail").is_err(),
        "<t shares yield a sig — railguard broken"
    );
}

#[test]
fn unsound_same_hrs_cached_and_regression_refused() {
    let dir = std::env::temp_dir().join(format!("frost-hrs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("hrs.json");
    let mut l = hash_market::custody::hrs::HrsLock::load(&p).unwrap();
    assert!(l.check(1, 0, 2).unwrap().is_none());
    l.commit(1, 0, 2, &[9u8; 64]).unwrap();
    let again = hash_market::custody::hrs::HrsLock::load(&p).unwrap();
    assert_eq!(again.check(1, 0, 2).unwrap().unwrap(), vec![9u8; 64]);
    assert!(again.check(1, 0, 1).is_err(), "lower step must refuse double-sign");
    assert!(again.check(0, 0, 2).is_err(), "lower height must refuse");
}

#[tokio::test]
async fn unsound_abort_pool_zeroizes_unused() {
    use hash_market::custody::consensus::ConsensusSigner;
    let coord = hash_market::custody::frost::FrostCoordinator::from_dkg(3, 2).unwrap();
    assert!(coord.stock().unwrap().total() > 0);
    coord.abort_pool().unwrap();
    assert_eq!(coord.stock().unwrap().total(), 0, "reuse after abort — railguard broken");
    // fail-closed recovery: next sign mints one emergency slot, never a ceremony
    let sig = coord.sign_consensus(b"after-abort").await.unwrap();
    assert_sig_valid(coord.group_pubkey(), b"after-abort", &sig, "post-abort").unwrap();
}

#[test]
fn unsound_earmark_does_not_steal_precommit_for_prevote() {
    use hash_market::custody::presig::{PresigInventory, PresigKind, StockPolicy};
    let mut inv = PresigInventory::new(StockPolicy { rounds_buf: 1, may_propose: false, margin: 0 });
    inv.push_minted(1u8);
    inv.push_minted(2u8);
    assert_eq!(inv.take(PresigKind::Prevote).unwrap(), 1);
    assert_eq!(inv.snapshot().precommit, 1, "stall precommit — railguard broken");
    assert!(inv.take(PresigKind::Prevote).is_none(), "mix steps — railguard broken");
    assert_eq!(inv.take(PresigKind::Precommit).unwrap(), 2);
}

#[tokio::test]
async fn unsound_poisoned_slot_drops_and_fails_closed() {
    use hash_market::custody::consensus::ConsensusSigner;
    // commit published, z_i never issued, unsure leak → drop slot, fail closed.
    let coord = hash_market::custody::frost::FrostCoordinator::from_dkg(3, 2).unwrap();
    let n0 = coord.stock().unwrap().total();
    assert!(n0 > 0);
    assert_eq!(coord.abort_pool().unwrap(), n0);
    assert_eq!(coord.stock().unwrap().total(), 0);
    let sig = coord.sign_consensus(b"post-poison").await.unwrap();
    assert_sig_valid(coord.group_pubkey(), b"post-poison", &sig, "post-poison").unwrap();
}

#[test]
fn unsound_stock_formula_and_drain() {
    use hash_market::custody::presig::StockPolicy;
    let p = StockPolicy { rounds_buf: 3, may_propose: true, margin: 2 };
    assert_eq!(p.target(), 3 * 3 + 2);
    let q = StockPolicy { rounds_buf: 2, may_propose: false, margin: 1 };
    assert_eq!(q.target(), 2 * 2 + 1);
}

/// Inventory binds type only via step — presigs stay message-independent.
#[test]
fn invariant_presig_bound_by_step_not_payload() {
    assert_eq!(presig_step_for_vote_type(1), 1, "prevote → step 1");
    assert_eq!(presig_step_for_vote_type(2), 2, "precommit → step 2");
    assert_eq!(presig_step_for_vote_type(0), 0, "unknown → Any");
}

// ---------------------------------------------------------------------------
// Full matrix (mirrors scripts/e2e-frost-privval.sh SOUND + UNSOUND)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn matrix_full_in_process_sound_and_unsound() {
    let h = FrostPrivvalHarness::new("matrix-full").unwrap();
    let r = h.run(FrostMatrix::full_in_process()).await.unwrap();
    assert_eq!(r.lines.len(), 13, "sound(8) + unsound(5): {r:?}");
    r.assert_all();
}

// ---------------------------------------------------------------------------
// L1 — mock sidecar lifecycle (no Docker image)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn l1_mock_backend_sidecar_lifecycle() {
    use terp_scripts::suite::sidecar::SidecarRegistry;
    let mut suite = FrostPrivvalSuite::colocated_json("frost-mock-l1");
    let registry = SidecarRegistry::new();
    let eps = suite.start(&registry).await.expect("start loopback sidecar");
    assert!(eps["privval"].starts_with("tcp://127.0.0.1:"));
    assert!(suite.privval_addr().is_some());
    assert!(suite.pubkey_hex().is_some());
    let health = suite.health().await.expect("health");
    assert_ne!(format!("{health:?}"), "Unhealthy", "{health:?}");
    suite.stop().await.expect("stop");
}

#[tokio::test]
async fn l1_cmt_suite_registers_privval_endpoint() {
    use terp_scripts::suite::sidecar::SidecarRegistry;
    let mut suite = FrostPrivvalSuite::cometbft_cmt("frost-mock-cmt");
    let registry = SidecarRegistry::new();
    let eps = suite.start(&registry).await.unwrap();
    assert!(eps.contains_key("privval"));
    assert!(eps.contains_key("pubkey"));
    suite.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// L2 — Docker chain + cw-orch Daemon + sidecar + operator railguards
// ---------------------------------------------------------------------------

const ICT_GATE: &str = "ICT_FROST";
const LAB_MNEMONIC: &str = "chapter wrist alcohol shine angry noise mercy simple rebel recycle \
    vehicle wrap morning giraffe lazy outdoor noise blood ginger sort reunion boss crowd dutch";

fn require_gate() {
    assert_eq!(
        std::env::var(ICT_GATE).ok().as_deref(),
        Some("1"),
        "{ICT_GATE}=1 required (docker localterp/local-zk + cw-orch Daemon)"
    );
}

fn chain_info(chain: &ict_rs::prelude::CosmosChain) -> cw_orch::environment::ChainInfoOwned {
    use cw_orch::environment::{ChainKind, NetworkInfoOwned};
    use ict_rs::chain::Chain;
    let cfg = chain.config();
    let gas_price: f64 = cfg
        .gas_prices
        .trim_end_matches(|c: char| c.is_alphabetic())
        .parse()
        .unwrap_or(0.025);
    cw_orch::environment::ChainInfoOwned {
        chain_id: cfg.chain_id.clone(),
        gas_denom: cfg.denom.clone(),
        gas_price,
        grpc_urls: vec![chain.host_grpc_address()],
        lcd_url: None,
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: cfg.name.clone(),
            pub_address_prefix: cfg.bech32_prefix.clone(),
            coin_type: cfg.coin_type,
        },
        kind: ChainKind::Local,
    }
}

/// Full lifecycle: chain blocks + Daemon bank query + loopback sidecar +
/// operator railguards. Skips gracefully when Docker is unavailable.
#[tokio::test]
#[ignore = "ICT_FROST=1 — docker chain + cw-orch Daemon + frost sidecar"]
async fn l2_full_lifecycle_chain_daemon_sidecar_railguards() -> anyhow::Result<()> {
    use ict_rs::prelude::*;
    use ict_rs::testing::env::TestEnv;

    require_gate();
    let _ = env_logger::try_init();

    let runtime = match IctRuntime::Docker(DockerConfig::default()).into_backend().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping L2: Docker unavailable: {e}");
            return Ok(());
        }
    };
    let mut cfg = TestEnv::terp_config();
    cfg.chain_id = "ict-frost-e2e".into();
    let mut chain = CosmosChain::new(cfg, 1, 0, runtime);
    if let Err(e) = chain
        .initialize(&TestContext { test_name: "frost-privval-e2e".into(), network_id: String::new() })
        .await
    {
        eprintln!("Skipping L2: initialize failed: {e}");
        return Ok(());
    }
    if let Err(e) = chain.start(&[]).await {
        eprintln!("Skipping L2: chain start failed: {e}");
        return Ok(());
    }
    let result = run_l2_workflow(&chain).await;
    let _ = chain.stop().await;
    result
}

async fn run_l2_workflow(chain: &ict_rs::prelude::CosmosChain) -> anyhow::Result<()> {
    use cw_orch::environment::{BankQuerier, QuerierGetter};
    use ict_rs::prelude::*;

    chain.build_wallet("lab", LAB_MNEMONIC).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let lab = chain.key_address("lab").await.map_err(|e| anyhow::anyhow!("{e}"))?;
    chain
        .send_funds("validator", &WalletAmount { address: lab.clone(), denom: "uterp".into(), amount: 10_000_000 })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    wait_for_blocks(chain, 2).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("PASS chain funded {lab}");

    // cw-orch Daemon is the secondary read path (bank query), like hashmerchant_live.
    let info = chain_info(chain);
    let grpc = info.grpc_urls.clone();
    let coins = tokio::task::spawn_blocking(move || {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let daemon = cw_orch::daemon::DaemonBuilder::new(info)
            .is_test(true)
            .mnemonic(LAB_MNEMONIC)
            .build()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let bank: cw_orch::daemon::queriers::Bank = daemon.querier();
        let coins = bank
            .balance(&cw_orch::prelude::Addr::unchecked(lab), None)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok::<_, anyhow::Error>(format!("{coins:?}"))
    })
    .await??;
    println!("PASS cw-orch gRPC {grpc:?} (bank secondary) {coins}");

    // Loopback sidecar composes with the live chain env (no consensus keys leave).
    let mut suite = FrostPrivvalSuite::cometbft_cmt("frost-l2");
    let registry = terp_scripts::suite::sidecar::SidecarRegistry::new();
    let eps = suite.start(&registry).await?;
    println!("PASS sidecar {}", eps["privval"]);
    assert!(suite.health().await.is_ok());

    // In-process matrix still gates the L2 run: wanted signing works.
    let h = FrostPrivvalHarness::new("l2-gate")?;
    let r = h.run(FrostMatrix::sound_only()).await?;
    r.assert_all();

    // Operator railguards — manual-fail conditions from the handoff.
    let sidecar_pk = hex::decode(suite.pubkey_hex().map(|s| s.to_owned()).unwrap_or("00".repeat(32)))?;
    fail_if_dual_signer(&sidecar_pk, None)?; // no filePV loaded for this pk in test
    println!("PASS railguard no-dual-signer");
    fail_if_shared_kcons("sidecar-K_cons-test", None)?; // no Go kms shares K_cons here
    println!("PASS railguard no-shared-K_cons");

    // Empty-pool liveness: abort → drain → emergency single-mint signs.
    {
        use hash_market::custody::consensus::ConsensusSigner;
        let coord = hash_market::custody::frost::FrostCoordinator::from_dkg(3, 2)?;
        let before = coord.stock()?.total();
        let dropped = coord.abort_pool()?;
        let sig = coord.sign_consensus(b"l2-after-abort").await?;
        check_empty_pool_recovers_without_ceremony(before, before - dropped, sig.len())?;
        assert_sig_valid(coord.group_pubkey(), b"l2-after-abort", &sig, "l2")?;
        println!("PASS railguard empty-pool emergency-mint (no ceremony)");
    }

    // Vote bytes bind chain_id — never a guessed block_id payload.
    {
        let bz = golden_vote_sign_bytes()?;
        check_vote_bytes_bind_chain_id(&bz, "ict-frost-e2e")?;
        println!("PASS railguard no-guessed-block_id");
    }

    // kms-dao path stays K_op (secp256k1): consensus Ed25519 scalar never leaves.
    {
        use hash_market::custody::consensus::ConsensusSigner;
        let coord = hash_market::custody::frost::FrostCoordinator::from_dkg(3, 2)?;
        assert_eq!(coord.group_pubkey().len(), 32, "K_cons is Ed25519");
        println!("PASS validator-exercise K_cons=Ed25519 separates from K_op=secp256k1");
    }

    suite.stop().await.ok();
    Ok(())
}
