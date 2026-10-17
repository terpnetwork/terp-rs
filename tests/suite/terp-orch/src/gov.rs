//! Draft and dry-run `cosmos.gov.v1.MsgSubmitProposal` for morocco-1 IBC clients.
//!
//! Submit-time checks (message type, gov module signer, deposit math) run inside
//! a node simulation. `MsgRecoverClient` itself is not executed until a proposal
//! passes, and it needs an Active substitute client that this tool does not create.
//! ibc-go v11 has no message that deletes a light client. Halted-chain drafts are
//! a public "do not recover" notice, not a close.

use anyhow::{bail, Context, Result};
use clap::Parser;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
use cosmos_sdk_proto::cosmos::gov::v1::MsgSubmitProposal;
use cosmos_sdk_proto::prost::Message;
use cosmos_sdk_proto::traits::Name;
use cw_orch::daemon::{CosmosOptions, DaemonBuilder, TxSender};
use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned, TxHandler};
use ibc_proto::ibc::core::client::v1::MsgRecoverClient;
use serde::Deserialize;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Used only inside a simulation when the draft has not been given a real substitute.
/// Never written back to the draft, and `--broadcast` refuses it.
pub const SUBSTITUTE_STAND_IN: &str = "07-tendermint-9000";

/// Canonical text ratified by the baseline proposal. Not copied into chain state.
pub const BASELINE_URL: &str =
    "https://s3.terp.network/static/gov/baseline-valuation/baseline.html";

const SHUTDOWN_CHAINS: &[&str] = &["stargaze-1", "evmos_9001-2", "omniflixhub-1"];
const OMIT_RECOVER: &[&str] = &["atomone-1"];

#[derive(Parser, Debug)]
#[command(
    name = "terp-gov-proposal",
    about = "Dry-run morocco-1 governance proposals via cw-orch. Does not broadcast unless asked."
)]
struct Args {
    /// Directory of draft JSON files. Default: this crate's light-client drafts.
    #[arg(long)]
    proposals_dir: Option<PathBuf>,

    /// Simulate one draft instead of every JSON file in the directory.
    #[arg(long)]
    proposal: Option<PathBuf>,

    /// dotenv file with MAIN_MNEMONIC. Default: scripts/release/.env
    #[arg(long, env = "UPGRADE_ENV_FILE")]
    env_file: Option<PathBuf>,

    /// Sign and broadcast. Refuses while a substitute client id is blank.
    /// Also requires TERP_GOV_BROADCAST=1 so a dry-run cannot post a deposit.
    #[arg(long)]
    broadcast: bool,

    #[arg(long, env = "CHAIN_ID")]
    chain_id: Option<String>,

    #[arg(long, env = "GRPC_URL")]
    grpc_url: Option<String>,

    #[arg(long, env = "LCD_URL")]
    lcd_url: Option<String>,

    /// Test-keyring account used when MAIN_MNEMONIC is empty.
    /// Broadcast still requires --broadcast and TERP_GOV_BROADCAST=1.
    #[arg(long, default_value = "do")]
    from: String,

    /// Keyring home for `--from`. Default: $HOME/.terpd
    #[arg(long, env = "TERP_HOME")]
    key_home: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DraftKind {
    Recover,
    /// Metadata-only notice. Does not change client state.
    RetireNotice,
    /// Metadata-only ratification. No messages, so it cannot mint, burn, or change parameters.
    Text,
    /// Recorded so it is not picked up by a recover draft. Not submitted.
    Omit,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Draft {
    pub kind: DraftKind,
    pub counterparty_chain_id: String,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub metadata: String,
    #[serde(default)]
    pub deposit: String,
    #[serde(default)]
    pub expedited: bool,
    /// Filled in after a new Active client exists. Blank is a dry-run stand-in only.
    #[serde(default)]
    pub substitute_client_id: String,
    #[serde(default)]
    pub subjects: Vec<Subject>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Subject {
    pub client_id: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub connection_state: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    /// Set when one proposal recovers several chains. Checked against the live client.
    #[serde(default)]
    pub chain_id: Option<String>,
    /// Per-subject substitute. Falls back to the draft substitute when empty.
    #[serde(default)]
    pub substitute_client_id: Option<String>,
    /// Governance closure mark. Not a chain state. ibc-go connection ends have no CLOSED value.
    #[serde(default)]
    pub closure: Option<String>,
}

#[derive(Debug)]
pub struct BuiltProposal {
    pub title: String,
    pub kind: DraftKind,
    pub message_count: usize,
    pub used_substitute_stand_in: bool,
    pub submit: MsgSubmitProposal,
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_proposals_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proposals")
}

pub fn load_draft(path: &Path) -> Result<Draft> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

pub fn parse_coin(s: &str) -> Result<ProtoCoin> {
    let i = s
        .find(|c: char| c.is_ascii_alphabetic())
        .context("deposit must look like 5000000000uterp")?;
    if i == 0 || !s[..i].chars().all(|c| c.is_ascii_digit()) {
        bail!("deposit amount must be an integer before the denom, got {s}");
    }
    Ok(ProtoCoin {
        denom: s[i..].to_string(),
        amount: s[..i].to_string(),
    })
}

fn client_id_well_formed(id: &str) -> bool {
    let Some((prefix, n)) = id.rsplit_once('-') else {
        return false;
    };
    prefix == "07-tendermint" && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
}

/// Build the submit message. `allow_stand_in` is true only for simulation.
pub fn build_submit(
    draft: &Draft,
    proposer: &str,
    gov_authority: &str,
    allow_stand_in: bool,
) -> Result<BuiltProposal> {
    if draft.title.is_empty() || draft.title.len() > 255 {
        bail!("title must be 1..=255 bytes");
    }
    if draft.metadata.len() > 255 {
        bail!(
            "metadata is {} bytes; the chain rejects metadata longer than 255",
            draft.metadata.len()
        );
    }
    if draft.summary.is_empty() || draft.summary.len() > 40 * 255 {
        bail!("summary must be 1..=10200 bytes");
    }
    if draft.expedited {
        bail!("expedited proposals are out of scope for these client drafts");
    }

    match draft.kind {
        DraftKind::Omit => bail!("omit drafts are not submitted"),
        DraftKind::RetireNotice => {
            if !SHUTDOWN_CHAINS.contains(&draft.counterparty_chain_id.as_str())
                && draft.counterparty_chain_id != "halted"
            {
                bail!(
                    "retire-notice is only for halted chains, got {}",
                    draft.counterparty_chain_id
                );
            }
            if draft.metadata.is_empty() {
                bail!("retire-notice needs metadata so the proposal is not an empty message");
            }
            if !draft.subjects.is_empty() && draft.subjects.iter().any(|s| s.client_id.is_empty()) {
                bail!("retire subject is missing a client id");
            }
            for subject in &draft.subjects {
                if !client_id_well_formed(&subject.client_id) {
                    bail!("bad client id {}", subject.client_id);
                }
                if subject.connection_id.is_some() && subject.closure.as_deref() != Some("STATE_CLOSED") {
                    bail!(
                        "connection {} must be recorded STATE_CLOSED in the retire proposal",
                        subject.connection_id.as_deref().unwrap_or("")
                    );
                }
            }
            let submit = MsgSubmitProposal {
                messages: vec![],
                initial_deposit: vec![parse_coin(&draft.deposit)?],
                proposer: proposer.to_string(),
                metadata: draft.metadata.clone(),
                title: draft.title.clone(),
                summary: draft.summary.clone(),
                expedited: false,
            };
            return Ok(BuiltProposal {
                title: draft.title.clone(),
                kind: draft.kind.clone(),
                message_count: 0,
                used_substitute_stand_in: false,
                submit,
            });
        }
        DraftKind::Text => {
            if !draft.subjects.is_empty() {
                bail!("a text proposal has no client subjects");
            }
            if draft.metadata.is_empty() {
                bail!("a text proposal needs metadata so the chain accepts an empty message list");
            }
            if draft.metadata.trim_start().starts_with('{') {
                bail!(
                    "metadata must be plain text. JSON metadata is checked against the title and summary, and the summary does not fit in 255 bytes"
                );
            }
            let submit = MsgSubmitProposal {
                messages: vec![],
                initial_deposit: vec![parse_coin(&draft.deposit)?],
                proposer: proposer.to_string(),
                metadata: draft.metadata.clone(),
                title: draft.title.clone(),
                summary: draft.summary.clone(),
                expedited: false,
            };
            return Ok(BuiltProposal {
                title: draft.title.clone(),
                kind: DraftKind::Text,
                message_count: 0,
                used_substitute_stand_in: false,
                submit,
            });
        }
        DraftKind::Recover => {}
    }

    if SHUTDOWN_CHAINS.contains(&draft.counterparty_chain_id.as_str()) {
        bail!(
            "{} is a halted chain; refusing MsgRecoverClient",
            draft.counterparty_chain_id
        );
    }
    if OMIT_RECOVER.contains(&draft.counterparty_chain_id.as_str()) {
        bail!(
            "{} is omitted from recover (a new client will be created instead)",
            draft.counterparty_chain_id
        );
    }
    if draft.subjects.is_empty() {
        bail!("recover draft has no subjects");
    }

    let mut any_stand_in = false;
    let mut messages = Vec::with_capacity(draft.subjects.len());
    for subject in &draft.subjects {
        if !client_id_well_formed(&subject.client_id) {
            bail!("bad client id {}", subject.client_id);
        }
        let (substitute, stand_in) = substitute_for(draft, subject, allow_stand_in)?;
        any_stand_in |= stand_in;
        if subject.client_id == substitute {
            bail!(
                "subject {} and substitute {substitute} must differ",
                subject.client_id
            );
        }
        let msg = MsgRecoverClient {
            subject_client_id: subject.client_id.clone(),
            substitute_client_id: substitute,
            signer: gov_authority.to_string(),
        };
        messages.push(cosmos_sdk_proto::Any {
            type_url: MsgRecoverClient::type_url(),
            value: msg.encode_to_vec(),
        });
    }

    let submit = MsgSubmitProposal {
        messages,
        initial_deposit: vec![parse_coin(&draft.deposit)?],
        proposer: proposer.to_string(),
        metadata: draft.metadata.clone(),
        title: draft.title.clone(),
        summary: draft.summary.clone(),
        expedited: false,
    };
    Ok(BuiltProposal {
        title: draft.title.clone(),
        kind: DraftKind::Recover,
        message_count: draft.subjects.len(),
        used_substitute_stand_in: any_stand_in,
        submit,
    })
}

fn substitute_for(draft: &Draft, subject: &Subject, allow_stand_in: bool) -> Result<(String, bool)> {
    let chosen = subject
        .substitute_client_id
        .clone()
        .filter(|id| !id.is_empty())
        .or_else(|| {
            if draft.substitute_client_id.is_empty() {
                None
            } else {
                Some(draft.substitute_client_id.clone())
            }
        });
    match chosen {
        Some(id) if id == SUBSTITUTE_STAND_IN => {
            bail!("refusing the simulation stand-in {SUBSTITUTE_STAND_IN}")
        }
        Some(id) => {
            if !client_id_well_formed(&id) {
                bail!("bad substitute client id {id}");
            }
            Ok((id, false))
        }
        None if allow_stand_in => Ok((SUBSTITUTE_STAND_IN.to_string(), true)),
        None => bail!(
            "substitute_client_id is empty for {}. Create an Active client first, write its id into the draft, then broadcast.",
            subject.client_id
        ),
    }
}

pub fn submit_any(submit: &MsgSubmitProposal) -> cosmos_sdk_proto::Any {
    cosmos_sdk_proto::Any {
        type_url: MsgSubmitProposal::type_url(),
        value: submit.encode_to_vec(),
    }
}

fn chain_kind(s: &str) -> ChainKind {
    match s.to_ascii_lowercase().as_str() {
        "testnet" => ChainKind::Testnet,
        "local" => ChainKind::Local,
        _ => ChainKind::Mainnet,
    }
}

fn chain_info(args: &Args) -> Result<ChainInfoOwned> {
    let chain_id = args
        .chain_id
        .clone()
        .or_else(|| std::env::var("CHAIN_ID").ok())
        .unwrap_or_else(|| "morocco-1".into());
    let grpc = args
        .grpc_url
        .clone()
        .or_else(|| std::env::var("GRPC_URL").ok())
        .unwrap_or_else(|| "http://192.168.10.101:9090".into());
    let gas_denom = std::env::var("GAS_DENOM").unwrap_or_else(|_| "uthiol".into());
    let gas_price: f64 = std::env::var("GAS_PRICE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.05);
    let prefix = std::env::var("ADDRESS_PREFIX").unwrap_or_else(|_| "terp".into());
    let kind = chain_kind(&std::env::var("CHAIN_KIND").unwrap_or_else(|_| "mainnet".into()));
    Ok(ChainInfoOwned {
        chain_id,
        gas_denom,
        gas_price,
        grpc_urls: vec![grpc],
        lcd_url: args
            .lcd_url
            .clone()
            .or_else(|| std::env::var("LCD_URL").ok()),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: prefix,
            coin_type: 118,
        },
        kind,
    })
}

fn http_get(url: &str) -> Result<String> {
    let rest = url
        .strip_prefix("http://")
        .context("LCD URL must be http:// (node is on the LAN, not TLS)")?;
    let (hostport, path) = rest.split_once('/').context("LCD URL needs a path")?;
    let (host, port) = match hostport.split_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().context("LCD port")?),
        None => (hostport, 80),
    };
    let mut stream = TcpStream::connect((host, port)).with_context(|| format!("connect {url}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    stream.set_write_timeout(Some(Duration::from_secs(20)))?;
    let req = format!(
        "GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    );
    stream.write_all(req.as_bytes())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    let raw = String::from_utf8_lossy(&buf);
    let (head, body) = raw.split_once("\r\n\r\n").context("HTTP response headers")?;
    let status = head.lines().next().unwrap_or("");
    if !status.contains(" 200 ") {
        bail!("{status} for {url}");
    }
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        return decode_chunks(body);
    }
    Ok(body.trim_end_matches('\0').to_string())
}

fn decode_chunks(mut rest: &str) -> Result<String> {
    let mut out = String::new();
    loop {
        let (size_line, after) = rest.split_once("\r\n").context("chunk size")?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).context("chunk length")?;
        if size == 0 {
            break;
        }
        if after.len() < size + 2 {
            bail!("short HTTP chunk");
        }
        out.push_str(&after[..size]);
        rest = &after[size + 2..];
    }
    Ok(out)
}

fn lcd_json(base: &str, path: &str) -> Result<serde_json::Value> {
    let url = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let body = http_get(&url)?;
    serde_json::from_str(&body).with_context(|| format!("json from {url}: {}", &body[..body.len().min(180)]))
}

fn gov_authority(lcd: &str) -> Result<String> {
    let v = lcd_json(lcd, "/cosmos/auth/v1beta1/module_accounts")?;
    let accounts = v
        .get("accounts")
        .and_then(|a| a.as_array())
        .context("module accounts")?;
    for account in accounts {
        if account.get("name").and_then(|n| n.as_str()) == Some("gov") {
            let addr = account
                .pointer("/base_account/address")
                .and_then(|a| a.as_str())
                .context("gov module address")?;
            return Ok(addr.to_string());
        }
    }
    bail!("gov module account not in LCD response")
}

fn client_height(lcd: &str, client_id: &str) -> Result<u128> {
    let state = lcd_json(lcd, &format!("/ibc/core/client/v1/client_states/{client_id}"))?;
    let rev = state
        .pointer("/client_state/latest_height/revision_number")
        .and_then(|v| v.as_str())
        .context("revision_number")?
        .parse::<u128>()?;
    let height = state
        .pointer("/client_state/latest_height/revision_height")
        .and_then(|v| v.as_str())
        .context("revision_height")?
        .parse::<u128>()?;
    Ok((rev << 64) | height)
}

fn live_client(lcd: &str, client_id: &str) -> Result<(String, String)> {
    let state = lcd_json(lcd, &format!("/ibc/core/client/v1/client_states/{client_id}"))?;
    let chain_id = state
        .pointer("/client_state/chain_id")
        .and_then(|v| v.as_str())
        .context("client state chain_id")?
        .to_string();
    let status = lcd_json(lcd, &format!("/ibc/core/client/v1/client_status/{client_id}"))?;
    let status = status
        .get("status")
        .and_then(|v| v.as_str())
        .context("client status")?
        .to_string();
    Ok((chain_id, status))
}

fn preflight(lcd: &str, draft: &Draft) -> Result<()> {
    if draft.subjects.is_empty() {
        bail!("draft has no subjects to check");
    }
    for subject in &draft.subjects {
        let (chain_id, status) = live_client(lcd, &subject.client_id)?;
        let expected = if draft.counterparty_chain_id == "live" {
            subject
                .chain_id
                .as_deref()
                .context("a live-batch subject needs chain_id")?
        } else {
            draft.counterparty_chain_id.as_str()
        };
        if draft.counterparty_chain_id != "halted" && chain_id != expected {
            bail!("{} is {chain_id}, draft says {expected}", subject.client_id);
        }
        if status != "Expired" && status != "Frozen" {
            bail!(
                "{} status is {status}; these drafts are only for expired or frozen clients",
                subject.client_id
            );
        }
        match draft.kind {
            DraftKind::Recover if SHUTDOWN_CHAINS.contains(&chain_id.as_str()) => {
                bail!("{} is on halted chain {chain_id}", subject.client_id);
            }
            DraftKind::RetireNotice if !SHUTDOWN_CHAINS.contains(&chain_id.as_str()) => {
                bail!("{} is {chain_id}, not a halted chain", subject.client_id);
            }
            DraftKind::Omit if OMIT_RECOVER.contains(&chain_id.as_str()) => {}
            DraftKind::Omit => bail!("{chain_id} is not in the omit list"),
            _ => {}
        }
        if let Some(sub_id) = subject
            .substitute_client_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .or(if draft.substitute_client_id.is_empty() {
                None
            } else {
                Some(draft.substitute_client_id.as_str())
            })
        {
            if sub_id != SUBSTITUTE_STAND_IN {
                let (sub_chain, sub_status) = live_client(lcd, sub_id)?;
                if sub_status != "Active" {
                    bail!("substitute {sub_id} for {} is {sub_status}, not Active", subject.client_id);
                }
                if sub_chain != chain_id {
                    bail!("substitute {sub_id} is {sub_chain}, subject {} is {chain_id}", subject.client_id);
                }
                let subject_height = client_height(lcd, &subject.client_id)?;
                let sub_height = client_height(lcd, sub_id)?;
                if subject_height >= sub_height {
                    bail!(
                        "substitute {sub_id} height {sub_height} is not above {} height {subject_height}",
                        subject.client_id
                    );
                }
            }
        }
        println!(
            "  {} {} {}{}{}{}",
            subject.client_id,
            chain_id,
            status,
            subject
                .connection_id
                .as_ref()
                .map(|c| format!(
                    " {c} chain-state {}",
                    subject.connection_state.as_deref().unwrap_or("unknown")
                ))
                .unwrap_or_default(),
            subject
                .channel_id
                .as_ref()
                .map(|c| format!(" channel {c}"))
                .unwrap_or_default(),
            subject
                .closure
                .as_ref()
                .map(|c| format!(" recorded {c}"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

fn uterp_balance(lcd: &str, address: &str) -> Result<String> {
    let v = lcd_json(lcd, &format!("/cosmos/bank/v1beta1/balances/{address}"))?;
    let coins = v.get("balances").and_then(|b| b.as_array());
    let amount = coins
        .and_then(|coins| {
            coins.iter().find_map(|c| {
                (c.get("denom").and_then(|d| d.as_str()) == Some("uterp"))
                    .then(|| c.get("amount").and_then(|a| a.as_str()).unwrap_or("0"))
            })
        })
        .unwrap_or("0");
    Ok(amount.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimClass {
    /// Node simulation accepted the submit, including the deposit debit.
    SubmitValid { gas: u64 },
    /// Inner messages and the gov signer were accepted. The account cannot pay the deposit.
    SubmitValidUnfunded,
    Unsound,
    Unclassified,
}

pub fn classify_sim_error(err: &str) -> SimClass {
    let l = err.to_ascii_lowercase();
    let unsound = [
        "invalid signer",
        "unroutable",
        "unable to resolve",
        "no handler exists",
        "invalid proposal",
        "invalid address",
        "error unmarshalling",
        "unknown message",
        "does not exist",
        "not found",
    ];
    // "account ... not found" is the proposer, not the inner message.
    let proposer_missing = l.contains("account") && l.contains("not found");
    if !proposer_missing && unsound.iter().any(|s| l.contains(s)) {
        return SimClass::Unsound;
    }
    if l.contains("insufficient funds")
        || l.contains("spendable balance")
        || l.contains("min deposit")
        || l.contains("smaller than")
    {
        return SimClass::SubmitValidUnfunded;
    }
    SimClass::Unclassified
}

fn redact_secrets(text: &str) -> String {
    let mut out = String::new();
    let mut hex_run = String::new();
    let flush = |hex_run: &mut String, out: &mut String| {
        if hex_run.len() >= 64 {
            out.push_str("[redacted]");
        } else {
            out.push_str(hex_run);
        }
        hex_run.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_hexdigit() {
            hex_run.push(ch);
        } else {
            flush(&mut hex_run, &mut out);
            out.push(ch);
        }
    }
    flush(&mut hex_run, &mut out);
    out
}

/// One 64-char hex line from `keys export --unarmored-hex`. Anything else is ignored
/// so a warning line cannot be mistaken for the key, and the key is never returned as text.
pub fn parse_exported_secret(stdout: &str) -> Result<[u8; 32]> {
    let mut found = None;
    for line in stdout.lines() {
        let line = line.trim();
        if line.len() != 64 || !line.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        if found.is_some() {
            bail!("key export returned more than one secret line");
        }
        let mut buf = [0u8; 32];
        for i in 0..32 {
            buf[i] = u8::from_str_radix(&line[i * 2..i * 2 + 2], 16).context("hex byte")?;
        }
        found = Some(buf);
    }
    found.context("key export did not return a 32-byte hex secret")
}

fn terpd_bin() -> String {
    if let Ok(bin) = std::env::var("TERPD") {
        if !bin.is_empty() {
            return bin;
        }
    }
    let known = "/Users/returniflost/go/bin/terpd";
    if Path::new(known).is_file() {
        known.to_string()
    } else {
        "terpd".to_string()
    }
}

fn key_home(args: &Args) -> Result<PathBuf> {
    if let Some(home) = &args.key_home {
        return Ok(home.clone());
    }
    let home = std::env::var("HOME").context("HOME is unset; pass --key-home")?;
    Ok(PathBuf::from(home).join(".terpd"))
}

fn keyring_address(bin: &str, name: &str, home: &Path) -> Result<String> {
    let out = Command::new(bin)
        .args([
            "keys",
            "show",
            name,
            "-a",
            "--keyring-backend",
            "test",
            "--home",
        ])
        .arg(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("run {bin} keys show {name}"))?;
    if !out.status.success() {
        bail!(
            "terpd keys show {name} failed: {}",
            redact_secrets(&String::from_utf8_lossy(&out.stderr))
        );
    }
    let addr = String::from_utf8(out.stdout)
        .context("keys show address")?
        .trim()
        .to_string();
    if !addr.starts_with("terp1") {
        bail!("keys show did not return a terp address");
    }
    Ok(addr)
}

fn keyring_secret(bin: &str, name: &str, home: &Path) -> Result<[u8; 32]> {
    let out = Command::new(bin)
        .args([
            "keys",
            "export",
            name,
            "--unarmored-hex",
            "--unsafe",
            "-y",
            "--keyring-backend",
            "test",
            "--home",
        ])
        .arg(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("run {bin} keys export {name}"))?;
    if !out.status.success() {
        bail!(
            "terpd keys export {name} failed: {}",
            redact_secrets(&String::from_utf8_lossy(&out.stderr))
        );
    }
    let stdout = String::from_utf8(out.stdout).context("key export encoding")?;
    parse_exported_secret(&stdout)
}

enum SignerSource {
    Mnemonic(String),
    Keyring { name: String, secret: [u8; 32] },
}

fn signer_source(args: &Args) -> Result<SignerSource> {
    let raw = std::env::var("MAIN_MNEMONIC")
        .or_else(|_| std::env::var("TEST_MNEMONIC"))
        .unwrap_or_default();
    let words = raw.split_whitespace().count();
    if words > 0 {
        if ![12, 15, 18, 21, 24].contains(&words) {
            bail!("MAIN_MNEMONIC has {words} words; expected 12, 15, 18, 21, or 24");
        }
        return Ok(SignerSource::Mnemonic(raw));
    }
    let bin = terpd_bin();
    let home = key_home(args)?;
    let secret = keyring_secret(&bin, &args.from, &home)?;
    Ok(SignerSource::Keyring {
        name: args.from.clone(),
        secret,
    })
}

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_json(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
    Ok(())
}

fn load_env(path: &Path) -> Result<()> {
    if path.exists() {
        dotenvy::from_path(path).with_context(|| format!("loading {}", path.display()))?;
    } else {
        bail!(
            "no env file at {} (set MAIN_MNEMONIC or pass --env-file)",
            path.display()
        );
    }
    Ok(())
}

fn draft_paths(args: &Args) -> Result<Vec<PathBuf>> {
    if let Some(one) = &args.proposal {
        return Ok(vec![one.clone()]);
    }
    let dir = args
        .proposals_dir
        .clone()
        .unwrap_or_else(default_proposals_dir);
    let mut paths = Vec::new();
    collect_json(&dir, &mut paths)?;
    paths.sort();
    if paths.is_empty() {
        bail!("no json drafts in {}", dir.display());
    }
    Ok(paths)
}

pub fn run() -> Result<()> {
    let args = Args::parse();
    let root = repo_root();
    let env_path = args.env_file.clone().unwrap_or_else(|| {
        std::env::var("UPGRADE_ENV_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| root.join("scripts/release/.env"))
    });
    load_env(&env_path)?;
    // The node's simulation is the deposit check. cw-orch's extra balance
    // assertion only looks at the gas fee and can fail a simulation the node accepted.
    // SAFETY: this process only sets the flag before cw-orch reads it. Nothing else
    // in this binary races on the environment.
    unsafe {
        std::env::set_var("CW_ORCH_WALLET_BALANCE_ASSERTION", "false");
    }

    let chain = chain_info(&args)?;
    let lcd = chain
        .lcd_url
        .clone()
        .unwrap_or_else(|| "http://192.168.10.101:1317".into());
    let gov = gov_authority(&lcd)?;
    println!("chain_id: {}", chain.chain_id);
    println!("grpc: {}", chain.grpc_urls.first().unwrap_or(&String::new()));
    println!("lcd: {lcd}");
    println!("gov module: {gov}");

    let signer = signer_source(&args)?;
    let mut builder = DaemonBuilder::new(chain);
    builder.load_network(false).is_test(true);
    let daemon = match &signer {
        SignerSource::Mnemonic(mnemonic) => {
            let words = mnemonic.split_whitespace().count();
            println!("signer: {words}-word mnemonic (not printed)");
            builder.mnemonic(mnemonic);
            builder
                .build()
                .context("build cw-orch daemon (check GRPC_URL / mnemonic)")?
        }
        SignerSource::Keyring { name, secret } => {
            println!("signer: test keyring `{name}` (secret not printed; simulation only)");
            let expected = keyring_address(&terpd_bin(), name, &key_home(&args)?)?;
            let daemon = builder
                .build_sender(CosmosOptions::default().secp256k1_secret(secret.to_vec()))
                .context("build cw-orch daemon from the test keyring")?;
            let got = daemon.sender_addr().to_string();
            if got != expected {
                bail!("keyring address {expected} did not match the cw-orch signer {got}");
            }
            daemon
        }
    };
    let proposer = daemon.sender_addr().to_string();
    let balance = uterp_balance(&lcd, &proposer).unwrap_or_else(|e| format!("unknown ({e})"));
    println!("proposer: {proposer}");
    println!("proposer uterp: {balance}");
    if args.broadcast {
        if std::env::var("TERP_GOV_BROADCAST").ok().as_deref() != Some("1") {
            bail!("refusing broadcast. Set TERP_GOV_BROADCAST=1 only when a deposit should be spent.");
        }
    } else {
        println!("mode: simulate only (no deposit is spent)");
    }

    let mut failures = 0u32;
    for path in draft_paths(&args)? {
        println!("\n== {}", path.display());
        let draft = load_draft(&path)?;
        println!(
            "{} {} | {}",
            match draft.kind {
                DraftKind::Recover => "recover",
                DraftKind::RetireNotice => "retire-notice",
                DraftKind::Text => "text",
                DraftKind::Omit => "omit",
            },
            draft.counterparty_chain_id,
            draft.title
        );
        if !draft.note.is_empty() {
            println!("note: {}", draft.note);
        }
        if draft.kind == DraftKind::Text {
            println!("no client check. This draft ratifies a document and does not read IBC state.");
        } else {
            preflight(&lcd, &draft)?;
        }
        if draft.kind == DraftKind::Omit {
            println!("not submitted. {}", draft.summary);
            continue;
        }
        let built = build_submit(&draft, &proposer, &gov, !args.broadcast)?;
        if built.kind == DraftKind::Recover {
            if built.used_substitute_stand_in {
                println!(
                    "substitute: SIMULATION STAND-IN {SUBSTITUTE_STAND_IN} (not a real client; execution would fail until an Active substitute exists)"
                );
            } else {
                println!("substitute: {}", draft.substitute_client_id);
            }
        } else if built.kind == DraftKind::RetireNotice {
            println!("inner messages: none (ibc-go has no client-delete message)");
        } else if built.kind == DraftKind::Text {
            println!(
                "inner messages: none (text ratification of {BASELINE_URL}; no mint, burn, transfer, or parameter change)"
            );
        }
        println!(
            "messages: {} deposit: {}",
            built.message_count, draft.deposit
        );
        let any = submit_any(&built.submit);
        if args.broadcast {
            if built.used_substitute_stand_in || built.kind != DraftKind::Recover && built.message_count != 0 {
                bail!("broadcast refused for {}", path.display());
            }
            let resp = daemon.rt_handle.block_on(
                daemon
                    .sender()
                    .commit_tx_any(vec![any], Some("terp gov proposal")),
            )?;
            println!("BROADCAST txhash: {}", resp.txhash);
            continue;
        }
        let sim = daemon.rt_handle.block_on(
            daemon
                .sender()
                .simulate(vec![any], Some("terp gov proposal dry-run")),
        );
        match sim {
            Ok((gas, fee)) => {
                println!("simulate: SUBMIT_VALID gas={gas} fee={fee}");
                if built.kind == DraftKind::Recover {
                    println!(
                        "submit-time checks passed, including the deposit debit, inside a discarded simulation. MsgRecoverClient was not executed."
                    );
                } else if built.kind == DraftKind::Text {
                    println!(
                        "submit-time checks passed, including the deposit debit, inside a discarded simulation. No tokens move and no parameters change."
                    );
                } else {
                    println!(
                        "submit-time checks passed, including the deposit debit, inside a discarded simulation. No client, connection, or channel state would change."
                    );
                }
            }
            Err(err) => {
                let text = err.to_string();
                match classify_sim_error(&text) {
                    SimClass::SubmitValidUnfunded => {
                        println!("simulate: SUBMIT_VALID_UNFUNDED");
                        println!(
                            "the node accepted the proposal bytes far enough to reject the deposit. No tokens moved. Inner MsgRecoverClient was not executed."
                        );
                        println!("node: {text}");
                    }
                    SimClass::Unsound => {
                        failures += 1;
                        println!("simulate: UNSOUND");
                        println!("node: {text}");
                    }
                    SimClass::SubmitValid { .. } | SimClass::Unclassified => {
                        failures += 1;
                        println!("simulate: UNCLASSIFIED (not treating this as a sound proposal)");
                        println!("node: {text}");
                    }
                }
            }
        }
    }
    if failures > 0 {
        bail!("{failures} draft(s) failed the dry-run");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(id: &str) -> Subject {
        Subject {
            client_id: id.into(),
            connection_id: None,
            connection_state: None,
            channel_id: None,
            chain_id: None,
            substitute_client_id: None,
            closure: None,
        }
    }

    fn recover() -> Draft {
        Draft {
            kind: DraftKind::Recover,
            counterparty_chain_id: "akashnet-2".into(),
            title: "Recover expired akashnet-2 light clients".into(),
            summary: "Dry-run recover.".into(),
            metadata: String::new(),
            deposit: "5000000000uterp".into(),
            expedited: false,
            substitute_client_id: String::new(),
            subjects: vec![subject("07-tendermint-32"), subject("07-tendermint-25")],
            note: String::new(),
        }
    }

    #[test]
    fn stand_in_is_only_for_simulation() {
        let draft = recover();
        let built = build_submit(&draft, "terp1proposer", "terp1gov", true).unwrap();
        assert!(built.used_substitute_stand_in);
        assert_eq!(built.message_count, 2);
        assert_eq!(built.submit.messages[0].type_url, "/ibc.core.client.v1.MsgRecoverClient");
        let msg = MsgRecoverClient::decode(built.submit.messages[0].value.as_slice()).unwrap();
        assert_eq!(msg.signer, "terp1gov");
        assert_eq!(msg.substitute_client_id, SUBSTITUTE_STAND_IN);
        assert_eq!(msg.subject_client_id, "07-tendermint-32");
        assert!(build_submit(&draft, "terp1proposer", "terp1gov", false).is_err());
    }

    #[test]
    fn halted_chain_cannot_be_recovered() {
        let mut draft = recover();
        draft.counterparty_chain_id = "evmos_9001-2".into();
        assert!(build_submit(&draft, "terp1proposer", "terp1gov", true).is_err());
    }

    #[test]
    fn atomone_is_not_a_recover_draft() {
        let mut draft = recover();
        draft.counterparty_chain_id = "atomone-1".into();
        assert!(build_submit(&draft, "terp1proposer", "terp1gov", true).is_err());
    }

    #[test]
    fn baseline_ratification_is_a_text_proposal() {
        let path = default_proposals_dir().join("ratify-baseline-valuation.json");
        let draft = load_draft(&path).unwrap();
        assert_eq!(draft.kind, DraftKind::Text);
        assert!(draft.subjects.is_empty());
        assert!(draft.summary.contains(BASELINE_URL));
        assert!(draft.metadata.contains(BASELINE_URL));
        assert!(draft.summary.contains("$21,046,446.44"));
        assert!(draft.summary.contains("$0.05012"));
        assert!(draft.summary.contains("$0.02360"));
        assert!(draft.metadata.len() <= 255);
        assert!(draft.summary.len() <= 40 * 255);
        let built = build_submit(&draft, "terp1proposer", "terp1gov", true).unwrap();
        assert!(built.submit.messages.is_empty());
        assert_eq!(built.submit.initial_deposit[0].amount, "5000000000");
        assert!(!built.submit.expedited);
    }

    #[test]
    fn retire_notice_has_no_inner_messages() {
        let draft = Draft {
            kind: DraftKind::RetireNotice,
            counterparty_chain_id: "halted".into(),
            title: "Do not recover halted clients".into(),
            summary: "No client is deleted.".into(),
            metadata: "record only; ibc-go has no MsgDeleteClient".into(),
            deposit: "5000000000uterp".into(),
            expedited: false,
            substitute_client_id: String::new(),
            subjects: vec![subject("07-tendermint-13")],
            note: String::new(),
        };
        let built = build_submit(&draft, "terp1proposer", "terp1gov", true).unwrap();
        assert!(built.submit.messages.is_empty());
        let mut with_conn = draft.clone();
        with_conn.subjects[0].connection_id = Some("connection-3".into());
        assert!(build_submit(&with_conn, "terp1proposer", "terp1gov", true).is_err());
        with_conn.subjects[0].closure = Some("STATE_CLOSED".into());
        assert!(build_submit(&with_conn, "terp1proposer", "terp1gov", true).is_ok());
        assert_eq!(built.submit.initial_deposit[0].amount, "5000000000");
        assert_eq!(built.submit.initial_deposit[0].denom, "uterp");
    }

    #[test]
    fn exported_hex_parses_without_keeping_the_text() {
        let line = "ab".repeat(32);
        let secret = parse_exported_secret(&format!("WARNING stay away\n{line}\n")).unwrap();
        assert_eq!(secret[0], 0xab);
        assert_eq!(secret[31], 0xab);
        assert!(parse_exported_secret("not a key\n").is_err());
        assert!(parse_exported_secret(&format!("{line}\n{line}\n")).is_err());
        let shown = redact_secrets(&format!("failed {line}"));
        assert!(!shown.contains(&line));
        assert!(shown.contains("[redacted]"));
    }

    #[test]
    fn insufficient_funds_is_not_an_unsound_message() {
        let err = "rpc error: spendable balance 100uterp is smaller than 5000000000uterp: insufficient funds";
        assert_eq!(classify_sim_error(err), SimClass::SubmitValidUnfunded);
        assert_eq!(
            classify_sim_error("invalid signer: terp1abc"),
            SimClass::Unsound
        );
    }
}
