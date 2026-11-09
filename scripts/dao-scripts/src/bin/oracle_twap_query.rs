//! First cross-chain TWAP: morocco-1 Polytone note -> Osmosis voice stargate
//! `ArithmeticTwapToNow`, callback into the oracle-registry (CallbackHint).
//! Tooling only. Dry-run by default; `--broadcast --chain-id morocco-1
//! --granter <addr>` with TERP_SCRIPT_BROADCAST=1 goes through kms_gate.
//!
//! Batch: `--markets <file>` (JSON list of {market_id, pool_id, base_denom,
//! quote_denom}) sends N `note.query` executes in ONE tx under ONE approval
//! (scope: execute `query` on the note, calls = N). One execute per market
//! because the registry callback records only the FIRST query result against
//! the single CallbackHint. Every hint shares the current morocco-1 height;
//! rows are keyed by (market_id, height) so they stay distinct.
//! The dry run also simulates the unsigned tx on the LCD as --granter.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use clap::Parser;
use cw_orch::prelude::*;
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};

const TWAP_PATH: &str = "/osmosis.twap.v1beta1.Query/ArithmeticTwapToNow";

#[derive(Parser, Debug)]
#[command(name = "oracle_twap_query")]
struct Cli {
    #[arg(long, default_value = "terp1qxww83x84y8th3gsldqjq2j4haal3awgkr3ce7mut69s39cgaj0s5j4usj")]
    note: String,
    #[arg(long, default_value = "terp198ddes0gjrufm4a7csky203fafzk0g69u4w3yuycqnsm7lt30rsq9nyt69")]
    registry: String,
    #[arg(long, default_value = "OSMO/USDC")]
    market_id: String,
    #[arg(long, default_value = "osmosis-1")]
    source_chain: String,
    #[arg(long, default_value_t = 1464)]
    pool_id: u64,
    #[arg(long, default_value = "uosmo")]
    base: String,
    #[arg(long, default_value = "ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4")]
    quote: String,
    /// TWAP window: start_time = morocco-1 block time - N minutes (always in the past).
    #[arg(long, default_value_t = 15)]
    minutes: i64,
    /// CallbackHint.height. Default: current morocco-1 height (must be > 0, unique per market).
    #[arg(long)]
    height: Option<u64>,
    #[arg(long, default_value_t = 600)]
    timeout_seconds: u64,
    #[arg(long, default_value = "plan-twap-query.json")]
    out: String,
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
    /// Batch file: JSON list of {market_id, pool_id, base_denom, quote_denom}.
    #[arg(long)]
    markets: Option<String>,
    /// Morocco LCD for the dry-run simulation (needs --granter).
    #[arg(long, default_value = "http://192.168.10.101:1317")]
    lcd: String,
    #[arg(long, default_value = "uthiol")]
    fee_denom: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
struct Market {
    market_id: String,
    pool_id: u64,
    base_denom: String,
    quote_denom: String,
}

/// Unsigned simulate on the LCD as `signer` (no session, no signature).
fn simulate(lcd: &str, signer: &str, denom: &str, msgs: &[cosmrs::Any]) -> Result<u64> {
    use cosmos_sdk_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
    use cosmos_sdk_proto::cosmos::tx::v1beta1::{
        AuthInfo, Fee, ModeInfo, SignerInfo, TxBody, TxRaw, mode_info,
    };
    use cosmos_sdk_proto::prost::Message;
    let lcd = lcd.trim_end_matches('/');
    let curl = |args: &[&str]| -> Result<serde_json::Value> {
        let out = std::process::Command::new("curl").args(args).output()?;
        serde_json::from_slice(&out.stdout).context("lcd response")
    };
    let acct = curl(&["-s", "--max-time", "20", &format!("{lcd}/cosmos/auth/v1beta1/accounts/{signer}")])?;
    let seq: u64 = acct["account"]["sequence"]
        .as_str()
        .or_else(|| acct["account"]["base_account"]["sequence"].as_str())
        .or_else(|| acct["account"]["base_vesting_account"]["base_account"]["sequence"].as_str())
        .ok_or_else(|| anyhow!("{signer}: no account/sequence on chain"))?
        .parse()?;
    let body = TxBody { messages: msgs.to_vec(), memo: "oracle_twap_query dry run".into(), ..Default::default() };
    let auth = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: None,
            sequence: seq,
            mode_info: Some(ModeInfo { sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })) }),
        }],
        fee: Some(Fee {
            amount: vec![ProtoCoin { denom: denom.into(), amount: "0".into() }],
            gas_limit: 0,
            payer: String::new(),
            granter: String::new(),
        }),
        ..Default::default()
    };
    let raw = TxRaw { body_bytes: body.encode_to_vec(), auth_info_bytes: auth.encode_to_vec(), signatures: vec![vec![]] };
    let b64 = base64::engine::general_purpose::STANDARD.encode(raw.encode_to_vec());
    let req = serde_json::json!({ "tx_bytes": b64 }).to_string();
    let v = curl(&["-s", "--max-time", "30", "-X", "POST", "-H", "content-type: application/json",
        "--data", &req, &format!("{lcd}/cosmos/tx/v1beta1/simulate")])?;
    match v["gas_info"]["gas_used"].as_str() {
        Some(g) => Ok(g.parse()?),
        None => bail!("simulate failed: {}", v["message"].as_str().map(str::to_string).unwrap_or(v.to_string())),
    }
}

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}
fn field_bytes(tag: u64, b: &[u8], out: &mut Vec<u8>) {
    varint((tag << 3) | 2, out);
    varint(b.len() as u64, out);
    out.extend_from_slice(b);
}

/// osmosis.twap.v1beta1.ArithmeticTwapToNowRequest
/// {1: pool_id, 2: base_asset, 3: quote_asset, 4: start_time (Timestamp{1: seconds, 2: nanos})}
fn encode_twap_req(pool: u64, base: &str, quote: &str, secs: i64) -> Vec<u8> {
    let mut o = Vec::new();
    varint(1 << 3, &mut o);
    varint(pool, &mut o);
    field_bytes(2, base.as_bytes(), &mut o);
    field_bytes(3, quote.as_bytes(), &mut o);
    let mut ts = Vec::new();
    varint(1 << 3, &mut ts);
    varint(secs as u64, &mut ts);
    field_bytes(4, &ts, &mut o);
    o
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let b64 = base64::engine::general_purpose::STANDARD;
    let info = kms_gate::morocco_info();
    let md = kms_gate::query_only(&info)?;
    let block = md.block_info()?;
    // Release the read-only daemon (and its cw-orch state lock) before kms_broadcast opens its own.
    drop(md);
    let now = block.time.seconds() as i64;
    let start = now - cli.minutes * 60;
    if cli.minutes <= 0 || start >= now {
        bail!("start_time must be in the past");
    }
    let height = cli.height.unwrap_or(block.height);
    if height == 0 {
        bail!("height must be > 0");
    }
    let markets: Vec<Market> = match &cli.markets {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).with_context(|| p.clone())?)
            .with_context(|| format!("{p}: expected [{{market_id,pool_id,base_denom,quote_denom}}]"))?,
        None => vec![Market {
            market_id: cli.market_id.clone(),
            pool_id: cli.pool_id,
            base_denom: cli.base.clone(),
            quote_denom: cli.quote.clone(),
        }],
    };
    if markets.is_empty() {
        bail!("no markets");
    }
    let mut seen = std::collections::BTreeSet::new();
    for m in &markets {
        if !seen.insert(m.market_id.clone()) {
            bail!("duplicate market_id {} (rows are keyed by (market_id, height))", m.market_id);
        }
    }
    let start_rfc = chrono_like(start);
    let mut msgs = vec![];
    let mut rows = vec![];
    for m in &markets {
        let data = encode_twap_req(m.pool_id, &m.base_denom, &m.quote_denom, start);
        let hint = serde_json::json!({
            "market_id": m.market_id, "source_chain": cli.source_chain,
            "pool_id": m.pool_id, "height": height,
        });
        let msg = serde_json::json!({"query": {
            "msgs": [{"stargate": {"path": TWAP_PATH, "data": b64.encode(&data)}}],
            "callback": {"receiver": cli.registry, "msg": b64.encode(serde_json::to_vec(&hint)?)},
            "timeout_seconds": cli.timeout_seconds.to_string(),
        }});
        rows.push(serde_json::json!({
            "callback_hint": hint,
            "osmosis_lcd_equivalent": format!(
                "https://lcd.osmosis.zone/osmosis/twap/v1beta1/ArithmeticTwapToNow?pool_id={}&base_asset={}&quote_asset={}&start_time={}",
                m.pool_id, m.base_denom, m.quote_denom, start_rfc),
            "tx_msg": {"type": "/cosmwasm.wasm.v1.MsgExecuteContract", "contract": cli.note, "msg": msg, "funds": []},
        }));
        msgs.push(msg);
    }
    let n = msgs.len() as u64;
    let sender = if cli.granter.is_empty() { "<granter>".to_string() } else { cli.granter.clone() };
    let sim = if cli.granter.is_empty() {
        Err(anyhow!("no --granter: simulation skipped"))
    } else {
        let anys = msgs.iter().map(|m| kms_gate::execute_any(&cli.granter, &cli.note, m)).collect::<Result<Vec<_>>>()?;
        simulate(&cli.lcd, &cli.granter, &cli.fee_denom, &anys)
    };
    let mut plan = serde_json::json!({
        "dry_run": !cli.broadcast,
        "morocco-1": {"height": block.height, "time_unix": now},
        "start_time": start_rfc, "start_time_unix": start,
        "sender": sender, "note": cli.note, "registry": cli.registry,
        "n_executes": n,
        "session_scope": {"allow_execute": {"address": cli.note, "keys": ["query"], "calls": n}, "ttl_secs": cli.ttl_secs},
        "simulated_gas_used": sim.as_ref().ok(),
        "simulate_error": sim.as_ref().err().map(|e| e.to_string()),
        "rows": rows,
    });
    if n == 1 {
        // Keep the single-query plan shape.
        plan["callback_hint"] = plan["rows"][0]["callback_hint"].clone();
        plan["osmosis_lcd_equivalent"] = plan["rows"][0]["osmosis_lcd_equivalent"].clone();
        plan["tx"] = serde_json::json!({"type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": sender, "contract": cli.note, "msg": msgs[0], "funds": []});
    }
    kms_gate::write_plan(&cli.out, &plan)?;
    match &sim {
        Ok(g) => println!("{n} note.query executes, simulated gas_used {g}"),
        Err(e) => println!("{n} note.query executes, simulation: {e}"),
    }
    if !cli.broadcast {
        return Ok(());
    }
    if cli.granter.is_empty() {
        bail!("broadcast refused: pass --granter");
    }
    if let Err(e) = &sim {
        bail!("broadcast refused: simulation failed: {e}");
    }
    let anys = msgs.iter().map(|m| kms_gate::execute_any(&cli.granter, &cli.note, m)).collect::<Result<Vec<_>>>()?;
    let ids: Vec<&str> = markets.iter().map(|m| m.market_id.as_str()).collect();
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "oracle-twap-query".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Execute(cw_orch_kms::ScopedCall {
                address: cli.note.clone(),
                keys: vec!["query".into()],
                calls: n,
                funds: vec![],
            }),
            summary: format!("Polytone TWAP queries x{n} ({}) at height {height} -> registry", ids.join(", ")),
        },
        anys,
        "oracle_twap_query",
    )?;
    println!("tx {tx}");
    Ok(())
}

/// RFC3339 (UTC, seconds) without extra deps.
fn chrono_like(t: i64) -> String {
    let days = t.div_euclid(86400);
    let s = t.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, (s / 60) % 60, s % 60)
}
