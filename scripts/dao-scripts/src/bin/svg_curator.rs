//! svg collection curator keys (the daily runtimes' signers) through cw-orch.
//!
//!   keygen   write a new curator mnemonic to <keys-dir>/<name>.mnemonic (0600, dir 0700).
//!            Prints only the public terp1 address. Refuses to overwrite.
//!   handoff  the curator moves wasm admin of collections it administers to --admin
//!            (MsgUpdateAdmin; the factory sets admin = creator). Collections already at
//!            --admin are skipped.
//!   daily    the curator (a series operator) creates the next series entry
//!            (create_series_collection from --msg-file), then hands its admin to --admin.
//!
//! Keys live in ~/.config/terp-svg-curators (the daily runtimes read the same files).
//! The mnemonic is read in-process and never printed or logged.
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use cw_orch::daemon::TxSender;
use cw_orch::environment::{ChainKind, WasmQuerier};
use cw_orch::prelude::*;
use dao_scripts::kms_gate;
use serde_json::Value;

#[derive(Parser)]
#[command(name = "svg_curator", about = "curator keys for the svg series: keygen | handoff | daily")]
struct Cli {
    cmd: String,
    #[arg(long, default_value = "~/.config/terp-svg-curators")]
    keys_dir: String,
    /// keygen: key name (benders | marain); handoff/daily: key to sign with.
    #[arg(long)]
    name: String,
    #[arg(long, default_value = "morocco-1")]
    network: String,
    #[arg(long)]
    grpc: String,
    #[arg(long)]
    rpc: Option<String>,
    #[arg(long)]
    lcd: Option<String>,
    #[arg(long, default_value = "uthiol")]
    gas_denom: String,
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
    /// New wasm admin (migration only): the Foundation DAO.
    #[arg(long, default_value = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w")]
    admin: String,
    /// handoff: collections (repeat).
    #[arg(long)]
    contract: Vec<String>,
    /// daily: svg-minter factory and the create_series_collection execute msg (JSON file).
    #[arg(long)]
    factory: Option<String>,
    #[arg(long)]
    msg_file: Option<PathBuf>,
    #[arg(long)]
    broadcast: bool,
}

fn keys_dir(c: &Cli) -> PathBuf {
    match c.keys_dir.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(&c.keys_dir),
    }
}

fn daemon(c: &Cli, mnemonic: String) -> Result<Daemon> {
    let kind = if c.network == "morocco-1" { ChainKind::Mainnet } else { ChainKind::Local };
    let info = kms_gate::chain_info(&c.network, &c.grpc, &c.gas_denom, c.gas_price, "terp", kind);
    let info = kms_gate::with_http_fallback(info, c.rpc.as_deref(), c.lcd.as_deref());
    let mut b = Daemon::builder(info);
    b.load_network(false);
    b.grpc_url(&c.grpc);
    b.mnemonic(mnemonic);
    Ok(b.build()?)
}

fn load(c: &Cli) -> Result<Daemon> {
    let p = keys_dir(c).join(format!("{}.mnemonic", c.name));
    let m = std::fs::read_to_string(&p).with_context(|| format!("curator key {}", p.display()))?;
    daemon(c, m.trim().to_string())
}

fn gate(c: &Cli) -> Result<()> {
    if !c.broadcast { bail!("dry run: add --broadcast (and {}=1) to send", kms_gate::BROADCAST_ENV); }
    if std::env::var(kms_gate::BROADCAST_ENV).ok().as_deref() != Some("1") { bail!("broadcast refused: set {}=1", kms_gate::BROADCAST_ENV); }
    if c.network != "morocco-1" && std::env::var(kms_gate::BROADCAST_CHAIN_ENV).ok().as_deref() != Some(c.network.as_str()) {
        bail!("broadcast refused: also set {}={}", kms_gate::BROADCAST_CHAIN_ENV, c.network);
    }
    Ok(())
}

fn handoff(d: &Daemon, admin: &str, contract: &str) -> Result<()> {
    let me = d.sender_addr().to_string();
    let info = d.wasm_querier().contract_info(&Addr::unchecked(contract)).map_err(|e| anyhow!("{contract}: {e}"))?;
    let cur = info.admin.map(|a| a.to_string());
    if cur.as_deref() == Some(admin) { println!("  {contract}: admin already {admin}"); return Ok(()); }
    if cur.as_deref() != Some(me.as_str()) { bail!("{contract}: admin {cur:?} is not this curator {me}"); }
    let msg = cosmos_sdk_proto::cosmwasm::wasm::v1::MsgUpdateAdmin { sender: me, new_admin: admin.into(), contract: contract.into() };
    let any = cosmrs::Any { type_url: "/cosmwasm.wasm.v1.MsgUpdateAdmin".into(), value: cosmos_sdk_proto::traits::Message::encode_to_vec(&msg) };
    let r = d.rt_handle.block_on(d.sender().commit_tx_any(vec![any], Some("svg curator: admin -> DAO")))?;
    if r.code != 0 { bail!("{contract}: MsgUpdateAdmin failed: {}", r.raw_log); }
    println!("  {contract}: admin -> {admin} tx {} gas {}", r.txhash, r.gas_used);
    Ok(())
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    env_logger::init();
    let c = Cli::parse();
    cw_orch_kms::validate_address(&c.admin, "terp").map_err(|e| anyhow!("--admin: {e}"))?;
    match c.cmd.as_str() {
        "keygen" => {
            let dir = keys_dir(&c);
            let p = dir.join(format!("{}.mnemonic", c.name));
            if p.exists() { bail!("{} exists; refusing to overwrite", p.display()); }
            write_new(&dir, &p)?;
            let m = std::fs::read_to_string(&p)?;
            let d = daemon(&c, m.trim().to_string())?;
            println!("curator {} address {} (key file {})", c.name, d.sender_addr(), p.display());
        }
        "address" => println!("curator {} address {}", c.name, load(&c)?.sender_addr()),
        "handoff" => {
            gate(&c)?;
            let d = load(&c)?;
            if c.contract.is_empty() { bail!("handoff needs --contract"); }
            for k in &c.contract { handoff(&d, &c.admin, k)?; }
        }
        "daily" => {
            gate(&c)?;
            let d = load(&c)?;
            let f = c.factory.clone().ok_or_else(|| anyhow!("daily needs --factory"))?;
            let p = c.msg_file.clone().ok_or_else(|| anyhow!("daily needs --msg-file"))?;
            let msg: Value = serde_json::from_str(&std::fs::read_to_string(&p)?)?;
            if msg.get("create_series_collection").is_none() { bail!("--msg-file must be a create_series_collection msg"); }
            let r = d.execute(&msg, &[], &Addr::unchecked(&f))?;
            let addr = r.event_attr_value("instantiate", "_contract_address").map_err(|e| anyhow!("{e}"))?;
            println!("  daily entry {addr} tx {} gas {}", r.txhash, r.gas_used);
            handoff(&d, &c.admin, &addr)?;
        }
        o => bail!("unknown command {o}"),
    }
    Ok(())
}

fn write_new(dir: &Path, p: &Path) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let m = bip39::Mnemonic::generate(24).map_err(|e| anyhow!("mnemonic: {e}"))?;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(p)?;
    f.write_all(m.to_string().as_bytes())?;
    Ok(())
}
