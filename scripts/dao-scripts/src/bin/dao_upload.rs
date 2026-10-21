//! Upload the DAO contract set through one authz and feegrant session.
//!
//! The daemon wallet is the grantee. `--granter` is the cold Terp address.
//! The session is the wallet catalog (sdk, ibc, wasmd, and Terp modules),
//! so a later script message does not need its own grant. When the chain
//! does not already cover that catalog, the script opens
//! http://127.0.0.1:8741 once per Zigner code. Sign each transaction on
//! Zigner, paste the public key and signature, and the script broadcasts
//! it. Later messages in this process are `MsgExec`, paid by the feegrant.
//!
//! Terp stores `cw_tokenfactory_issuer-osmosis.wasm` (the chain module is the
//! Osmosis tokenfactory), `dao_voting_token_staked-default.wasm`, and
//! `cw_vesting-staking.wasm`. The Thorchain builds are not uploaded.
//!
//! `--spawn` boots the local Terp image (`240u-1`) instead of a mock chain,
//! funds the signer (`LOCAL_MNEMONIC`, or a fresh key when that is unset) and the granter from the genesis validator, and
//! runs `DaoDaoSuite::on_spawn` before upload. The container stops when the
//! process exits. `TERP_IMAGE_VERSION` selects the tag; the default is `v6.3.0`.
//!
//! ```sh
//! cargo run -p dao-scripts --bin dao_upload -- --granter terp1... --chain terp-mainnet
//! CW_ORCH_GRANTER=terp1... cargo run -p dao-scripts --bin dao_upload -- --spawn
//! ```

use anyhow::{anyhow, Result};
use clap::Parser;
use cw_orch::prelude::networks::{TERP_MAINNET, TERP_TESTNET};
use cw_orch::prelude::*;
use cw_orch_kms::{enable_approval, ApprovalSession, SpawnedTerp, CW_ORCH_GRANTER_ENV};
use dao_cw_orch::terp_wasm_file;
use dao_testing::DaoDaoSuite;

#[derive(Parser, Debug)]
#[clap(name = "dao-upload")]
struct Cli {
    /// Cold bech32 address. This key signs the grant. `CW_ORCH_GRANTER` is the default.
    #[clap(long, env = CW_ORCH_GRANTER_ENV)]
    granter: String,

    /// `terp-mainnet` / `morocco-1` or `terp-testnet` / `120u-1`. Ignored with `--spawn`.
    #[clap(long, default_value = "terp-mainnet")]
    chain: String,

    /// Boot the local Terp image, fund the signer and the granter, then run suite preflight.
    #[clap(long)]
    spawn: bool,

    /// How long the wallet grant and feegrant stay valid, in seconds.
    #[clap(long, default_value_t = 2 * 60 * 60)]
    ttl_secs: u64,

    /// Zigner account index for the Terp key. The QR uses this index.
    #[clap(long, default_value_t = 0u32)]
    account_index: u32,

    /// Fee allowance in the chain gas denom. The session wallet spends this.
    #[clap(long, default_value_t = 100_000_000u128)]
    allowance: u128,
}

fn resolve_chain(name: &str) -> Result<ChainInfo> {
    match name {
        "terp-mainnet" | "morocco-1" | "mainnet" => Ok(TERP_MAINNET),
        "terp-testnet" | "120u-1" | "testnet" => Ok(TERP_TESTNET),
        other => Err(anyhow!(
            "unknown chain {other:?}; use terp-mainnet / morocco-1 or terp-testnet / 120u-1"
        )),
    }
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    let _ = dotenv::dotenv();
    env_logger::init();

    let cli = Cli::parse();
    println!(
        "terp wasm: issuer={}, voting-token-staked={}, vesting={}",
        terp_wasm_file("cw_tokenfactory_issuer"),
        terp_wasm_file("dao_voting_token_staked"),
        terp_wasm_file("cw_vesting"),
    );
    println!(
        "skipped: dao_voting_token_staked-thorchain.wasm, cw_tokenfactory_issuer-thorchain.wasm"
    );
    let spawned = if cli.spawn {
        let chain = SpawnedTerp::boot().map_err(|err| anyhow!(err))?;
        chain.fund(&cli.granter).map_err(|err| anyhow!(err))?;
        println!("funded granter {}", cli.granter);
        Some(chain)
    } else {
        None
    };
    let chain = if let Some(spawned) = &spawned {
        spawned.chain_info()
    } else {
        resolve_chain(&cli.chain)?.into()
    };

    let mut session = ApprovalSession::wallet(cli.granter);
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cli.allowance;

    let mut builder = Daemon::builder(chain);
    if spawned.is_some() {
        builder.is_test(true).load_network(false);
    }
    let daemon = enable_approval(&builder, session).map_err(|err| anyhow!(err))?;
    let dao = DaoDaoSuite::new(daemon.clone());
    if spawned.is_some() {
        dao.on_spawn()?;
        println!("spawn preflight: signer has a spendable balance");
    }
    dao.upload_if_needed()?;

    let ids = daemon.state().get_all_code_ids()?;
    println!("{ids:#?}");
    Ok(())
}
