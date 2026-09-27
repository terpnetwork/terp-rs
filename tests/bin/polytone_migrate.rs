//! polytone_migrate — upload Polytone via `cw_orch_polytone::Polytone` and
//! optionally register / instantiate on Terp.
//!
//! This is the Terp-scripts sibling of `DaoPolytoneSuite`. Both call the same
//! cw-orch suite (`Polytone::store_if_needed` / `store_on`).
//!
//! ```sh
//! cargo run -p terp-scripts --bin polytone_migrate -- --chain terp-testnet
//! cargo run -p terp-scripts --bin polytone_migrate -- --chain terp-mainnet
//! ```
//!
//! Default: upload note / voice / proxy only, no instantiate, no admin.
//! `--full-connection` instantiates note+voice with admin = None unless `--admin` is set.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use cw_orch::prelude::networks::{TERP_MAINNET, TERP_TESTNET};
use cw_orch::prelude::*;
use cw_orch::tokio::runtime::Runtime;
use cw_orch_polytone::Polytone;
use log::info;

#[derive(Parser, Debug)]
#[clap(name = "polytone-migrate")]
pub struct Cli {
    /// `terp-mainnet` / `morocco-1` or `terp-testnet` / `120u-1`
    #[clap(long, short)]
    pub chain: Option<String>,

    /// Remote chain id for IBC (unused unless `--full-connection`)
    #[clap(long, short = 'r')]
    pub remote_chain: Option<String>,

    /// Skip upload when code IDs already exist in cw-orch state (default true)
    #[clap(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub upload_if_needed: bool,

    /// Also instantiate note + voice (admin defaults to none / immutable)
    #[clap(long, short = 'f')]
    pub full_connection: bool,

    /// Optional instantiate admin. Omit for immutable (no admin).
    #[clap(long)]
    pub admin: Option<String>,

    /// Abstract IBC client (informational / future infrastructure update)
    #[clap(long)]
    pub ibc_client: Option<String>,

    /// Abstract IBC host
    #[clap(long)]
    pub ibc_host: Option<String>,

    /// Abstract registry
    #[clap(long)]
    pub registry: Option<String>,
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

fn upload_polytone(daemon: Daemon, upload_if_needed: bool) -> Result<Polytone<Daemon>> {
    let polytone = if upload_if_needed {
        Polytone::store_if_needed(daemon)?
    } else {
        Polytone::store_on(daemon)?
    };
    let note = polytone.note.code_id()?;
    let voice = polytone.voice.code_id()?;
    let proxy = polytone.proxy.code_id()?;
    info!("Uploaded / loaded — note: {note}, voice: {voice}, proxy: {proxy}");
    println!("polytone:note={note}");
    println!("polytone:voice={voice}");
    println!("polytone:proxy={proxy}");
    Ok(polytone)
}

fn instantiate_note_and_voice(
    polytone: &Polytone<Daemon>,
    admin: Option<String>,
) -> Result<()> {
    info!(
        "Instantiating note + voice (admin={})",
        admin.as_deref().unwrap_or("<none/immutable>")
    );
    polytone.instantiate_note(admin.clone())?;
    polytone.instantiate_voice(admin)?;
    if let Ok(addr) = polytone.note.address() {
        info!("note address: {addr}");
    }
    if let Ok(addr) = polytone.voice.address() {
        info!("voice address: {addr}");
    }
    Ok(())
}

fn print_registry_hints(cli: &Cli, chain_id: &str) {
    info!(
        "Abstract registry is not auto-written for code-only polytone \
         (register_natives requires live addresses). After instantiate, \
         propose standalones / update factory+VC on {chain_id}."
    );
    if let Some(r) = &cli.registry {
        info!("registry hint: {r}");
    }
    if let Some(c) = &cli.ibc_client {
        info!("ibc-client hint: {c}");
    }
    if let Some(h) = &cli.ibc_host {
        info!("ibc-host hint: {h}");
    }
}

fn main() -> Result<()> {
    let _ = dotenv::dotenv();
    env_logger::init();

    let cli = Cli::parse();
    let chain_name = cli.chain.as_deref().unwrap_or("terp-testnet");
    let chain = resolve_chain(chain_name)?;
    let chain_id = chain.chain_id;
    info!("=== Polytone migrate via cw_orch_polytone::Polytone on {} ===", chain_id);

    let rt = Runtime::new().context("tokio runtime")?;
    let daemon = DaemonBuilder::new(chain)
        .handle(rt.handle())
        .build()
        .context("DaemonBuilder (need deployer key / mnemonic in env)")?;

    info!("[Phase A] store note/voice/proxy");
    let polytone = upload_polytone(daemon, cli.upload_if_needed)?;

    info!("[Phase B] Abstract registry (hints only for upload-only)");
    print_registry_hints(&cli, chain_id);

    if cli.full_connection {
        info!("[Phase D] instantiate (immutable unless --admin)");
        instantiate_note_and_voice(&polytone, cli.admin.clone())?;
        if cli.remote_chain.is_some() {
            info!(
                "[Phase E] IBC channel not auto-created here; use polytone connect / hermes"
            );
        }
    }

    info!("[Phase F] verify code IDs");
    polytone.note.code_id()?;
    polytone.voice.code_id()?;
    polytone.proxy.code_id()?;
    info!("=== done — cw-orch state.json should now have polytone:* for {} ===", chain_id);
    Ok(())
}
