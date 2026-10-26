//! Terp (morocco-1) -> Osmosis (osmosis-1) Polytone note.
//!
//! Default: the cw-v3 Polytone codes already stored on morocco-1 (instantiate
//! permission Everybody). Code ids come from cw-orch state
//! (`morocco-1.code_ids.polytone:*`), and their checksums are checked on chain.
//! A fresh note is instantiate2'd through cw-orch-kms, sent by the granter,
//! with the Foundation DAO as admin. No gov proposal, no upload.
//!
//! The listener is optional: morocco-1 state has no `polytone:listener`. Pass
//! `--listener-code-id` once one is stored.
//!
//! `--via-gov` keeps the old path: gov instantiate2 of the Nobody-gated
//! DAO-DAO codes 4/5 inside a `MsgSubmitProposal`.
//!
//! The Osmosis side stays DAO-DAO's (listener 125, note 126, proxy 127, voice
//! 128). Its voice is polytone 1.0.0 and speaks the same handshake
//! (`polytone-1`, extension `JSON-CosmosMsg`) as the v3 note (cw2 1.0.2).
//!
//! Dry-run by default. `--broadcast --chain-id morocco-1` with
//! TERP_SCRIPT_BROADCAST=1 goes through kms_gate.

use anyhow::{bail, Result};
use clap::Parser;
use dao_foundation_suite::ibc_ops::PARENT_CORE;
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use dao_scripts::polytone as pt;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "polytone_osmosis")]
struct Cli {
    /// cw-orch state file (default: $STATE_FILE, ~/.cw-orch/state.json, public/state.json).
    #[arg(long)]
    state: Option<PathBuf>,
    /// Override the note code id from state.
    #[arg(long)]
    note_code_id: Option<u64>,
    /// Listener code id on morocco-1. Without it no listener is instantiated.
    #[arg(long)]
    listener_code_id: Option<u64>,
    /// Admin of the new note (and listener): the Foundation DAO by default.
    /// Admin rotation is planned: the Foundation DAO holds these roles only while we deploy and bootstrap.
    #[arg(long, default_value = PARENT_CORE)]
    admin: String,
    /// morocco-1 connection to osmosis-1 (pins the note). Unset until it exists.
    #[arg(long)]
    connection: Option<String>,
    /// Osmosis voice address, once instantiated.
    #[arg(long)]
    voice: Option<String>,
    /// Legacy: gov proposal instantiating the Nobody-gated codes 4/5.
    #[arg(long)]
    via_gov: bool,
    #[arg(long, default_value_t = 5_000_000_000)]
    deposit_uterp: u128,
    #[arg(long)]
    skip_verify: bool,
    #[arg(long)]
    broadcast: bool,
    #[arg(long, default_value = "")]
    chain_id: String,
    /// kms granter. Sends the instantiate2, so it fixes the note address.
    #[arg(long, default_value = "")]
    granter: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 1800)]
    ttl_secs: u64,
    #[arg(long, default_value = "out/polytone_osmosis.json")]
    out: String,
}

fn checksums(d: &cw_orch::daemon::senders::QueryOnlyDaemon, ids: &[(u64, &str)]) -> Result<serde_json::Value> {
    let q = cw_orch::daemon::queriers::CosmWasm::new_async(d.channel());
    let mut out = serde_json::Map::new();
    for (id, want) in ids {
        let got = d.rt_handle.block_on(q._code(*id))?.checksum.as_slice().iter().map(|b| format!("{:02x}", b)).collect::<String>();
        if got != *want {
            bail!("code {id} checksum {got} != {want}");
        }
        out.insert(id.to_string(), serde_json::json!(got));
    }
    Ok(serde_json::Value::Object(out))
}

struct Built {
    msgs: Vec<cosmrs::Any>,
    scope: Scope,
    title: &'static str,
    plan: serde_json::Value,
    note: Option<String>,
    verify_ids: Vec<(u64, &'static str)>,
}

fn build_v3(cli: &Cli, pair: Option<(&str, &str)>) -> Result<Built> {
    let state = pt::codes_from_state("morocco-1", cli.state.as_deref())?;
    let note_code = cli.note_code_id.unwrap_or(state.note);
    let listener_code = cli.listener_code_id.or(state.listener);
    let sender = if cli.granter.is_empty() { None } else { Some(cli.granter.as_str()) };
    let note = sender.map(|s| pt::predict_for(s, pt::V3_NOTE_CHECKSUM, pt::V3_NOTE_SALT)).transpose()?;
    let listener = match (sender, listener_code) {
        (Some(s), Some(_)) => Some(pt::predict_for(s, pt::V3_LISTENER_CHECKSUM, pt::V3_LISTENER_SALT)?),
        _ => None,
    };
    let note_msg = pt::note_init(pair);
    let mut msgs = vec![];
    let mut verify_ids = vec![(note_code, pt::V3_NOTE_CHECKSUM)];
    if let Some(v) = state.voice { verify_ids.push((v, pt::V3_VOICE_CHECKSUM)); }
    if let Some(p) = state.proxy { verify_ids.push((p, pt::V3_PROXY_CHECKSUM)); }
    if let Some(s) = sender {
        msgs.push(pt::instantiate2_any(s, &cli.admin, note_code, pt::NOTE_LABEL, &note_msg, pt::V3_NOTE_SALT)?);
    }
    let mut listener_plan = serde_json::json!({"skipped": "no polytone:listener code on morocco-1 (state or --listener-code-id)"});
    if let Some(lc) = listener_code {
        verify_ids.push((lc, pt::V3_LISTENER_CHECKSUM));
        let lmsg = serde_json::json!({"note": note.clone().unwrap_or_else(|| "<predicted note>".into())});
        if let Some(s) = sender {
            msgs.push(pt::instantiate2_any(s, &cli.admin, lc, pt::LISTENER_LABEL, &lmsg, pt::V3_LISTENER_SALT)?);
        }
        listener_plan = serde_json::json!({"code_id": lc, "label": pt::LISTENER_LABEL, "salt": pt::V3_LISTENER_SALT, "msg": lmsg, "predicted": listener});
    }
    Ok(Built {
        plan: serde_json::json!({
            "path": "cw-v3 codes, direct instantiate2 via cw-orch-kms (no gov)",
            "state": {"source": state.source, "note": state.note, "voice": state.voice, "proxy": state.proxy, "listener": state.listener},
            "sender": sender.unwrap_or("<granter> (pass --granter to predict addresses)"),
            "admin": cli.admin,
            "note": {"code_id": note_code, "label": pt::NOTE_LABEL, "salt": pt::V3_NOTE_SALT, "msg": note_msg, "predicted": note},
            "listener": listener_plan,
            "session_scope": {"allow": "MsgInstantiateContract2", "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
        }),
        note,
        msgs,
        scope: Scope::Instantiate2,
        title: "Instantiate cw-v3 Polytone note to osmosis-1",
        verify_ids,
    })
}

fn build_gov(cli: &Cli, pair: Option<(&str, &str)>) -> Result<Built> {
    let gov = kms_gate::module_address("gov", "terp")?;
    let addrs = pt::predicted_terp_note(&gov)?;
    let title = "Instantiate the Polytone note to osmosis-1";
    let summary = format!("Gov instantiate2 of polytone note (code {}) and listener (code {}). Predicted note {}, listener {}. Admin is the gov module.",
        pt::TERP_NOTE_CODE, pt::TERP_LISTENER_CODE, addrs.note, addrs.listener);
    let mut msgs = vec![];
    if !cli.granter.is_empty() {
        msgs.push(pt::gov_submit_any(pt::terp_note_msgs(&gov, pair)?, &cli.granter, (cli.deposit_uterp, "uterp"), title, &summary)?);
    }
    Ok(Built {
        plan: serde_json::json!({
            "path": "legacy gov proposal (--via-gov), DAO-DAO codes 4/5 (permission Nobody)",
            "predicted_note": addrs.note, "predicted_listener": addrs.listener,
            "gov_proposal": {"title": title, "summary": summary, "deposit": format!("{}uterp", cli.deposit_uterp)},
        }),
        note: Some(addrs.note),
        msgs,
        scope: Scope::Sdk(vec!["/cosmos.gov.v1.MsgSubmitProposal".into()]),
        title,
        verify_ids: vec![(pt::TERP_NOTE_CODE, pt::NOTE_CHECKSUM), (pt::TERP_LISTENER_CODE, pt::LISTENER_CHECKSUM)],
    })
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let morocco = kms_gate::morocco_info();
    let remote_port = cli.voice.as_ref().map(|v| format!("wasm.{v}"));
    let pair = match (&cli.connection, &remote_port) {
        (Some(c), Some(p)) => Some((c.as_str(), p.as_str())),
        (None, None) => None,
        _ => bail!("pass both --connection and --voice, or neither"),
    };
    let built = if cli.via_gov { build_gov(&cli, pair)? } else { build_v3(&cli, pair)? };
    let verify = if cli.skip_verify {
        serde_json::json!("skipped")
    } else {
        let mut osmo: cw_orch::environment::ChainInfoOwned = cw_orch::daemon::networks::OSMOSIS_1.into();
        if let Ok(g) = std::env::var("OSMOSIS_GRPC") {
            osmo.grpc_urls = vec![g];
        }
        let od = kms_gate::query_only(&osmo)?;
        let ref_voice = od.rt_handle.block_on(
            cw_orch::daemon::queriers::CosmWasm::new_async(od.channel())
                ._contract_info(&cosmwasm_std::Addr::unchecked(pt::OSMO_REFERENCE_VOICE)),
        )?;
        if ref_voice.code_id != pt::OSMO_VOICE_CODE {
            bail!("DAO-DAO reference voice is code {}, expected {}", ref_voice.code_id, pt::OSMO_VOICE_CODE);
        }
        let osmo_sums = checksums(&od, &[(pt::OSMO_LISTENER_CODE, pt::LISTENER_CHECKSUM), (pt::OSMO_NOTE_CODE, pt::NOTE_CHECKSUM), (pt::OSMO_PROXY_CODE, pt::PROXY_CHECKSUM), (pt::OSMO_VOICE_CODE, pt::VOICE_CHECKSUM)])?;
        drop(od);
        let md = kms_gate::query_only(&morocco)?;
        serde_json::json!({
            "morocco-1": checksums(&md, &built.verify_ids)?,
            "osmosis-1": osmo_sums,
            "osmosis_reference_voice": {"address": pt::OSMO_REFERENCE_VOICE, "code_id": ref_voice.code_id},
        })
    };
    let note_port = built.note.as_ref().map(|n| format!("wasm.{n}")).unwrap_or_else(|| "wasm.<note>".into());
    kms_gate::write_plan(&cli.out, &serde_json::json!({
        "dry_run": !cli.broadcast,
        "source": {"osmosis_codes": pt::DAO_DAO_POLYTONE_JSON, "chains_ts": pt::DAO_DAO_CHAINS_TS},
        "verify": verify,
        "morocco-1": built.plan,
        "msgs": built.msgs.iter().map(|m| m.type_url.clone()).collect::<Vec<_>>(),
        "osmosis-1": {"codes": {"listener": pt::OSMO_LISTENER_CODE, "note": pt::OSMO_NOTE_CODE, "proxy": pt::OSMO_PROXY_CODE, "voice": pt::OSMO_VOICE_CODE},
            "voice_instantiate": {"code_id": pt::OSMO_VOICE_CODE, "label": pt::VOICE_LABEL, "admin": null, "msg": pt::voice_init()},
            "signer": "an osmo1 account (not this script)"},
        "channel": {"a": note_port, "b": remote_port.unwrap_or_else(|| "wasm.<osmosis voice>".into()), "version": pt::CHANNEL_VERSION, "ordering": "unordered",
            "needs": "a morocco-1 <-> osmosis-1 connection and a relayer"},
    }))?;
    if !cli.broadcast {
        return Ok(());
    }
    if cli.granter.is_empty() || built.msgs.is_empty() {
        bail!("broadcast refused: pass --granter");
    }
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info: morocco,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "polytone-osmosis".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: built.scope,
            summary: built.title.into(),
        },
        built.msgs,
        "polytone_osmosis",
    )?;
    println!("tx {tx}");
    Ok(())
}
