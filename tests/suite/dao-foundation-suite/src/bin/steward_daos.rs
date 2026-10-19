//! Dry-run the liquidity-steward DAO proposals.
//!
//! Writes the foundation proposal (two oversight DAOs and two custody DAOs)
//! and, per custody, the TERP subDAO proposal plus the Osmosis polytone
//! payloads. Sends nothing. There is no `--broadcast`.

use anyhow::{Context, Result, bail};
use clap::Parser;
use dao_foundation_suite::ibc_ops::native_delegation::{CODE_CORE, PARENT_CORE};
use dao_foundation_suite::ibc_ops::steward::{
    RemoteCodeIds, build_steward_plan, decode_hex, load_steward_accounts,
};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "steward_daos")]
struct Args {
    #[arg(long, env = "TERP_LCD", default_value = "http://192.168.10.101:1317")]
    lcd: String,
    /// dao-dao-core code checksum hex. Queried from the LCD when omitted.
    #[arg(long)]
    checksum: Option<String>,
    #[arg(long, default_value_t = CODE_CORE)]
    core_code_id: u64,
    #[arg(long, default_value = PARENT_CORE)]
    creator: String,
    #[arg(long)]
    accounts: Option<PathBuf>,
    /// Osmosis polytone note on TERP. Omit until that note exists.
    #[arg(long)]
    polytone_note: Option<String>,
    #[arg(long)]
    osmosis_core: Option<u64>,
    #[arg(long)]
    osmosis_voting_cw4: Option<u64>,
    #[arg(long)]
    osmosis_cw4_group: Option<u64>,
    #[arg(long)]
    osmosis_proposal_single: Option<u64>,
}

fn fetch_checksum(lcd: &str, code_id: u64) -> Result<Vec<u8>> {
    let url = format!(
        "{}/cosmwasm/wasm/v1/code/{code_id}",
        lcd.trim_end_matches('/')
    );
    let out = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "20",
            "-H",
            "User-Agent: terp-steward-daos",
            &url,
        ])
        .output()
        .context("curl code info")?;
    if !out.status.success() {
        bail!(
            "code {code_id} query failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let hash = v["code_info"]["data_hash"]
        .as_str()
        .context("code info has no data_hash")?;
    decode_hex(hash).map_err(|e| anyhow::anyhow!(e))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let accounts_path = args.accounts.clone().unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../steward-accounts.json")
    });
    let accounts = load_steward_accounts(&accounts_path).map_err(|e| anyhow::anyhow!(e))?;
    let checksum = match &args.checksum {
        Some(hex) => decode_hex(hex).map_err(|e| anyhow::anyhow!(e))?,
        None => fetch_checksum(&args.lcd, args.core_code_id)?,
    };
    let remote = match (
        args.osmosis_core,
        args.osmosis_voting_cw4,
        args.osmosis_cw4_group,
        args.osmosis_proposal_single,
    ) {
        (Some(core), Some(voting_cw4), Some(cw4_group), Some(proposal_single)) => {
            Some(RemoteCodeIds {
                core,
                voting_cw4,
                cw4_group,
                proposal_single,
            })
        }
        (None, None, None, None) => None,
        _ => bail!("pass all four --osmosis-* code ids, or none of them"),
    };
    let plan = build_steward_plan(
        &accounts,
        &args.creator,
        &checksum,
        args.polytone_note.as_deref(),
        remote.as_ref(),
    )
    .map_err(|e| anyhow::anyhow!(e))?;

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out/steward");
    std::fs::create_dir_all(&out_dir)?;
    let foundation_path = out_dir.join("01-foundation-stewards-custody.json");
    std::fs::write(
        &foundation_path,
        serde_json::to_string_pretty(&plan.foundation_proposal)?,
    )?;
    let mut files = vec![foundation_path];
    for sub in &plan.subdaos {
        let path = out_dir.join(format!("{}.json", sub.file_stem));
        std::fs::write(&path, serde_json::to_string_pretty(&sub.body)?)?;
        files.push(path);
    }
    let index = serde_json::json!({
        "dry_run": true,
        "broadcast": false,
        "creator": plan.creator,
        "core_code_id": args.core_code_id,
        "checksum": plan.checksum_hex,
        "addresses": plan.addresses,
        "veto_seconds": dao_foundation_suite::ibc_ops::steward::VETO_SECONDS,
        "files": files,
        "note": "SubDAO proposals are submitted by the custody DAO after the foundation proposal executes. The Osmosis instantiate stays unready until its polytone account exists and its member is an osmo address.",
    });
    let index_path = out_dir.join("plan.json");
    std::fs::write(&index_path, serde_json::to_string_pretty(&index)?)?;
    println!("{}", serde_json::to_string_pretty(&index)?);
    Ok(())
}
