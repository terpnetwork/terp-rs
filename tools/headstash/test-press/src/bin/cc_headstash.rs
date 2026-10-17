//! Build Headstash **Product A** circuit keys for CosmWasm zkvm upload.
//!
//! Writes the store-circuit blob (`params || cs || vk || CircuitFooter`) that
//! `upload_circuit` / on-chain store-circuit expects. Does **not** publish or
//! upload unless `--upload` is passed with a chain daemon.
//!
//! ## Offline keygen (default — no network, no mnemonic)
//!
//! ```bash
//! cd crates/headstash
//! export RAYON_NUM_THREADS=8   # optional parallel keygen
//! export HEADSTASH_VK_PATH=artifacts/headstash_vk.bin   # optional path
//!
//! cargo run -p zk-test-press --bin cc_headstash --features interface
//! # or:
//! just demo-keys
//! ```
//!
//! ## Optional: upload after keys exist
//!
//! ```bash
//! export MNEMONIC='…'   # never commit
//! # CHAIN=local for localterp; default TERP public testnet
//! cargo run -p zk-test-press --bin cc_headstash --features interface -- --upload
//! ```
//!
//! Product A: 6 public inputs (anchor, nd, v, recp, nf, cmx), K=18, Pasta.

use std::env;
use std::path::PathBuf;

use cw_orch::{
    anyhow::{self, Context},
    daemon::Daemon,
    prelude::*,
};
use zk_headstash::suite::suite::build_headstash_keys_to;

fn main() -> Result<(), anyhow::Error> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    let do_upload = args.iter().any(|a| a == "--upload");

    let vk_path = env::var("HEADSTASH_VK_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("artifacts").join("headstash_vk.bin"));

    // Always keygen offline first (daemon not required for the blob).
    build_headstash_keys_to(&vk_path).map_err(|e| anyhow::anyhow!("keygen: {e}"))?;
    eprintln!("offline keygen complete: {}", vk_path.display());

    if !do_upload {
        eprintln!("next: store-circuit / upload_circuit with this blob (omit secrets from git)");
        return Ok(());
    }

    let mnemonic = env::var("MNEMONIC").unwrap_or_default();
    if mnemonic.is_empty() {
        anyhow::bail!(
            "--upload requires MNEMONIC in the environment (never commit mnemonics)."
        );
    }

    let chain_id = env::var("CHAIN").unwrap_or_else(|_| "testnet".into());
    let network = match chain_id.as_str() {
        "local" | "localterp" | "localnet" => {
            cw_orch::daemon::networks::TERP_LOCALNET.clone()
        }
        _ => cw_orch::daemon::networks::TERP_TESTNET.clone(),
    };

    let terp = Daemon::builder(network)
        .mnemonic(&mnemonic)
        .build()
        .context("Daemon::build")?;

    let hs = zk_test_press::suites::headstash::HeadstashSuite::new(terp);

    // Ensure upload path sees a blob: suite vk_path may differ from HEADSTASH_VK_PATH.
    {
        use cw_orch::prelude::CircuitUploadable;
        let suite_path = zk_headstash::suite::HeadstashCircuitSuite::<Daemon>::vk_path();
        if suite_path != vk_path {
            if let Some(parent) = suite_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&vk_path, &suite_path).with_context(|| {
                format!(
                    "copy {} → {} for upload_circuit",
                    vk_path.display(),
                    suite_path.display()
                )
            })?;
            eprintln!("copied key blob to suite vk_path: {}", suite_path.display());
        }
    }

    eprintln!("uploading circuit blob to chain…");
    hs.circuit
        .upload_circuit()
        .context("upload_circuit")?;
    eprintln!("upload_circuit ok");

    Ok(())
}
