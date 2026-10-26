//! Liquidity-steward multi-DAO: the Foundation proposal that instantiate2s
//! the two steward (oversight) DAOs and two custody DAOs, plus the follow-up
//! TERP subDAO proposals each custody DAO submits.
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
    assert_steward_admin, build_steward_plan_with_parent, decode_hex, load_steward_accounts,
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
    let note = match (&cli.polytone_note, cli.no_polytone) {
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
    let plan = build_steward_plan_with_parent(&accounts, &cli.foundation, &sum, note.as_deref(), None, parent.as_deref())
        .map_err(|e| anyhow!(e))?;
    let msgs: Vec<CosmosMsg> = serde_json::from_value(
        plan.foundation_proposal["propose"]["msg"]["propose"]["msgs"].clone(),
    )?;
    if msgs.len() != 4 {
        bail!("expected 4 foundation instantiate2 msgs, got {}", msgs.len());
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
        "invariants": "foundation is neither wasm nor DAO admin of any of the 4 DAOs; both admins equal the x/gov module parent (checked)",
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
