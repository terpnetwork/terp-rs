//! Store + instantiate passkey, recovery, ed25519, eth on morocco-1.
//!
//! ```bash
//! cd crates/terp-rs
//! set -a && source .env && set +a
//! cargo run -p terp-authenticator-suite --features daemon --bin deploy-noncircuit-auth
//! ```
//!
//! Env: `MAIN_MNEMONIC` (required). Optional `TERP_GRPC` (default https://grpc.terp.network:443),
//! `ETH_SIGNER` (0x… for terp-eth instantiate).

use std::env;

use cosmwasm_std::Binary;
use cw_orch::daemon::networks::TERP_MAINNET;
use cw_orch::prelude::*;
use terp_authenticator_suite::{ETH_GOLDEN_SIGNER, TerpAuthenticatorSuite};

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::from_filename(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.env"),
    );
    env_logger::init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let mnemonic = env::var("MAIN_MNEMONIC").map_err(|_| {
        anyhow::anyhow!("MAIN_MNEMONIC unset — load crates/terp-rs/.env (never print the seed)")
    })?;
    if mnemonic.trim().is_empty() {
        anyhow::bail!("MAIN_MNEMONIC is empty");
    }

    // Public https://grpc.terp.network is Cloudflare HTML, not gRPC. Prefer LAN sentry.
    let grpc = env::var("TERP_GRPC").unwrap_or_else(|_| "http://192.168.10.101:9090".into());
    let eth_signer = env::var("ETH_SIGNER").unwrap_or_else(|_| ETH_GOLDEN_SIGNER.into());

    let mut chain: ChainInfoOwned = TERP_MAINNET.into();
    chain.grpc_urls = vec![grpc.clone()];

    eprintln!("chain={} grpc={} (mnemonic not logged)", chain.chain_id, grpc);

    let daemon = DaemonBuilder::new(chain)
        .mnemonic(mnemonic)
        .deployment_id("noncircuit-auth")
        .load_network(false)
        .build()?;

    let admin = daemon.sender_addr();
    eprintln!("sender={admin}");

    let suite = TerpAuthenticatorSuite::new(daemon);
    suite.upload_noncircuit_priority()?;

    suite.passkey.instantiate(
        &terp_passkey::InstantiateMsg {
            owner: Some(admin.to_string()),
            registration: terp_passkey::PasskeyRegistration {
                origin: Some("https://app.terp.network".into()),
                credential_id: Binary::from(b"cred-id-1"),
            },
        },
        Some(&admin),
        &[],
    )?;

    suite.recovery.instantiate(
        &terp_recovery::InstantiateMsg {
            config: terp_recovery::RecoveryConfig {
                guardians: vec![terp_recovery::GuardianConfig {
                    address: admin.to_string(),
                    pubkey: None,
                    commitment: None,
                }],
                threshold: 1,
                hash_alg: terp_recovery::RecoveryHashAlg::Sha256,
                rehash_rounds: 1,
                domain: terp_recovery::DEFAULT_DOMAIN.into(),
                allow_address_only_approvals: true,
            },
        },
        Some(&admin),
        &[],
    )?;

    suite.ed25519.instantiate(
        &terp_ed25519::InstantiateMsg {
            owner: Some(admin.to_string()),
            pubkey: Binary::from([1u8; 32]),
        },
        Some(&admin),
        &[],
    )?;

    suite.eth.instantiate(
        &terp_eth::InstantiateMsg {
            owner: Some(admin.to_string()),
            signer: eth_signer,
        },
        Some(&admin),
        &[],
    )?;

    println!("terp-passkey  code_id={} addr={}", suite.passkey.code_id()?, suite.passkey.addr_str()?);
    println!("terp-recovery code_id={} addr={}", suite.recovery.code_id()?, suite.recovery.addr_str()?);
    println!("terp-ed25519  code_id={} addr={}", suite.ed25519.code_id()?, suite.ed25519.addr_str()?);
    println!("terp-eth      code_id={} addr={}", suite.eth.code_id()?, suite.eth.addr_str()?);
    Ok(())
}
