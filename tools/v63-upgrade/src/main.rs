//! Submit the morocco-1 expedited plan `v6.3` from a hot wallet.
//!
//! Dry-run unless the env file sets `UPGRADE_BROADCAST=1`. Never reads
//! `crates/terp-rs/.env`. Never submits plan `v6.4` and never calls the DAO.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use cosmrs::proto::cosmos::base::v1beta1::Coin;
use cosmrs::proto::cosmos::gov::v1::MsgSubmitProposal;
use cosmrs::proto::cosmos::upgrade::v1beta1::{MsgSoftwareUpgrade, Plan};
use cw_orch_daemon::{DaemonBuilder, TxSender};
use cw_orch_networks::networks::TERP_MAINNET;
use prost::Message;
use serde_json::Value;

const GOV_MODULE: &str = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq";
const EXPEDITED_DEPOSIT: &str = "100000000000uterp";
const PLAN_NAME: &str = "v6.3";

fn main() -> Result<()> {
    let env_path = env_path()?;
    if env_path.file_name().and_then(|s| s.to_str()) == Some(".env") {
        bail!("refusing {env_path:?}; use .env.v63-upgrade");
    }
    let env = load_env(&env_path)?;
    let draft = read_draft()?;
    let height: i64 = draft.height.parse().context("draft height")?;
    let env_height: i64 = env
        .get("HEIGHT")
        .context("HEIGHT missing from the env file")?
        .parse()
        .context("HEIGHT")?;
    if env_height != height {
        bail!("env HEIGHT {env_height} != draft {height}");
    }
    if draft.deposit != EXPEDITED_DEPOSIT {
        bail!("draft deposit {} is not the expedited minimum", draft.deposit);
    }
    if env.get("DEPOSIT").map(String::as_str) != Some(EXPEDITED_DEPOSIT) {
        bail!("env DEPOSIT must be {EXPEDITED_DEPOSIT}");
    }
    if !draft.expedited || draft.name != PLAN_NAME {
        bail!("draft must be expedited plan {PLAN_NAME}");
    }
    if draft.authority != GOV_MODULE {
        bail!("draft authority is not the gov module");
    }
    if draft.info.contains("file://") || !draft.info.contains("4161cf4531f357ce") {
        bail!("plan.info is not the published v6.3 checksum JSON");
    }

    let lcd = env
        .get("LCD")
        .map(String::as_str)
        .unwrap_or("http://192.168.10.101:1317");
    let chain_id = env.get("CHAIN_ID").map(String::as_str).unwrap_or("morocco-1");
    if chain_id != "morocco-1" {
        bail!("CHAIN_ID must be morocco-1");
    }
    preflight(lcd, height)?;

    let broadcast = env.get("UPGRADE_BROADCAST").map(String::as_str) == Some("1");
    println!("plan={PLAN_NAME} expedited=true height={height} deposit={EXPEDITED_DEPOSIT}");
    println!("authority={GOV_MODULE}");
    println!("title={}", draft.title);
    println!("summary={}", draft.summary);
    println!("info={}", draft.info);
    if !broadcast {
        println!("dry-run: UPGRADE_BROADCAST is not 1. nothing was signed or broadcast.");
        return Ok(());
    }

    let mnemonic = env
        .get("MAIN_MNEMONIC")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .context("MAIN_MNEMONIC is empty")?;
    let grpc = env
        .get("GRPC")
        .map(String::as_str)
        .unwrap_or("http://192.168.10.101:9090");
    // Re-check the queue immediately before signing.
    preflight(lcd, height)?;

    let rt = tokio::runtime::Runtime::new().context("tokio")?;
    let mut daemon = DaemonBuilder::new(TERP_MAINNET);
    let daemon = daemon
        .handle(rt.handle())
        .grpc_url(grpc)
        .gas(Some("uterp"), Some(0.05))
        .mnemonic(mnemonic)
        .load_network(false)
        .is_test(true)
        .build()
        .context("cw-orch daemon (grpc and mnemonic)")?;
    let signer = daemon.sender().account_id();
    println!("signer={signer}");

    #[allow(deprecated)]
    let upgrade = MsgSoftwareUpgrade {
        authority: GOV_MODULE.to_string(),
        plan: Some(Plan {
            name: PLAN_NAME.to_string(),
            time: None,
            height,
            info: draft.info.clone(),
            upgraded_client_state: None,
        }),
    };
    let upgrade_any = cosmrs::Any {
        type_url: "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade".into(),
        value: upgrade.encode_to_vec(),
    };
    let (amount, denom) = split_coin(EXPEDITED_DEPOSIT)?;
    let submit = MsgSubmitProposal {
        messages: vec![upgrade_any],
        initial_deposit: vec![Coin { denom, amount }],
        proposer: signer.to_string(),
        metadata: String::new(),
        title: draft.title,
        summary: draft.summary,
        expedited: true,
    };
    let any = cosmrs::Any {
        type_url: "/cosmos.gov.v1.MsgSubmitProposal".into(),
        value: submit.encode_to_vec(),
    };
    let resp = rt
        .block_on(daemon.sender().commit_tx_any(vec![any], Some("v6.3 expedited")))
        .context("broadcast")?;
    println!("broadcast txhash={}", resp.txhash);
    Ok(())
}

struct Draft {
    height: String,
    info: String,
    deposit: String,
    title: String,
    summary: String,
    expedited: bool,
    name: String,
    authority: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn read_draft() -> Result<Draft> {
    let path = repo_root().join("networks/upgrades/v6.3/draft_proposal.json");
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let v: Value = serde_json::from_str(&raw)?;
    let msg = &v["messages"][0];
    Ok(Draft {
        height: json_str(&msg["plan"]["height"])?,
        info: json_str(&msg["plan"]["info"])?,
        deposit: json_str(&v["deposit"])?,
        title: json_str(&v["title"])?,
        summary: json_str(&v["summary"])?,
        expedited: v["expedited"].as_bool().unwrap_or(false),
        name: json_str(&msg["plan"]["name"])?,
        authority: json_str(&msg["authority"])?,
    })
}

fn json_str(v: &Value) -> Result<String> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("expected string in draft, got {v}"))
}

fn env_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("V63_UPGRADE_ENV") {
        return Ok(PathBuf::from(p));
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.env.v63-upgrade"))
}

fn load_env(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read {} (create it and set MAIN_MNEMONIC)", path.display()))?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"');
        out.insert(k.trim().to_string(), v.to_string());
    }
    Ok(out)
}

fn preflight(lcd: &str, height: i64) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    for status in [
        "PROPOSAL_STATUS_VOTING_PERIOD",
        "PROPOSAL_STATUS_DEPOSIT_PERIOD",
    ] {
        let url = format!("{lcd}/cosmos/gov/v1/proposals?proposal_status={status}");
        let body: Value = client.get(&url).send()?.error_for_status()?.json()?;
        let n = body["proposals"].as_array().map(|a| a.len()).unwrap_or(0);
        if n != 0 {
            bail!("{n} proposal(s) in {status}; refusing to submit another");
        }
    }
    let plan: Value = client
        .get(format!("{lcd}/cosmos/upgrade/v1beta1/current_plan"))
        .send()?
        .error_for_status()?
        .json()?;
    if !plan["plan"].is_null() {
        bail!("a software upgrade is already planned: {plan}");
    }
    let status: Value = client
        .get("https://rpc.terp.network/status")
        .send()?
        .error_for_status()?
        .json()?;
    let current: i64 = status["result"]["sync_info"]["latest_block_height"]
        .as_str()
        .context("height")?
        .parse()?;
    // Expedited voting is 86400s. At 3s/block that is 28800 blocks.
    if height - current < 28_800 {
        bail!("height {height} is fewer than 28800 blocks ahead of {current}; the 24h vote would pass the halt");
    }
    let params: Value = client
        .get(format!("{lcd}/cosmos/gov/v1/params/deposit"))
        .send()?
        .error_for_status()?
        .json()?;
    let expedited = params["params"]["expedited_min_deposit"][0]["amount"]
        .as_str()
        .unwrap_or("");
    if expedited != "100000000000" {
        bail!("chain expedited min deposit changed: {expedited}");
    }
    Ok(())
}

fn split_coin(s: &str) -> Result<(String, String)> {
    let i = s
        .find(|c: char| !c.is_ascii_digit())
        .context("deposit amount")?;
    Ok((s[..i].to_string(), s[i..].to_string()))
}
