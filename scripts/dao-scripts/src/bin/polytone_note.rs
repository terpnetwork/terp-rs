//! Chain-parametric Polytone note on morocco-1 (cw-v3 codes from cw-orch state),
//! pinned to one of the new Terp connections: `--remote juno` (Terp client
//! 07-tendermint-46) or `--remote osmosis` (07-tendermint-44). Connection ids
//! and the remote voice are inputs; nothing is guessed.
//!
//! Dry-run (default) writes the plan: predicted note/listener (instantiate2 by
//! `--granter`), the pinned pair, the remote voice instantiate, and the channel
//! handshake Hermes must run. `--broadcast` sends the note (and listener)
//! instantiate2 through a cw-orch-kms session; it never opens channels.
//! Supersedes `polytone_osmosis` (kept for the legacy `--via-gov` path).

use anyhow::{bail, Result};
use clap::Parser;
use dao_foundation_suite::ibc_ops::native_delegation::PARENT_CORE;
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use dao_scripts::polytone as pt;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "polytone_note")]
struct Cli {
    /// juno | osmosis
    #[arg(long)]
    remote: String,
    /// morocco-1 connection id on the new client to that chain. Pins the note.
    #[arg(long)]
    connection: Option<String>,
    /// Remote-side connection id (counterparty of --connection). Used to predict proxies.
    #[arg(long)]
    remote_connection: Option<String>,
    /// Remote voice address. Pins the note's `remote_port` to `wasm.<voice>`.
    #[arg(long)]
    voice: Option<String>,
    /// Terp senders whose remote proxies should be predicted (e.g. steward-juno).
    #[arg(long = "proxy-for")]
    proxy_for: Vec<String>,
    #[arg(long)]
    state: Option<PathBuf>,
    #[arg(long)]
    note_code_id: Option<u64>,
    #[arg(long)]
    listener_code_id: Option<u64>,
    /// Admin of the note (and listener).
    #[arg(long, default_value = PARENT_CORE)]
    admin: String,
    /// Allow an unpinned note (first channel handshake wins). Off by default.
    #[arg(long)]
    allow_unpinned: bool,
    #[arg(long)]
    skip_verify: bool,
    #[arg(long)]
    broadcast: bool,
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long, default_value = "")]
    granter: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 1800)]
    ttl_secs: u64,
    #[arg(long)]
    out: Option<String>,
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let r = pt::remote(&cli.remote)?;
    let out = cli.out.clone().unwrap_or_else(|| format!("out/polytone_note_{}.json", r.chain_id));
    let morocco = kms_gate::morocco_info();
    let remote_port = cli.voice.as_ref().map(|v| format!("wasm.{v}"));
    if let Some(v) = &cli.voice {
        if !v.starts_with(&format!("{}1", r.prefix)) {
            bail!("voice {v} is not a {} address", r.prefix);
        }
    }
    let pair = match (&cli.connection, &remote_port) {
        (Some(c), Some(p)) => Some((c.as_str(), p.as_str())),
        (None, None) => None,
        _ => bail!("pass both --connection and --voice, or neither"),
    };
    let blocked_unpinned = pair.is_none() && !cli.allow_unpinned;

    let state = pt::codes_from_state("morocco-1", cli.state.as_deref())?;
    let note_code = cli.note_code_id.unwrap_or(state.note);
    let listener_code = cli.listener_code_id.or(state.listener);
    let sender = if cli.granter.is_empty() { None } else { Some(cli.granter.as_str()) };
    let note = sender.map(|s| pt::predict_for(s, pt::V3_NOTE_CHECKSUM, &r.note_salt())).transpose()?;
    let note_msg = pt::note_init(pair);
    let mut msgs = vec![];
    let mut verify_ids = vec![(note_code, pt::V3_NOTE_CHECKSUM)];
    if let Some(s) = sender {
        msgs.push(pt::instantiate2_any(s, &cli.admin, note_code, &r.note_label(), &note_msg, &r.note_salt())?);
    }
    let mut listener = serde_json::json!({"skipped": "no polytone:listener code on morocco-1"});
    if let Some(lc) = listener_code {
        verify_ids.push((lc, pt::V3_LISTENER_CHECKSUM));
        let lmsg = serde_json::json!({"note": note.clone().unwrap_or_else(|| "<predicted note>".into())});
        if let Some(s) = sender {
            msgs.push(pt::instantiate2_any(s, &cli.admin, lc, &r.listener_label(), &lmsg, &r.listener_salt())?);
        }
        let pred = sender.map(|s| pt::predict_for(s, pt::V3_LISTENER_CHECKSUM, &r.listener_salt())).transpose()?;
        listener = serde_json::json!({"code_id": lc, "label": r.listener_label(), "salt": r.listener_salt(), "msg": lmsg, "predicted": pred});
    }
    let proxies: serde_json::Value = match (&cli.voice, &cli.remote_connection, &note) {
        (Some(v), Some(rc), Some(n)) => cli.proxy_for.iter()
            .map(|s| Ok((s.clone(), serde_json::json!(pt::predict_proxy(v, rc, n, s)?))))
            .collect::<Result<serde_json::Map<_, _>>>()?.into(),
        _ => serde_json::json!({"pending": "needs --voice, --remote-connection and --granter"}),
    };
    let verify = if cli.skip_verify {
        serde_json::json!("skipped")
    } else {
        let md = kms_gate::query_only(&morocco)?;
        let q = cw_orch::daemon::queriers::CosmWasm::new_async(md.channel());
        let mut m = serde_json::Map::new();
        for (id, want) in &verify_ids {
            let got: String = md.rt_handle.block_on(q._code(*id))?.checksum.as_slice().iter().map(|b| format!("{b:02x}")).collect();
            if got != *want {
                bail!("morocco-1 code {id} checksum {got} != {want}");
            }
            m.insert(id.to_string(), got.into());
        }
        serde_json::json!({"morocco-1": m})
    };
    let note_port = note.as_ref().map(|n| format!("wasm.{n}")).unwrap_or_else(|| "wasm.<note>".into());
    kms_gate::write_plan(&out, &serde_json::json!({
        "dry_run": !cli.broadcast,
        "remote": r.chain_id,
        "terp_client": r.terp_client,
        "verify": verify,
        "morocco-1": {
            "sender": sender, "admin": cli.admin,
            "note": {"code_id": note_code, "label": r.note_label(), "salt": r.note_salt(), "msg": note_msg, "predicted": note},
            "listener": listener,
            "pinned": pair.is_some(),
        },
        "remote_side": {
            "codes": {"listener": r.listener_code, "note": r.note_code, "proxy": r.proxy_code, "voice": r.voice_code},
            "voice": cli.voice,
            "voice_instantiate": {"code_id": r.voice_code, "label": "polytone_voice_from_morocco-1", "admin": null, "msg": r.voice_init(), "signer": format!("a {} account (not this script)", r.prefix)},
            "remote_connection": cli.remote_connection,
            "predicted_proxies": proxies,
        },
        "channel": {"a": note_port, "b": remote_port.clone().unwrap_or_else(|| format!("wasm.<{} voice>", r.chain_id)),
            "connection": cli.connection, "version": pt::CHANNEL_VERSION, "ordering": "unordered",
            "needs": format!("Hermes: connection on {} (both sides), then chan-open on the ports above", r.terp_client)},
        "blocker": if blocked_unpinned { serde_json::json!("note not pinned: pass --connection and --voice once they exist") } else { serde_json::Value::Null },
    }))?;
    if !cli.broadcast {
        return Ok(());
    }
    if blocked_unpinned {
        bail!("broadcast refused: the note must be pinned (--connection, --voice)");
    }
    if sender.is_none() || msgs.is_empty() {
        bail!("broadcast refused: pass --granter");
    }
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info: morocco,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: format!("polytone-note-{}", r.chain_id),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Instantiate2,
            summary: format!("Instantiate cw-v3 Polytone note to {}", r.chain_id),
        },
        msgs,
        "polytone_note",
    )?;
    println!("tx {tx}");
    Ok(())
}
