//! Store the app contracts through one authz and feegrant session.
//!
//! The cold key signs one grant on the approval page (random loopback port, printed at start). The grant stores
//! only these Wasm files, pinned by checksum. The ephemeral key then stores
//! them and, unless `--retain`, revokes the grant.
//!
//! The set is the contracts to exercise now: cw721-svg, its auction, bounty
//! escrow, the reserve auction, the raffle, the marketplace, the passkey
//! authenticator, the hash-based recovery authenticator, and the oracle
//! registry. This script does not instantiate them.
//!
//! On groot-wan, pass `--spawn`. That boots the local Terp image (`240u-1`)
//! and funds the signer and the granter. The container stops when the process
//! exits. `TERP_IMAGE_VERSION` selects the tag; the default is `v6.3.0`.
//!
//! Wasm is read from `ARTIFACTS_DIR`, or from `artifacts/` in this workspace.
//!
//! The granter is the airgapped key that signs `cw-orch-granter:v1`. Leave
//! `--granter` empty. `CW_ORCH_GRANTER` still names one when you already know it.
//!
//! ```sh
//! cargo run -p dao-scripts --bin app_upload -- --spawn
//! ```

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use clap::Parser;
use cw_orch::prelude::networks::{TERP_MAINNET, TERP_TESTNET};
use cw_orch::prelude::*;
use cw_orch_kms::{
    enable_session, planned_spend_limit, ApprovalSession, SpawnedTerp, SigningRequest,
    CW_ORCH_GRANTER_ENV,
};

/// Nine stores plus the closing revoke.
const TX_COUNT: u64 = 10;

/// Logical name and artifact file. The file name is the package name with hyphens folded.
const APPS: &[(&str, &str)] = &[
    ("cw721-svg", "cw721_svg.wasm"),
    ("svg-auction", "svg_auction.wasm"),
    ("bounty-escrow", "terp_bounty_escrow.wasm"),
    ("reserve-auction", "revenue_reserve_auction.wasm"),
    ("raffle", "nftfi_raffle.wasm"),
    ("marketplace", "revenue_marketplace.wasm"),
    ("passkey", "terp_passkey.wasm"),
    ("recovery", "terp_recovery.wasm"),
    ("oracle-registry", "cw_oracle_registry.wasm"),
];

#[derive(Parser, Debug)]
#[clap(name = "app-upload")]
struct Cli {
    /// Optional. When empty, the airgapped signature chooses the granter.
    #[clap(long, env = CW_ORCH_GRANTER_ENV, default_value = "")]
    granter: String,

    /// `terp-mainnet` / `morocco-1` or `terp-testnet` / `120u-1`. Ignored with `--spawn`.
    #[clap(long)]
    chain: Option<String>,

    /// Boot the local Terp image on this machine, then store the app Wasm.
    #[clap(long)]
    spawn: bool,

    /// Refused. A session names the Wasm it will store.
    #[clap(long)]
    wallet: bool,

    /// How long the wallet grant and feegrant stay valid, in seconds.
    #[clap(long, default_value_t = 2 * 60 * 60)]
    ttl_secs: u64,

    /// Zigner account index for the Terp key.
    #[clap(long, default_value_t = 0u32)]
    account_index: u32,

    /// Fee allowance in uterp. The default covers nine stores and the revoke.
    #[clap(long, default_value_t = planned_spend_limit(TX_COUNT))]
    allowance: u128,

    /// Leave the authz and feegrant in place.
    #[clap(long)]
    retain: bool,
}

fn artifacts_dir() -> PathBuf {
    std::env::var("ARTIFACTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts"))
}

fn read_apps(dir: &Path) -> Result<Vec<(&'static str, &'static str, Vec<u8>)>> {
    let mut out = Vec::with_capacity(APPS.len());
    for (name, file) in APPS {
        let path = dir.join(file);
        let bytes = std::fs::read(&path).map_err(|err| anyhow!("read {}: {err}", path.display()))?;
        if bytes.is_empty() {
            return Err(anyhow!("{} is empty", path.display()));
        }
        out.push((*name, *file, bytes));
    }
    Ok(out)
}

fn resolve_chain(name: &str) -> Result<ChainInfo> {
    match name {
        "terp-mainnet" | "morocco-1" | "mainnet" => Ok(TERP_MAINNET),
        "terp-testnet" | "120u-1" | "testnet" => Ok(TERP_TESTNET),
        other => Err(anyhow!(
            "unknown chain {other:?}; use terp-mainnet / morocco-1 or terp-testnet / 120u-1, or --spawn"
        )),
    }
}

fn store_with_retry(chain: &Daemon, file: &str, bytes_path: &Path) -> Result<u64> {
    let mut last = String::new();
    for attempt in 1..=3 {
        let wasm = WasmPath::new(bytes_path).map_err(|err| anyhow!("{file}: {err}"))?;
        match chain.rt_handle.block_on(cw_orch::daemon::upload_wasm(
            chain.sender(),
            wasm,
            None,
        )) {
            Ok(response) => {
                let code_id = response
                    .uploaded_code_id()
                    .map_err(|err| anyhow!("{file}: {err}"))?;
                println!("stored {file} code {code_id} tx {}", response.txhash);
                return Ok(code_id);
            }
            Err(err) => {
                last = err.to_string();
                let transient = last.contains("h2 protocol")
                    || last.contains("stream error")
                    || last.contains("connection reset")
                    || last.contains("error reading a body");
                if !transient || attempt == 3 {
                    return Err(anyhow!("{file}: {last}"));
                }
                println!("store {file} attempt {attempt} lost the chain connection. retrying");
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }
    Err(anyhow!("{file}: {last}"))
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();

    assert_eq!(APPS.len() as u64 + 1, TX_COUNT);
    let cli = Cli::parse();
    if cli.wallet {
        return Err(anyhow!(
            "--wallet is not a grant. This upload stores the app wasm files by checksum"
        ));
    }
    if !cli.spawn && cli.chain.is_none() {
        return Err(anyhow!(
            "pass --spawn on groot-wan, or name a chain with --chain"
        ));
    }

    let dir = artifacts_dir();
    let apps = read_apps(&dir)?;
    println!("artifacts {}", dir.display());
    for (name, file, bytes) in &apps {
        println!("  {name} {file} {} bytes", bytes.len());
    }

    let granter = if cli.granter.is_empty() {
        println!("sign once on the approval page below. that key is the granter");
        cw_orch_kms::capture_granter_sync(
            cw_orch_kms::page_addr(),
            cli.account_index,
            "app-upload",
        )
        .map_err(|err| anyhow!(err))?
    } else {
        cli.granter.clone()
    };
    println!("granter {granter}");
    let spawned = if cli.spawn {
        let chain = SpawnedTerp::boot().map_err(|err| anyhow!(err))?;
        chain.fund(&granter).map_err(|err| anyhow!(err))?;
        println!("funded granter {granter}");
        Some(chain)
    } else {
        None
    };
    let chain = if let Some(spawned) = &spawned {
        spawned.chain_info()
    } else {
        resolve_chain(cli.chain.as_deref().unwrap_or(""))?.into()
    };

    let mut session = ApprovalSession::open(&granter);
    for (_, _, bytes) in &apps {
        session = session.allow_store(bytes).map_err(|err| anyhow!(err))?;
    }
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cli.allowance;
    session.retain = cli.retain;

    let mut builder = Daemon::builder(chain);
    if spawned.is_some() {
        builder.is_test(true).load_network(false);
    }
    let actions = apps
        .iter()
        .map(|(_, file, _)| SigningRequest::new("session-key").body(format!("store {file}")))
        .collect();
    let run = enable_session(&builder, session, actions).map_err(|err| anyhow!(err))?;
    if spawned.is_some() {
        println!("spawn preflight: local chain is 240u-1");
    }
    println!("storing app contracts through the ephemeral key");
    for (_, file, _) in &apps {
        store_with_retry(&run.daemon, file, &dir.join(file))?;
    }
    println!("uploads finished");
    run.close().map_err(|err| anyhow!(err))?;
    Ok(())
}
