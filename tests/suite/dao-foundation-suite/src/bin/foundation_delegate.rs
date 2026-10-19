//! Foundation DAO proposals.
//!
//! Default writes the FIAB-SUBDAO instantiate, funded with 1,000,000 TERP and
//! 1,000,000 THIOL. Dry-run does not open the env file. `--broadcast` discovers
//! proposal module A through cw-orch, checks it is the known pre-propose, and
//! submits that JSON with `DAO_MEMBER1_MNEMONIC`.
//!
//! `--bounty` writes the BOUNTY-SUBDAO instantiate instead: single-choice,
//! then the bounties proposal module, funded with 1,000,000 TERP. That path
//! does not broadcast.

use anyhow::{Context, Result, bail};
use clap::Parser;
use cw_orch::environment::{ChainKind, NetworkInfoOwned};
use cw_orch::prelude::{Addr, ChainInfoOwned, Coin, DaemonBuilder, TxHandler};
use dao_foundation_suite::ibc_ops::{
    BLOCKPRO_DELEGATE_DESCRIPTION, BLOCKPRO_DELEGATE_TITLE, BOUNTY_PROPOSAL_TITLE,
    DROP_VETO_DESCRIPTION, DROP_VETO_TITLE, FIAB_MEMBER, FIAB_PROPOSAL_TITLE,
    FIAB_TOPUP_DESCRIPTION, FIAB_TOPUP_TITLE, FOUNDATION_DESCRIPTION, PRE_PROPOSE_A, PROPOSAL_A,
    blockpro_delegate_msg, bounty_proposal_description, drop_veto_msg, fiab_topup_msg,
    foundation_proposal_wrapper, morocco_bounty_instantiate, morocco_fiab_instantiate,
};
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_testing::{ProposalEntry, single_choice_route};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(name = "foundation_delegate")]
struct Args {
    #[arg(long, default_value_t = true)]
    dry_run: bool,
    #[arg(long, default_value_t = false)]
    broadcast: bool,
    #[arg(long, env = "CHAIN_ID", default_value = "morocco-1")]
    chain_id: String,
    #[arg(long, default_value = "http://192.168.10.101:9090")]
    grpc_url: String,
    #[arg(long, default_value = "http://192.168.10.101:1317")]
    lcd_url: String,
    /// Env file with DAO_MEMBER1_MNEMONIC. Read only when broadcasting.
    #[arg(long)]
    env_file: Option<PathBuf>,
    #[arg(
        long,
        env = "PARENT_CORE",
        default_value = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w"
    )]
    parent_core: String,
    /// Write the older follow-ups: drop veto, BlockPro 100k, and a FIAB top-up.
    /// The FIAB proposal already attaches the million of each denom.
    #[arg(long, default_value_t = false)]
    next: bool,
    /// FIAB core, once proposal 4 has executed. Needed for the top-up send.
    #[arg(long, env = "FIAB_CORE")]
    fiab_core: Option<String>,
    /// Write the bounty subDAO proposal instead of FIAB. Dry-run only.
    #[arg(long, default_value_t = false)]
    bounty: bool,
    /// Sole cw4 member of the bounty subDAO.
    #[arg(long)]
    member: Option<String>,
    /// On-chain code id of the bounties proposal module.
    #[arg(long)]
    bounty_code_id: Option<u64>,
    /// JSON instantiate message for the bounties proposal module.
    #[arg(long)]
    bounty_init: Option<PathBuf>,
}

fn write_bounty(args: &Args, out_dir: &PathBuf) -> Result<()> {
    let member = args
        .member
        .as_deref()
        .map(str::trim)
        .filter(|member| member.starts_with("terp1") && !member.contains(char::is_whitespace))
        .context("--member is the bounty subDAO's terp1 account")?;
    let code_id = args
        .bounty_code_id
        .filter(|id| *id > 0)
        .context("--bounty-code-id is the uploaded bounties proposal module")?;
    let init_path = args
        .bounty_init
        .as_ref()
        .context("--bounty-init is the bounties instantiate JSON")?;
    let init_bytes =
        std::fs::read(init_path).with_context(|| format!("read {}", init_path.display()))?;
    let init_json: serde_json::Value = serde_json::from_slice(&init_bytes)
        .context("--bounty-init must be the bounties instantiate JSON")?;
    if !init_json.is_object() {
        bail!("--bounty-init must be a JSON object");
    }
    let bounty_module = ModuleInstantiateInfo {
        code_id,
        msg: cosmwasm_std::to_json_binary(&init_json)?,
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "bounty-bounties".into(),
        salt: None,
    };
    let description = bounty_proposal_description(code_id);
    let body = foundation_proposal_wrapper(
        BOUNTY_PROPOSAL_TITLE,
        &description,
        vec![morocco_bounty_instantiate(
            &args.parent_core,
            member,
            bounty_module,
        )],
    );
    let path = out_dir.join("06-bounty-subdao.json");
    std::fs::write(&path, serde_json::to_string_pretty(&body)?)?;
    eprintln!("wrote {}", path.display());
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "parent_core": args.parent_core,
            "dry_run": true,
            "member": member,
            "bounty_code_id": code_id,
            "bounty": {
                "title": BOUNTY_PROPOSAL_TITLE,
                "description": description,
                "path": path,
                "propose": body,
            },
        }))?
    );
    Ok(())
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
    if args.bounty && args.next {
        bail!("pass either --bounty or --next");
    }
    if args.bounty {
        if args.broadcast {
            bail!("the bounty proposal is written only; it is not broadcast");
        }
        return write_bounty(&args, &out_dir);
    }
    if args.next {
        return write_next(&args, &out_dir);
    }

    eprintln!("fiab member: {FIAB_MEMBER}");
    eprintln!("this command writes only the FIAB proposal");

    let fiab = foundation_proposal_wrapper(
        FIAB_PROPOSAL_TITLE,
        FOUNDATION_DESCRIPTION,
        vec![morocco_fiab_instantiate(&args.parent_core)],
    );

    let fiab_path = out_dir.join("02-fiab-subdao.json");
    std::fs::write(&fiab_path, serde_json::to_string_pretty(&fiab)?)?;
    eprintln!("wrote {}", fiab_path.display());

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "parent_core": args.parent_core,
            "proposal_module": PROPOSAL_A,
            "pre_propose": PRE_PROPOSE_A,
            "dry_run": !args.broadcast,
            "fiab_member": FIAB_MEMBER,
            "fiab": {
                "title": FIAB_PROPOSAL_TITLE,
                "description": FOUNDATION_DESCRIPTION,
                "path": fiab_path,
                "propose": fiab,
            },
        }))?
    );

    if !args.broadcast {
        return Ok(());
    }

    let env_path = args.env_file.clone().unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ibc-ops/cw-orch-ibc/.env")
    });
    load_env(&env_path)?;
    let mut secrets = Vec::new();
    let phrase = required_phrase("DAO_MEMBER1_MNEMONIC", &mut secrets)?;
    let chain = chain_info(&args);
    let mut builder = DaemonBuilder::new(chain);
    builder.mnemonic(&phrase).load_network(false).is_test(true);
    let daemon = builder
        .build()
        .map_err(|err| anyhow::anyhow!(redact(&err.to_string(), &secrets)))
        .context("build cw-orch daemon")?;
    let sender = daemon.sender_addr().to_string();
    println!("proposer {sender}");
    let route = single_choice_route(&daemon, &args.parent_core, Some("A"))
        .map_err(|err| anyhow::anyhow!(redact(&err.to_string(), &secrets)))?;
    let ProposalEntry::PreProposeSingle { pre_propose } = &route.entry else {
        bail!(
            "proposal module {} is not dao-pre-propose-single",
            route.proposal
        );
    };
    if pre_propose != PRE_PROPOSE_A {
        bail!("discovered pre-propose {pre_propose} is not {PRE_PROPOSE_A}");
    }
    if route.proposal != PROPOSAL_A {
        bail!(
            "discovered proposal module {} is not {PROPOSAL_A}",
            route.proposal
        );
    }
    let body: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&fiab_path)?)?;
    eprintln!("broadcasting FIAB instantiate proposal from {sender}");
    let resp = daemon
        .execute(&body, &[] as &[Coin], &Addr::unchecked(pre_propose))
        .map_err(|err| anyhow::anyhow!("fiab: {}", redact(&err.to_string(), &secrets)))?;
    println!(
        "fiab tx {} code {} height {}",
        resp.txhash, resp.code, resp.height
    );
    if resp.code != 0 {
        bail!("fiab failed: {}", redact(&resp.raw_log, &secrets));
    }
    Ok(())
}

fn chain_info(args: &Args) -> ChainInfoOwned {
    ChainInfoOwned {
        chain_id: args.chain_id.clone(),
        gas_denom: "uthiol".into(),
        gas_price: 0.05,
        grpc_urls: vec![args.grpc_url.clone()],
        lcd_url: Some(args.lcd_url.clone()),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: "terp".into(),
            coin_type: 118,
        },
        kind: ChainKind::Mainnet,
    }
}

fn load_env(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!(
            "no env file at {} — fill DAO_MEMBER1_MNEMONIC",
            path.display()
        );
    }
    dotenvy::from_path(path).with_context(|| format!("load {}", path.display()))?;
    Ok(())
}

fn required_phrase(key: &str, secrets: &mut Vec<String>) -> Result<String> {
    let phrase = std::env::var(key)
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    if phrase.is_empty() {
        bail!("{key} is empty. Fill it in the env file; the value is not printed.");
    }
    secrets.push(phrase.clone());
    Ok(phrase)
}

fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            out = out.replace(secret, "[redacted]");
        }
    }
    if out.contains("Bad Phrase") {
        return "DAO_MEMBER1_MNEMONIC is not a valid English mnemonic. The phrase was not printed and nothing was signed.".into();
    }
    out
}
