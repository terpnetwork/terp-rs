//! Liquidity-steward multi-DAO. Default (tuned) structure: the Foundation
//! proposal instantiate2s steward-akash, custody-akash and steward-juno on Terp
//! (x/gov module parent). Follow-ups: custody-akash opens a Polytone account on
//! Osmosis; steward-juno creates custody-juno on Juno through its Polytone proxy
//! (Juno x/gov = wasm admin, dao admin, vetoer; proxy = sole member).
//! `--legacy-four` restores the old four-DAO plan with custody-juno on Terp.
//!
//! cw-orch runtime. The dao-dao-core checksum comes from cw-orch's read-only
//! gRPC querier (or `--checksum`). Default dry-run writes `out/steward/` and
//! checks that the Foundation is never wasm or DAO admin. `--broadcast` sends
//! only the Foundation `propose` to PRE_PROPOSE_A through a cw-orch-kms
//! session scoped to that contract, key `propose`, one call. Custody-side
//! proposals are later votes of those DAOs, not run here.

use anyhow::{anyhow, bail, Result};
use clap::Parser;
use cosmwasm_std::CosmosMsg;
use cw_orch_kms::ScopedCall;
use dao_foundation_suite::ibc_ops::native_delegation::{CODE_CORE, PARENT_CORE, PRE_PROPOSE_A};
use dao_foundation_suite::ibc_ops::steward::{
    assert_steward_admin, build_steward_plan_tuned, build_steward_plan_with_parent, decode_bech32,
    decode_hex, load_steward_accounts, JunoRemote, JUNO_DAO_CODES, JUNO_GOV,
};
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "steward_multi_dao")]
struct Cli {
    /// dao-dao-core checksum hex. Queried over gRPC when omitted.
    #[arg(long)]
    checksum: Option<String>,
    #[arg(long, default_value_t = CODE_CORE)]
    core_code_id: u64,
    #[arg(long, default_value = PARENT_CORE)]
    foundation: String,
    #[arg(long, default_value = PRE_PROPOSE_A)]
    pre_propose: String,
    #[arg(long)]
    accounts: Option<PathBuf>,
    #[arg(long)]
    polytone_note: Option<String>,
    /// Do not default to the predicted morocco-1 Polytone note.
    #[arg(long)]
    no_polytone: bool,
    /// Parent (dao admin + wasm admin) of the steward and custody DAOs. Default:
    /// the x/gov module account, queried from the chain (auth ModuleAccountByName "gov").
    #[arg(long)]
    gov_parent: Option<String>,
    /// Keep the four DAOs self-admin (old behaviour).
    #[arg(long)]
    no_gov_parent: bool,
    /// Old plan: four DAOs on Terp including custody-juno.
    #[arg(long)]
    legacy_four: bool,
    /// Terp note pinned to the new Terp<->Juno connection (polytone_note --remote juno).
    #[arg(long)]
    juno_note: Option<String>,
    /// Juno voice paired with --juno-note.
    #[arg(long)]
    juno_voice: Option<String>,
    /// Juno-side connection id (counterparty of the Terp connection on 07-tendermint-46).
    #[arg(long)]
    juno_connection: Option<String>,
    /// Juno x/gov module account. Default: queried on juno-1, else the known address.
    #[arg(long)]
    juno_gov: Option<String>,
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
    #[arg(long, default_value = "out/steward")]
    out_dir: String,
}

fn checksum(cli: &Cli, info: &cw_orch::environment::ChainInfoOwned) -> Result<Vec<u8>> {
    if let Some(h) = &cli.checksum {
        return decode_hex(h).map_err(|e| anyhow!(e));
    }
    let d = kms_gate::query_only(info)?;
    let code = d
        .rt_handle
        .block_on(cw_orch::daemon::queriers::CosmWasm::new_async(d.channel())._code(cli.core_code_id))
        .map_err(|e| anyhow!("code {} info: {e}", cli.core_code_id))?;
    Ok(code.checksum.as_slice().to_vec())
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let morocco = kms_gate::morocco_info();
    // Default: the gov-instantiate2 note to osmosis-1 predicted by polytone_osmosis
    // (DAO-DAO code ids; not instantiated until that gov proposal passes).
    let note = match (&cli.polytone_note, cli.no_polytone || !cli.legacy_four) {
        (_, true) => None,
        (Some(n), _) => Some(n.clone()),
        (None, false) => {
            let n = dao_scripts::polytone::predicted_terp_note(&kms_gate::module_address("gov", "terp")?)?.note;
            eprintln!("polytone note {n} (legacy gov prediction; the cw-v3 note address depends on the polytone_osmosis granter, pass --polytone-note)");
            Some(n)
        }
    };
    let accounts_path = cli.accounts.clone().unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../steward-accounts.json")
    });
    let accounts = load_steward_accounts(&accounts_path).map_err(|e| anyhow!(e))?;
    let sum = checksum(&cli, &morocco)?;
    let parent = if cli.no_gov_parent {
        None
    } else if let Some(p) = &cli.gov_parent {
        Some(p.clone())
    } else {
        Some(kms_gate::query_module_account(&kms_gate::query_only(&morocco)?, "gov", "terp")?)
    };
    if parent.as_deref() == Some(cli.foundation.as_str()) {
        bail!("gov parent equals the Foundation DAO");
    }
    eprintln!("steward parent: {parent:?} (x/gov module account)");
    let mut juno_info = serde_json::Value::Null;
    let (plan, expect) = if cli.legacy_four {
        (build_steward_plan_with_parent(&accounts, &cli.foundation, &sum, note.as_deref(), None, parent.as_deref())
            .map_err(|e| anyhow!(e))?, 4)
    } else {
        let juno_gov = match &cli.juno_gov {
            Some(g) => g.clone(),
            None => {
                let mut ji: cw_orch::environment::ChainInfoOwned = cw_orch::daemon::networks::JUNO_1.into();
                if let Ok(g) = std::env::var("JUNO_GRPC") { ji.grpc_urls = vec![g]; }
                match kms_gate::query_only(&ji).and_then(|d| kms_gate::query_module_account(&d, "gov", "juno")) {
                    Ok(g) => g,
                    Err(e) => { eprintln!("juno gov query failed ({e}); using {JUNO_GOV}"); JUNO_GOV.to_string() }
                }
            }
        };
        if juno_gov != JUNO_GOV || kms_gate::module_address("gov", "juno")? != JUNO_GOV {
            bail!("juno gov module {juno_gov} != expected {JUNO_GOV}");
        }
        let creator_bytes = decode_bech32(&cli.foundation).map_err(|e| anyhow!(e))?.1;
        let steward_juno = cw_orch_kms::instantiate2_bech32("terp", &sum, &creator_bytes, b"steward-juno").map_err(|e| anyhow!(e))?;
        let proxy = match (&cli.juno_voice, &cli.juno_connection, &cli.juno_note) {
            (Some(v), Some(c), Some(n)) => Some(dao_scripts::polytone::predict_proxy(v, c, n, &steward_juno)?),
            _ => None,
        };
        juno_info = serde_json::json!({"gov": juno_gov, "note": cli.juno_note, "voice": cli.juno_voice, "connection": cli.juno_connection, "steward_juno_proxy": proxy, "codes": "DAO-DAO v2.7.0 juno-1: core 4862, voting_cw4 4872, cw4_group 1992, proposal_single 4869"});
        let jr = JunoRemote { note: cli.juno_note.as_deref(), proxy: proxy.as_deref(), juno_gov: &juno_gov, codes: JUNO_DAO_CODES };
        (build_steward_plan_tuned(&accounts, &cli.foundation, &sum, cli.polytone_note.as_deref(), parent.as_deref(), &jr)
            .map_err(|e| anyhow!(e))?, 3)
    };
    let msgs: Vec<CosmosMsg> = serde_json::from_value(
        plan.foundation_proposal["propose"]["msg"]["propose"]["msgs"].clone(),
    )?;
    if msgs.len() != expect {
        bail!("expected {expect} foundation instantiate2 msgs, got {}", msgs.len());
    }
    for m in &msgs {
        assert_steward_admin(m, &cli.foundation, parent.as_deref()).map_err(|e| anyhow!(e))?;
    }
    std::fs::create_dir_all(&cli.out_dir)?;
    for s in &plan.subdaos {
        std::fs::write(format!("{}/{}.json", cli.out_dir, s.file_stem), serde_json::to_string_pretty(&s.body)?)?;
    }
    kms_gate::write_plan(&format!("{}/plan.json", cli.out_dir), &serde_json::json!({
        "dry_run": !cli.broadcast,
        "chain_id": morocco.chain_id,
        "checksum": plan.checksum_hex,
        "creator": plan.creator,
        "addresses": plan.addresses,
        "foundation_contract": cli.pre_propose,
        "foundation_proposal": plan.foundation_proposal,
        "subdao_files": plan.subdaos.iter().map(|s| &s.file_stem).collect::<Vec<_>>(),
        "parent": parent,
        "structure": if cli.legacy_four { "legacy-four" } else { "tuned: steward-akash + custody-akash + steward-juno on Terp; custody-juno on Juno via Polytone" },
        "juno": juno_info,
        "invariants": "foundation is neither wasm nor DAO admin of any Terp DAO; both admins equal the x/gov module parent (checked)",
        "session_scope": {"allow_execute": {"address": cli.pre_propose, "keys": ["propose"], "calls": 1}, "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
    }))?;
    if !cli.broadcast {
        return Ok(());
    }
    if cli.granter.is_empty() {
        bail!("broadcast refused: pass --granter (a Foundation member)");
    }
    let any = kms_gate::execute_any(&cli.granter, &cli.pre_propose, &plan.foundation_proposal)?;
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info: morocco,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "steward-multi-dao".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Execute(ScopedCall { address: cli.pre_propose.clone(), keys: vec!["propose".into()], calls: 1, funds: vec![] }),
            summary: "Foundation proposal: liquidity steward + custody DAOs".into(),
        },
        vec![any],
        "steward_multi_dao",
    )?;
    println!("tx {tx}");
    Ok(())
}
