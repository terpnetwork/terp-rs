//! Two Foundation DAO proposals:
//!   1. 100,000 TERP to every bonded validator
//!   2. instantiate FIAB-SUBDAO (parent admin, sole member)
//!
//! Default is dry-run. `--broadcast` submits both via `terpd` (no relayer).

use anyhow::{bail, Context, Result};
use clap::Parser;
use cw_orch_ibc::{
    blockpro_delegate_msg, drop_veto_msg, fiab_topup_msg, foundation_proposal_wrapper,
    morocco_fiab_instantiate, NativeDelegationPlan, BLOCKPRO_DELEGATE_DESCRIPTION,
    BLOCKPRO_DELEGATE_TITLE, DELEGATION_DESCRIPTION, DELEGATION_TITLE, DROP_VETO_DESCRIPTION,
    DROP_VETO_TITLE, FIAB_MEMBER, FIAB_TOPUP_DESCRIPTION, FIAB_TOPUP_TITLE, FOUNDATION_DESCRIPTION,
    FOUNDATION_TITLE, PRE_PROPOSE_A, PROPOSAL_A,
};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "foundation_delegate")]
struct Args {
    #[arg(long, env = "TERP_LCD", default_value = "http://192.168.10.101:1317")]
    lcd: String,
    #[arg(long, env = "TERP_RPC", default_value = "http://192.168.10.101:26657")]
    rpc: String,
    #[arg(long, default_value_t = true)]
    dry_run: bool,
    #[arg(long, default_value_t = false)]
    broadcast: bool,
    #[arg(long, env = "CHAIN_ID", default_value = "morocco-1")]
    chain_id: String,
    #[arg(
        long,
        env = "PARENT_CORE",
        default_value = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w"
    )]
    parent_core: String,
    #[arg(long, default_value = "do")]
    from: String,
    #[arg(long, default_value = "test")]
    keyring_backend: String,
    #[arg(long, default_value = "/Users/returniflost/go/bin/terpd")]
    terpd: String,
    #[arg(long)]
    others: Option<String>,
    /// Write the next three payloads (drop veto, BlockPro 100k, FIAB 1e6/1e6).
    #[arg(long, default_value_t = false)]
    next: bool,
    /// FIAB core, once proposal 4 has executed. Needed for the top-up send.
    #[arg(long, env = "FIAB_CORE")]
    fiab_core: Option<String>,
}

fn fetch_bonded(lcd: &str) -> Result<Vec<String>> {
    let url = format!(
        "{}/cosmos/staking/v1beta1/validators?status=BOND_STATUS_BONDED&pagination.limit=200",
        lcd.trim_end_matches('/')
    );
    let out = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "20",
            "-H",
            "User-Agent: terp-foundation-delegate",
            &url,
        ])
        .output()
        .context("curl LCD")?;
    if !out.status.success() {
        bail!("LCD fetch failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    Ok(v["validators"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x["operator_address"].as_str().map(str::to_string))
        .collect())
}

fn submit(args: &Args, label: &str, path: &PathBuf) -> Result<String> {
    let msg = std::fs::read_to_string(path)?;
    let out = Command::new(&args.terpd)
        .args([
            "tx",
            "wasm",
            "execute",
            PRE_PROPOSE_A,
            &msg,
            "--from",
            &args.from,
            "--keyring-backend",
            &args.keyring_backend,
            "--chain-id",
            &args.chain_id,
            "--node",
            &args.rpc,
            "--gas",
            "auto",
            "--gas-adjustment",
            "1.6",
            "--gas-prices",
            "0.05uthiol",
            "--yes",
            "--output",
            "json",
        ])
        .output()
        .with_context(|| format!("terpd {label}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        bail!("{label} failed:\n{stderr}\n{stdout}");
    }
    Ok(stdout)
}

fn write_next(args: &Args, out_dir: &PathBuf) -> Result<()> {
    let veto = foundation_proposal_wrapper(
        DROP_VETO_TITLE,
        DROP_VETO_DESCRIPTION,
        vec![drop_veto_msg()],
    );
    let bp = foundation_proposal_wrapper(
        BLOCKPRO_DELEGATE_TITLE,
        BLOCKPRO_DELEGATE_DESCRIPTION,
        vec![blockpro_delegate_msg()],
    );
    let veto_path = out_dir.join("03-drop-veto.json");
    let bp_path = out_dir.join("04-delegate-blockpro.json");
    std::fs::write(&veto_path, serde_json::to_string_pretty(&veto)?)?;
    std::fs::write(&bp_path, serde_json::to_string_pretty(&bp)?)?;
    eprintln!("wrote {}", veto_path.display());
    eprintln!("wrote {}", bp_path.display());

    let topup = if let Some(addr) = args.fiab_core.as_deref() {
        let body = foundation_proposal_wrapper(
            FIAB_TOPUP_TITLE,
            FIAB_TOPUP_DESCRIPTION,
            vec![fiab_topup_msg(addr)],
        );
        let path = out_dir.join("05-fiab-topup.json");
        std::fs::write(&path, serde_json::to_string_pretty(&body)?)?;
        eprintln!("wrote {} -> {addr}", path.display());
        Some(body)
    } else {
        eprintln!("FIAB_CORE unset — skip 05-fiab-topup.json until proposal 4 executes");
        None
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "note": "3 and 4 keep the 7-day lock; this only changes new proposals",
            "drop_veto": { "title": DROP_VETO_TITLE, "path": veto_path, "propose": veto },
            "blockpro": { "title": BLOCKPRO_DELEGATE_TITLE, "path": bp_path, "propose": bp },
            "fiab_topup": topup,
        }))?
    );
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out");
    std::fs::create_dir_all(&out_dir)?;
    if args.next {
        return write_next(&args, &out_dir);
    }

    let others = if let Some(raw) = args.others.as_ref() {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        match fetch_bonded(&args.lcd) {
            Ok(v) => {
                eprintln!("bonded validators from {}: {}", args.lcd, v.len());
                v
            }
            Err(e) => {
                eprintln!("LCD unavailable ({e}); pass --others valoperA,valoperB");
                Vec::new()
            }
        }
    };

    let plan = NativeDelegationPlan::bonded_set(others);
    eprintln!(
        "delegation: {} validators × {} {}",
        plan.other_validators.len(),
        plan.other_stake,
        plan.stake_denom
    );
    eprintln!("fiab member: {FIAB_MEMBER}");

    let del = foundation_proposal_wrapper(
        DELEGATION_TITLE,
        DELEGATION_DESCRIPTION,
        plan.cosmos_msgs_any(&args.parent_core),
    );
    let fiab = foundation_proposal_wrapper(
        FOUNDATION_TITLE,
        FOUNDATION_DESCRIPTION,
        vec![morocco_fiab_instantiate(&args.parent_core)],
    );

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out");
    std::fs::create_dir_all(&out_dir)?;
    let del_path = out_dir.join("01-delegate-bonded.json");
    let fiab_path = out_dir.join("02-fiab-subdao.json");
    std::fs::write(&del_path, serde_json::to_string_pretty(&del)?)?;
    std::fs::write(&fiab_path, serde_json::to_string_pretty(&fiab)?)?;
    eprintln!("wrote {} and {}", del_path.display(), fiab_path.display());

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "parent_core": args.parent_core,
            "proposal_module": PROPOSAL_A,
            "pre_propose": PRE_PROPOSE_A,
            "dry_run": !args.broadcast,
            "fiab_member": FIAB_MEMBER,
            "delegation": {
                "title": DELEGATION_TITLE,
                "description": DELEGATION_DESCRIPTION,
                "path": del_path,
                "propose": del,
            },
            "fiab": {
                "title": FOUNDATION_TITLE,
                "description": FOUNDATION_DESCRIPTION,
                "path": fiab_path,
                "propose": fiab,
            },
        }))?
    );

    if !args.broadcast {
        return Ok(());
    }

    eprintln!("broadcasting delegation proposal as {}", args.from);
    let r1 = submit(&args, "delegation", &del_path)?;
    println!("{r1}");
    eprintln!("broadcasting FIAB instantiate proposal as {}", args.from);
    let r2 = submit(&args, "fiab", &fiab_path)?;
    println!("{r2}");
    Ok(())
}
