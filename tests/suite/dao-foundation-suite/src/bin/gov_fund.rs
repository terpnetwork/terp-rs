//! Fund cosmos governance proposals from `MAIN_MNEMONIC`, then optionally
//! pass a foundation-DAO proposal with the two member keys.
//!
//! The deposit list is general: repeat `--deposit <id>:<amount><denom>`.
//! With no `--deposit`, the two morocco-1 halves are proposal 60 and
//! proposal 61 at 5,000 TERP each, in one transaction.
//!
//! `--dao-fund` proposes a treasury deposit, votes yes with
//! `DAO_MEMBER1_MNEMONIC` and `DAO_MEMBER2_MNEMONIC`, and executes.
//! Two yes votes from those members pass before the 32-hour period ends.
//! Nothing is broadcast unless `--broadcast` is set. Mnemonics are never printed.

use anyhow::{Context, Result, bail};
use clap::Parser;
use cosmwasm_std::Uint256;
use cw_orch::environment::{ChainKind, NetworkInfoOwned};
use cw_orch::prelude::{
    Addr, ChainInfoOwned, ChainState, Coin, Daemon, DaemonBuilder, QueryHandler, Stargate,
    TxHandler,
};
use dao_foundation_suite::ibc_ops::foundation_proposal_wrapper;
use dao_foundation_suite::ibc_ops::native_delegation::{
    GovDepositSpec, HALF_MIN_DEPOSIT_UTERP, PARENT_CORE, PRE_PROPOSE_A, PROPOSAL_A, STAKE_DENOM,
    execute_proposal_msg, gov_deposit_any_msg, parse_gov_deposit, vote_yes_msg,
};
use prost_types::Any;
use std::path::{Path, PathBuf};
use std::process::Command;

const MEMBERS: &[&str] = &[
    "terp14eygcfdmwpl00nw2amqpujg73nyy59azqu55em",
    "terp1fpdz820lcfze3alx2gukhp7yrnhshe4dv96wgj",
    "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052",
];

#[derive(Parser, Debug)]
#[command(name = "gov_fund")]
struct Args {
    #[arg(long)]
    env_file: Option<PathBuf>,
    /// Sign and broadcast. Omit this to print the signer addresses and the messages.
    #[arg(long, default_value_t = false)]
    broadcast: bool,
    /// `proposal:amountDenom`, for example `60:5000000000uterp`. Repeat for each proposal.
    #[arg(long = "deposit")]
    deposits: Vec<String>,
    /// Skip the MAIN_MNEMONIC deposits.
    #[arg(long, default_value_t = false)]
    no_deposits: bool,
    /// Propose a DAO treasury deposit, vote yes with both members, and execute.
    #[arg(long, default_value_t = false)]
    dao_fund: bool,
    /// Existing DAO proposal to vote on and execute. Skips creating one.
    #[arg(long)]
    dao_proposal: Option<u64>,
    /// Chain proposal the DAO deposits on.
    #[arg(long, default_value_t = 61)]
    dao_gov: u64,
    /// Micro-units the DAO deposits when `--dao-fund` creates the proposal.
    #[arg(long, default_value_t = HALF_MIN_DEPOSIT_UTERP)]
    dao_uterp: u128,
    #[arg(long, env = "CHAIN_ID", default_value = "morocco-1")]
    chain_id: String,
    #[arg(long, env = "GRPC_URL", default_value = "http://192.168.10.101:9090")]
    grpc_url: String,
    #[arg(long, env = "LCD_URL", default_value = "http://192.168.10.101:1317")]
    lcd_url: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let deposits = if args.no_deposits {
        Vec::new()
    } else if args.deposits.is_empty() {
        vec![
            parse_gov_deposit(&format!("60:{HALF_MIN_DEPOSIT_UTERP}{STAKE_DENOM}"))
                .map_err(|err| anyhow::anyhow!(err))?,
            parse_gov_deposit(&format!("61:{HALF_MIN_DEPOSIT_UTERP}{STAKE_DENOM}"))
                .map_err(|err| anyhow::anyhow!(err))?,
        ]
    } else {
        args.deposits
            .iter()
            .map(|spec| parse_gov_deposit(spec).map_err(|err| anyhow::anyhow!(err)))
            .collect::<Result<Vec<_>>>()?
    };
    let dao = args.dao_fund || args.dao_proposal.is_some();
    if deposits.is_empty() && !dao {
        bail!("nothing to do: pass --deposit or --dao-fund");
    }
    let env_file = resolve_env_file(args.env_file.clone());
    load_env(&env_file, dao)?;

    println!("env: {}", env_file.display());
    println!("chain: {} grpc: {}", args.chain_id, args.grpc_url);
    println!("broadcast: {}", args.broadcast);
    for spec in &deposits {
        println!(
            "deposit proposal {} amount {}{}",
            spec.proposal_id, spec.amount, spec.denom
        );
    }
    if dao {
        match args.dao_proposal {
            Some(id) => println!("dao: vote yes and execute proposal {id}"),
            None => println!(
                "dao: propose a treasury deposit of {} {STAKE_DENOM} on gov proposal {}, then both members vote yes and the run executes it",
                args.dao_uterp, args.dao_gov
            ),
        }
    }

    let chain = chain_info(&args);
    let mut secrets = Vec::new();
    let mut shared = None;
    if !deposits.is_empty() {
        let mnemonic = required_phrase("MAIN_MNEMONIC", &mut secrets)?;
        let daemon = connect(&chain, &mnemonic, None).context("MAIN_MNEMONIC daemon")?;
        shared = Some(daemon.state());
        let sender = daemon.sender_addr().to_string();
        let spendable = spendable_uterp(&args.lcd_url, &sender).unwrap_or(0);
        let needed: u128 = deposits
            .iter()
            .filter(|spec| spec.denom == STAKE_DENOM)
            .map(|spec| spec.amount.to_string().parse::<u128>().unwrap_or(u128::MAX))
            .fold(0u128, |acc, n| acc.saturating_add(n));
        println!("MAIN_MNEMONIC address: {sender}");
        println!("MAIN_MNEMONIC spendable uterp: {spendable}");
        println!("deposit total uterp: {needed}");
        if args.broadcast && spendable < needed {
            bail!("MAIN_MNEMONIC spendable uterp {spendable} is below the deposit total {needed}");
        }
        if args.broadcast {
            let msgs = deposits
                .iter()
                .map(|spec| deposit_any(&sender, spec))
                .collect();
            let resp = Stargate::commit_any(&daemon, msgs, None)
                .map_err(|err| anyhow::anyhow!(redact(&err.to_string(), &secrets)))?;
            println!(
                "deposits tx {} code {} height {}",
                resp.txhash, resp.code, resp.height
            );
            if resp.code != 0 {
                bail!("deposit tx failed: {}", redact(&resp.raw_log, &secrets));
            }
        }
    }

    if dao {
        let member1 = required_phrase("DAO_MEMBER1_MNEMONIC", &mut secrets)?;
        let member2 = required_phrase("DAO_MEMBER2_MNEMONIC", &mut secrets)?;
        if member1 == member2 {
            bail!("DAO_MEMBER1_MNEMONIC and DAO_MEMBER2_MNEMONIC are the same phrase");
        }
        if let Ok(main) = std::env::var("MAIN_MNEMONIC") {
            if main.trim() == member1.trim() || main.trim() == member2.trim() {
                bail!("MAIN_MNEMONIC is also a DAO member phrase; keep the depositor separate");
            }
        }
        let voter1 = connect(&chain, &member1, shared).context("DAO_MEMBER1 daemon")?;
        let voter2 =
            connect(&chain, &member2, Some(voter1.state())).context("DAO_MEMBER2 daemon")?;
        let addr1 = voter1.sender_addr().to_string();
        let addr2 = voter2.sender_addr().to_string();
        println!("DAO member 1: {addr1}");
        println!("DAO member 2: {addr2}");
        if !MEMBERS.contains(&addr1.as_str()) || !MEMBERS.contains(&addr2.as_str()) {
            bail!("a DAO mnemonic derived an address outside the foundation cw4 group");
        }
        if !args.broadcast {
            println!("dry run: no vote or execute was sent");
            return Ok(());
        }
        let proposal_id = match args.dao_proposal {
            Some(id) => id,
            None => propose(&args, &voter1, &secrets)?,
        };
        send_wasm(
            &voter1,
            PROPOSAL_A,
            &vote_yes_msg(proposal_id),
            &secrets,
            "member 1 vote",
        )?;
        send_wasm(
            &voter2,
            PROPOSAL_A,
            &vote_yes_msg(proposal_id),
            &secrets,
            "member 2 vote",
        )?;
        let status = proposal_status(&args.lcd_url, proposal_id)?;
        if status != "passed" {
            voter1
                .wait_blocks(1)
                .map_err(|err| anyhow::anyhow!(redact(&err.to_string(), &secrets)))?;
        }
        let status = proposal_status(&args.lcd_url, proposal_id)?;
        if status != "passed" {
            bail!(
                "DAO proposal {proposal_id} status is {status}; execute waits until it is passed"
            );
        }
        send_wasm(
            &voter1,
            PROPOSAL_A,
            &execute_proposal_msg(proposal_id),
            &secrets,
            "execute",
        )?;
        println!("executed DAO proposal {proposal_id}");
    } else if !args.broadcast {
        println!("dry run: no deposit was sent");
    }
    Ok(())
}

fn propose(args: &Args, voter: &Daemon, secrets: &[String]) -> Result<u64> {
    let before = proposal_count(&args.lcd_url)?;
    let msg = gov_deposit_any_msg(
        PARENT_CORE,
        args.dao_gov,
        Uint256::from(args.dao_uterp),
        STAKE_DENOM,
    );
    let title = format!("Deposit treasury TERP on chain proposal {}", args.dao_gov);
    let description = format!(
        "This proposal deposits {} {STAKE_DENOM} from this DAO treasury onto cosmos governance proposal {}. It is a deposit, not a vote. The depositor on the message is this DAO.",
        args.dao_uterp, args.dao_gov
    );
    let body = foundation_proposal_wrapper(&title, &description, vec![msg]);
    send_wasm(voter, PRE_PROPOSE_A, &body, secrets, "propose")?;
    let after = proposal_count(&args.lcd_url)?;
    if after <= before {
        voter
            .wait_blocks(1)
            .map_err(|err| anyhow::anyhow!(redact(&err.to_string(), secrets)))?;
    }
    let after = proposal_count(&args.lcd_url)?;
    if after <= before {
        bail!("proposal count stayed at {before} after the propose transaction");
    }
    println!("opened DAO proposal {after}");
    Ok(after)
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

fn deposit_any(depositor: &str, spec: &GovDepositSpec) -> Any {
    match gov_deposit_any_msg(depositor, spec.proposal_id, spec.amount, &spec.denom) {
        cosmwasm_std::CosmosMsg::Any(any) => Any {
            type_url: any.type_url,
            value: any.value.to_vec(),
        },
        other => panic!("deposit builder returned {other:?}"),
    }
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
        rpc_urls: vec![],
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
        candidates.push(workspace.join(path));
        candidates.push(default_env_file());
    } else {
        candidates.push(default_env_file());
    }
    candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(default_env_file)
}

fn load_env(path: &Path, dao: bool) -> Result<()> {
    if !path.exists() {
        if dao {
            bail!(
                "no env file at {} — fill MAIN_MNEMONIC, DAO_MEMBER1_MNEMONIC, and DAO_MEMBER2_MNEMONIC",
                path.display()
            );
        }
        bail!(
            "no env file at {} — fill MAIN_MNEMONIC. DAO member phrases are not used for a wallet deposit.",
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
    let body = smart(
        lcd,
        PROPOSAL_A,
        &format!(r#"{{"proposal":{{"proposal_id":{id}}}}}"#),
    )?;
    Ok(body
        .pointer("/data/proposal/status")
        .or_else(|| body.pointer("/data/status"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string())
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
            "30",
            "-H",
            "User-Agent: terp-gov-fund",
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
