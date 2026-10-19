//! Resume or create the foundation DAO actions for the v6.3 expedited upgrade,
//! then vote yes on the open chain proposals.
//!
//! The upgrade body is `networks/upgrades/v6.3/draft_proposal.json`. Governance
//! submits only plan v6.3. The v6.3 binary arms v6.4. If a DAO proposal with
//! the upgrade title already exists, this run votes and executes that proposal
//! instead of opening another one. The second DAO proposal votes yes on chain
//! proposals 60, 61, and 62. 62 is the id the executed submit creates. Both
//! member keys vote, then this process executes the submit before the chain
//! votes. Nothing is broadcast unless `--broadcast` is set. Mnemonics are
//! never printed.

use anyhow::{Context, Result, bail};
use clap::Parser;
use cw_orch::environment::{ChainKind, NetworkInfoOwned};
use cw_orch::prelude::{
    Addr, ChainInfoOwned, ChainState, Coin, Daemon, DaemonBuilder, QueryHandler, TxHandler,
};
use dao_foundation_suite::ibc_ops::foundation_proposal_wrapper;
use dao_foundation_suite::ibc_ops::native_delegation::{
    PARENT_CORE, PRE_PROPOSE_A, PROPOSAL_A, STAKE_DENOM, UpgradeDraft, execute_proposal_msg,
    gov_submit_upgrade_any_msg, gov_vote_yes_any_msg, load_upgrade_cut, v63_upgrade_draft_path,
    vote_yes_msg,
};
use std::path::{Path, PathBuf};
use std::process::Command;

const MEMBERS: &[&str] = &[
    "terp14eygcfdmwpl00nw2amqpujg73nyy59azqu55em",
    "terp1fpdz820lcfze3alx2gukhp7yrnhshe4dv96wgj",
    "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052",
];

const GOV_MODULE: &str = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq";

#[derive(Parser, Debug)]
#[command(name = "dao_upgrade")]
struct Args {
    /// Env file with DAO_MEMBER1_MNEMONIC and DAO_MEMBER2_MNEMONIC.
    #[arg(long)]
    env_file: Option<PathBuf>,
    /// Sign and broadcast. The default prints the plan and sends nothing.
    #[arg(long)]
    broadcast: bool,
    /// v6.3 expedited draft. Defaults to the workspace file.
    #[arg(long)]
    draft: Option<PathBuf>,
    /// Chain proposal id the submit creates.
    #[arg(long, default_value_t = 62)]
    upgrade_id: u64,
    /// Chain proposals the DAO votes yes on, after the upgrade exists.
    #[arg(long = "vote", default_values_t = [60u64, 61, 62])]
    votes: Vec<u64>,
    #[arg(long, default_value = "morocco-1")]
    chain_id: String,
    #[arg(long, default_value = "http://192.168.10.101:9090")]
    grpc_url: String,
    #[arg(long, default_value = "http://192.168.10.101:1317")]
    lcd_url: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let draft_path = args.draft.clone().unwrap_or_else(v63_upgrade_draft_path);
    let cut = load_upgrade_cut(&draft_path).map_err(|err| anyhow::anyhow!(err))?;
    let draft = &cut.draft;
    if draft.authority != GOV_MODULE {
        bail!(
            "draft authority is {}, expected the gov module {GOV_MODULE}",
            draft.authority
        );
    }
    if draft.plan_name != "v6.3" {
        bail!("draft plan name is {}, expected v6.3", draft.plan_name);
    }
    if !args.votes.contains(&args.upgrade_id) {
        bail!(
            "vote list {:?} does not include upgrade id {}",
            args.votes,
            args.upgrade_id
        );
    }

    println!("draft: {}", draft_path.display());
    println!(
        "upgrade: plan {} height {} halt {} submit by {} deposit {}{}",
        draft.plan_name,
        draft.height,
        cut.halt_time_utc,
        cut.submit_by_utc,
        draft.deposit_amount,
        draft.deposit_denom
    );
    println!("v6.3.0 {}", cut.v63_commit);
    println!(
        "v6.4.0 {} armed at height {}",
        cut.v64_commit, cut.v64_height
    );
    println!("v6.4 is armed by the v6.3 binary and is not a second governance message");
    println!(
        "dao 1: {} submits the upgrade from {PARENT_CORE}; both members vote yes; the run executes it",
        draft.title
    );
    match find_proposal(&args.lcd_url, &draft.title) {
        Ok(Some((id, status))) => println!(
            "dao 1 reuses proposal {id} ({status}) and does not open another with that title"
        ),
        Ok(None) => println!("dao 1 has no existing proposal and would open one"),
        Err(err) => println!("dao 1 lookup failed: {err}"),
    }
    let vote_title = format!("Vote yes on chain proposals {:?}", args.votes);
    match find_proposal(&args.lcd_url, &vote_title) {
        Ok(Some((id, status))) => println!("dao 2 reuses proposal {id} ({status})"),
        Ok(None) => println!("dao 2 has no existing proposal and would open one"),
        Err(err) => println!("dao 2 lookup failed: {err}"),
    }
    println!(
        "dao 2: {} votes yes on chain proposals {:?}",
        PARENT_CORE, args.votes
    );
    println!("broadcast: {}", args.broadcast);
    if !args.broadcast {
        println!("dry run: no proposal, vote, or execute was sent");
        return Ok(());
    }

    let env_file = resolve_env_file(args.env_file.clone());
    load_env(&env_file)?;
    println!("env: {}", env_file.display());
    ensure_gov_ready(&args, draft)?;
    let upgrade_done = matches!(
        find_proposal(&args.lcd_url, &draft.title)?,
        Some((_, ref status)) if status == "executed"
    );
    if !upgrade_done {
        let dao_balance = spendable_uterp(&args.lcd_url, PARENT_CORE).unwrap_or(0);
        let needed = draft
            .deposit_amount
            .to_string()
            .parse::<u128>()
            .unwrap_or(u128::MAX);
        println!("DAO spendable uterp: {dao_balance}");
        println!("upgrade deposit uterp: {needed}");
        if draft.deposit_denom == STAKE_DENOM && dao_balance < needed {
            bail!("DAO spendable uterp {dao_balance} is below the expedited deposit {needed}");
        }
    }

    let chain = chain_info(&args);
    let mut secrets = Vec::new();
    let member1 = required_phrase("DAO_MEMBER1_MNEMONIC", &mut secrets)?;
    let member2 = required_phrase("DAO_MEMBER2_MNEMONIC", &mut secrets)?;
    if member1 == member2 {
        bail!("DAO_MEMBER1_MNEMONIC and DAO_MEMBER2_MNEMONIC are the same phrase");
    }
    let voter1 = connect(&chain, &member1, None)?;
    let voter2 = connect(&chain, &member2, Some(voter1.state()))?;
    let addr1 = voter1.sender_addr().to_string();
    let addr2 = voter2.sender_addr().to_string();
    println!("DAO member 1: {addr1}");
    println!("DAO member 2: {addr2}");
    if !MEMBERS.contains(&addr1.as_str()) || !MEMBERS.contains(&addr2.as_str()) {
        bail!("a DAO mnemonic derived an address outside the foundation cw4 group");
    }

    let submit_msgs = vec![gov_submit_upgrade_any_msg(PARENT_CORE, &draft)];
    let vote_msgs = args
        .votes
        .iter()
        .copied()
        .map(|id| gov_vote_yes_any_msg(PARENT_CORE, id))
        .collect();
    let submit_id = resume_or_open(
        &args,
        &voter1,
        &voter2,
        &secrets,
        &draft.title,
        &submit_description(&draft),
        submit_msgs,
    )?;
    ensure_upgrade_submitted(&args, &draft)?;
    let vote_id = resume_or_open(
        &args,
        &voter1,
        &voter2,
        &secrets,
        &vote_title,
        &format!(
            "This proposal votes yes, from this DAO, on cosmos governance proposals {}. Proposal {} is the expedited v6.3 upgrade submitted by the proposal executed immediately before this one. v6.4 is not voted here; the v6.3 binary arms it.",
            args.votes
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            args.upgrade_id
        ),
        vote_msgs,
    )?;
    println!("DAO proposals {submit_id} then {vote_id} are executed");
    Ok(())
}

fn resume_or_open(
    args: &Args,
    voter1: &Daemon,
    voter2: &Daemon,
    secrets: &[String],
    title: &str,
    description: &str,
    msgs: Vec<cosmwasm_std::CosmosMsg>,
) -> Result<u64> {
    let existing = find_proposal(&args.lcd_url, title)?;
    let (id, status) = match existing {
        Some(found) => found,
        None => {
            let id = propose(args, voter1, secrets, title, description, msgs)?;
            (id, "open".to_string())
        }
    };
    match status.as_str() {
        "executed" => {
            println!("DAO proposal {id} is already executed");
            return Ok(id);
        }
        "open" => {
            pass(args, voter1, voter2, secrets, id)?;
            execute(args, voter1, secrets, id)?;
        }
        "passed" => execute(args, voter1, secrets, id)?,
        other => bail!("DAO proposal {id} ({title}) is {other}; not resuming it"),
    }
    Ok(id)
}

fn submit_description(
    draft: &dao_foundation_suite::ibc_ops::native_delegation::UpgradeDraft,
) -> String {
    format!(
        "Submit the expedited governance proposal \"{}\" from this DAO. The plan is {} at height {}. The deposit is {} {}. v6.4 is armed by that binary two blocks later and is not a second message. {}",
        draft.title,
        draft.plan_name,
        draft.height,
        draft.deposit_amount,
        draft.deposit_denom,
        draft.summary
    )
}

fn propose(
    args: &Args,
    voter: &Daemon,
    secrets: &[String],
    title: &str,
    description: &str,
    msgs: Vec<cosmwasm_std::CosmosMsg>,
) -> Result<u64> {
    let before = proposal_count(&args.lcd_url)?;
    let body = foundation_proposal_wrapper(title, description, msgs);
    send_wasm(voter, PRE_PROPOSE_A, &body, secrets, "propose")?;
    let mut after = proposal_count(&args.lcd_url)?;
    if after <= before {
        voter
            .wait_blocks(1)
            .map_err(|err| anyhow::anyhow!(redact(&err.to_string(), secrets)))?;
        after = proposal_count(&args.lcd_url)?;
    }
    if after <= before {
        bail!("proposal count stayed at {before} after {title}");
    }
    println!("opened DAO proposal {after}: {title}");
    Ok(after)
}

fn pass(args: &Args, voter1: &Daemon, voter2: &Daemon, secrets: &[String], id: u64) -> Result<()> {
    for (label, voter) in [("member 1", voter1), ("member 2", voter2)] {
        let addr = voter.sender_addr().to_string();
        if already_voted(&args.lcd_url, id, &addr)? {
            println!("{label} already voted yes on DAO proposal {id}");
            continue;
        }
        send_wasm(
            voter,
            PROPOSAL_A,
            &vote_yes_msg(id),
            secrets,
            &format!("{label} vote on {id}"),
        )?;
    }
    let status = proposal_status(&args.lcd_url, id)?;
    if status != "passed" {
        voter1
            .wait_blocks(1)
            .map_err(|err| anyhow::anyhow!(redact(&err.to_string(), secrets)))?;
    }
    let status = proposal_status(&args.lcd_url, id)?;
    if status != "passed" {
        bail!("DAO proposal {id} status is {status}");
    }
    println!("DAO proposal {id} passed");
    Ok(())
}

fn execute(_args: &Args, voter: &Daemon, secrets: &[String], id: u64) -> Result<()> {
    send_wasm(
        voter,
        PROPOSAL_A,
        &execute_proposal_msg(id),
        secrets,
        &format!("execute {id}"),
    )
}

fn ensure_gov_ready(args: &Args, draft: &UpgradeDraft) -> Result<()> {
    for id in &args.votes {
        if *id == args.upgrade_id {
            match gov_proposal(&args.lcd_url, *id) {
                Ok(proposal) => {
                    let status = proposal
                        .pointer("/proposal/status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let title = proposal
                        .pointer("/proposal/title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let expedited = proposal
                        .pointer("/proposal/expedited")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let dao_executed = matches!(
                        find_proposal(&args.lcd_url, &draft.title)?,
                        Some((_, ref status)) if status == "executed"
                    );
                    if dao_executed
                        && title == draft.title
                        && expedited
                        && status == "PROPOSAL_STATUS_VOTING_PERIOD"
                    {
                        println!("chain proposal {id} is already the expedited upgrade in voting");
                        continue;
                    }
                    bail!(
                        "chain proposal {id} already exists; refusing to vote it as the new upgrade"
                    );
                }
                Err(_) => println!("chain proposal {id} is not on chain yet"),
            }
            continue;
        }
        let proposal =
            gov_proposal(&args.lcd_url, *id).with_context(|| format!("chain proposal {id}"))?;
        let status = proposal
            .pointer("/proposal/status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        println!("chain proposal {id} status {status}");
        if status != "PROPOSAL_STATUS_VOTING_PERIOD" {
            bail!("chain proposal {id} is {status}, not in the voting period");
        }
    }
    Ok(())
}

fn ensure_upgrade_submitted(args: &Args, draft: &UpgradeDraft) -> Result<()> {
    let proposal = gov_proposal(&args.lcd_url, args.upgrade_id)
        .with_context(|| format!("chain proposal {} after submit", args.upgrade_id))?;
    let status = proposal
        .pointer("/proposal/status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let title = proposal
        .pointer("/proposal/title")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expedited = proposal
        .pointer("/proposal/expedited")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    println!(
        "chain proposal {} status {status} expedited {expedited} title {title}",
        args.upgrade_id
    );
    if title != draft.title || !expedited || status != "PROPOSAL_STATUS_VOTING_PERIOD" {
        bail!(
            "chain proposal {} is not the expedited upgrade in voting; the DAO vote was not executed",
            args.upgrade_id
        );
    }
    Ok(())
}

fn gov_proposal(lcd: &str, id: u64) -> Result<serde_json::Value> {
    lcd_get(&format!(
        "{}/cosmos/gov/v1/proposals/{id}",
        lcd.trim_end_matches('/')
    ))
}

fn send_wasm<E: serde::Serialize + std::fmt::Debug>(
    daemon: &Daemon,
    contract: &str,
    msg: &E,
    secrets: &[String],
    label: &str,
) -> Result<()> {
    let resp = daemon
        .execute(msg, &[] as &[Coin], &Addr::unchecked(contract))
        .map_err(|err| anyhow::anyhow!("{label}: {}", redact(&err.to_string(), secrets)))?;
    println!(
        "{label} tx {} code {} height {}",
        resp.txhash, resp.code, resp.height
    );
    if resp.code != 0 {
        bail!("{label} failed: {}", redact(&resp.raw_log, secrets));
    }
    Ok(())
}

fn connect(
    chain: &ChainInfoOwned,
    mnemonic: &str,
    shared: Option<cw_orch::daemon::DaemonState>,
) -> Result<Daemon> {
    let mut builder = DaemonBuilder::new(chain.clone());
    builder.mnemonic(mnemonic).load_network(false);
    if let Some(state) = shared {
        builder.state(state);
    } else {
        // A temp state file stays off the shared ~/.cw-orch/state.json lock.
        builder.is_test(true);
    }
    builder.build().context("build cw-orch daemon")
}

fn chain_info(args: &Args) -> ChainInfoOwned {
    let gas_price = std::env::var("GAS_PRICE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.05);
    ChainInfoOwned {
        chain_id: args.chain_id.clone(),
        gas_denom: std::env::var("GAS_DENOM").unwrap_or_else(|_| "uthiol".into()),
        gas_price,
        grpc_urls: vec![args.grpc_url.clone()],
        lcd_url: Some(args.lcd_url.clone()),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: std::env::var("ADDRESS_PREFIX").unwrap_or_else(|_| "terp".into()),
            coin_type: 118,
        },
        kind: ChainKind::Mainnet,
    }
}

fn default_env_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ibc-ops/cw-orch-ibc/.env")
}

fn resolve_env_file(requested: Option<PathBuf>) -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(path) = requested {
        let terp_rs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../..");
        candidates.push(path.clone());
        candidates.push(terp_rs.join(&path));
        candidates.push(workspace.join(&path));
        candidates.push(default_env_file());
    } else {
        candidates.push(default_env_file());
    }
    candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(default_env_file)
}

fn load_env(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!(
            "no env file at {} — fill DAO_MEMBER1_MNEMONIC and DAO_MEMBER2_MNEMONIC",
            path.display()
        );
    }
    dotenvy::from_path(path).with_context(|| format!("load {}", path.display()))?;
    Ok(())
}

fn required_phrase(key: &str, secrets: &mut Vec<String>) -> Result<String> {
    let raw = std::env::var(key).unwrap_or_default();
    let phrase = raw.trim().trim_matches('"').trim_matches('\'').to_string();
    if phrase.is_empty() || phrase.contains("word1 word2") {
        bail!("{key} is empty. Fill it in the env file; the value is not printed.");
    }
    let words = phrase.split_whitespace().count();
    if ![12, 15, 18, 21, 24].contains(&words) {
        bail!("{key} has {words} words; expected 12, 15, 18, 21, or 24");
    }
    secrets.push(phrase.clone());
    Ok(phrase)
}

fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() > 8 {
            out = out.replace(secret, "[redacted]");
        }
    }
    out
}

fn proposal_count(lcd: &str) -> Result<u64> {
    let body = smart(lcd, PROPOSAL_A, r#"{"proposal_count":{}}"#)?;
    body.pointer("/data")
        .and_then(|v| v.as_u64())
        .context("proposal count")
}

fn proposal_status(lcd: &str, id: u64) -> Result<String> {
    Ok(proposal_record(lcd, id)?.0)
}

fn proposal_record(lcd: &str, id: u64) -> Result<(String, String)> {
    let body = smart(
        lcd,
        PROPOSAL_A,
        &format!(r#"{{"proposal":{{"proposal_id":{id}}}}}"#),
    )?;
    let status = body
        .pointer("/data/proposal/status")
        .or_else(|| body.pointer("/data/status"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let title = body
        .pointer("/data/proposal/title")
        .or_else(|| body.pointer("/data/title"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((status, title))
}

/// Highest id with this exact title. An older closed proposal does not hide a later open one.
fn find_proposal(lcd: &str, title: &str) -> Result<Option<(u64, String)>> {
    let count = proposal_count(lcd)?;
    let mut found = None;
    for id in 1..=count {
        let (status, got) = proposal_record(lcd, id)?;
        if got == title {
            found = Some((id, status));
        }
    }
    Ok(found)
}

fn already_voted(lcd: &str, id: u64, voter: &str) -> Result<bool> {
    let query = format!(r#"{{"get_vote":{{"proposal_id":{id},"voter":"{voter}"}}}}"#);
    match smart(lcd, PROPOSAL_A, &query) {
        Ok(body) => Ok(body
            .pointer("/data/vote")
            .map(|vote| !vote.is_null())
            .unwrap_or(false)),
        Err(err) => {
            let text = err.to_string().to_ascii_lowercase();
            if text.contains("not found")
                || text.contains("no vote")
                || text.contains("unknown vote")
            {
                Ok(false)
            } else {
                Err(err)
            }
        }
    }
}

fn spendable_uterp(lcd: &str, address: &str) -> Result<u128> {
    let body = lcd_get(&format!(
        "{}/cosmos/bank/v1beta1/spendable_balances/{address}",
        lcd.trim_end_matches('/')
    ))?;
    Ok(body
        .pointer("/balances")
        .and_then(|v| v.as_array())
        .and_then(|rows| {
            rows.iter().find_map(|row| {
                if row.get("denom").and_then(|v| v.as_str()) != Some(STAKE_DENOM) {
                    return None;
                }
                row.get("amount").and_then(|v| v.as_str())
            })
        })
        .unwrap_or("0")
        .parse()?)
}

fn smart(lcd: &str, contract: &str, query: &str) -> Result<serde_json::Value> {
    let encoded = base64_std(query.as_bytes());
    lcd_get(&format!(
        "{}/cosmwasm/wasm/v1/contract/{contract}/smart/{encoded}",
        lcd.trim_end_matches('/')
    ))
}

fn lcd_get(url: &str) -> Result<serde_json::Value> {
    let output = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "20",
            "-H",
            "User-Agent: terp-dao-upgrade",
            url,
        ])
        .output()
        .context("curl LCD")?;
    if !output.status.success() {
        bail!(
            "LCD {url} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).with_context(|| format!("LCD JSON from {url}"))
}

fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n =
            (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8) | u32::from(bytes[i + 2]);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rest = bytes.len() - i;
    if rest == 1 {
        let n = u32::from(bytes[i]) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}
