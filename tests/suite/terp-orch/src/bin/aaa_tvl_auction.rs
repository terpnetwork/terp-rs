//! AAA DAO Touch Vinyl auctions on morocco-1, signed only through cw-orch-kms.
//!
//! Stage `instantiate`: MsgInstantiateContract of reserve-auction code 92 (AAA DAO admin + fee
//! manager, 0% fee, 1 USDC min reserve). The address is recorded in cw-orch state
//! (`revenue_reserve_auction`) and in `<out>/auction-address.txt`.
//! Stage `propose`: AAA DAO proposal through its pre-propose module with 3 cw721 send_nft msgs
//! (create_auction hook: reserve 1000000, 604800 s, bidding only) to that address.
//! Stage `all`: both, under one cw-orch-kms session (one Zigner approval).
//!
//! DRY RUN BY DEFAULT: read-only chain validation over the public LCD, the exact messages, and a
//! gas simulation (unsigned tx, signer = granter, POST /cosmos/tx/v1beta1/simulate). No key,
//! no session, nothing signed. `--broadcast` (plus AAA_TVL_BROADCAST=1) opens a cw-orch-kms
//! session: Zigner signs an authz grant to an ephemeral session key on a random-port loopback
//! approval page, the session key broadcasts MsgExec(granter){..} with fees in uthiol, then the
//! session revokes its grant. No terpd, no offline JSON, no key material is ever printed.
//!
//! Build: cargo build -p terp-orch --no-default-features --features gov --bin aaa_tvl_auction
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
use cosmos_sdk_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignerInfo, TxBody, TxRaw, mode_info,
};
use cosmos_sdk_proto::cosmwasm::wasm::v1::{MsgExecuteContract, MsgInstantiateContract};
use cosmos_sdk_proto::prost::Message;
use cosmwasm_std::{Binary, Coin, Decimal, Uint128};
use revenue_reserve_auction::msg::InstantiateMsg;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::str::FromStr;

const CHAIN_ID: &str = "morocco-1";
const CODE_ID: u64 = 92;
const CODE_CHECKSUM: &str = "B55B4E025575FC6A5982EAD25E6A4CA7C774C3EF2C6633ADB854463E8E3EA42F";
const AAA_DAO: &str = "terp1j6yclnqvmm2l5kghduc0ag7zzssvurwzhct0zxfq82jfglkckc7qxs7dd5";
const PRE_PROPOSE: &str = "terp1cmcz4nc5gcnnpgacv7hp3tuum8t5x30hl4uaqyuw4vp67kf586cs4yuk3s";
const CW721: &str = "terp1vansx03g9sxntv86y2f38zu28ddyjc36ful8a3fapdrl7xrmtsmq0jv4jv";
const TOKENS: [&str; 3] = ["AAA_TVL_00", "AAA_TVL_001", "AAA_TVL_002"];
const USDC: &str = "ibc/422FAC8ED757E2DEA4C475B86684ED037C6F3DC0211647D83C92EE54EC072E4E";
const RESERVE: u128 = 1_000_000;
const DURATION: u64 = 604_800;
const LABEL: &str = "pm-reserve-auction-aaa";
const STATE_ID: &str = "revenue_reserve_auction";
const PLACEHOLDER: &str = "__RESERVE_AUCTION_ADDR__";
const T_INST: &str = "/cosmwasm.wasm.v1.MsgInstantiateContract";
const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
const TITLE: &str = "List Touch Vinyls 00, 001, 002 in USDC (auction)";
const DESCRIPTION: &str = "Send Touch Vinyls AAA_TVL_00, AAA_TVL_001 and AAA_TVL_002 from the AAA DAO into the reserve auction __RESERVE_AUCTION_ADDR__. Each lot is an auction with a 1 USDC reserve; it runs 7 days from the first bid at or above the reserve, bids must beat the last by 5%, and bids in the last 10 minutes extend it by 10 minutes. No protocol fee: 100% of proceeds go to the AAA DAO, the seller.";

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Stage {
    Instantiate,
    Propose,
    All,
}

#[derive(Parser)]
#[command(about = "AAA DAO Touch Vinyl reserve auctions via cw-orch-kms (dry run by default)")]
struct Cli {
    #[arg(value_enum, default_value = "all")]
    stage: Stage,
    /// Sign and broadcast through cw-orch-kms. Also needs AAA_TVL_BROADCAST=1.
    #[arg(long)]
    broadcast: bool,
    /// Cold granter (Zigner). Sends the instantiate and the proposal; must be an AAA cw4 member.
    #[arg(long, default_value = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052")]
    granter: String,
    #[arg(long, env = "TERP_LCD", default_value = "https://api.terp.network")]
    lcd: String,
    #[arg(long, env = "TERP_RPC", default_value = "https://rpc.terp.network")]
    rpc: String,
    /// gRPC for --broadcast (cw-orch Daemon). Dry runs never use it.
    #[arg(
        long,
        env = "GRPC_URL",
        default_value = "https://grpc.terp.network:443"
    )]
    grpc_url: String,
    /// Auction address for `propose` (else cw-orch state, else placeholder in dry run).
    #[arg(long)]
    auction: Option<String>,
    #[arg(long, default_value = "aaa-tvl-auction-out")]
    out: PathBuf,
    #[arg(long, default_value_t = 0u32)]
    account_index: u32,
    #[arg(long, default_value_t = 30 * 60)]
    ttl_secs: u64,
    /// uthiol per unit gas.
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
    /// Reproduce the session build sequence without a signature: pre-flight build, page open until a short challenge expires, rebuild.
    #[arg(long)]
    session_selftest: bool,
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("warn,cw_orch=debug,cw-orch=debug,cw_orch_daemon=debug"),
    )
    .init();
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.out)?;
    println!(
        "mode: {}",
        if cli.broadcast {
            "BROADCAST (cw-orch-kms)"
        } else {
            "DRY RUN (simulate only, nothing signed)"
        }
    );
    let v = validate(&cli)?;
    let inst = instantiate_msg(&cli.granter)?;
    let (inst_any, inst_json) = inst;

    if !cli.broadcast {
        // Read-only gRPC check so a dry run also proves the endpoint --broadcast will use.
        if let Err(e) = grpc_preflight(&cli) {
            println!("[WARN] {e:#}");
        }
        let mut report =
            json!({"_NOT_SIGNED": "dry run; nothing signed or broadcast", "validation": v});
        if matches!(cli.stage, Stage::Instantiate | Stage::All) {
            print_msg("instantiate", &inst_json)?;
            let gas = simulate(&cli, &[inst_any.clone()])?;
            println!(
                "simulate instantiate: gas_used={gas} (fee @ {} uthiol/gas x1.3 ~= {} uthiol)",
                cli.gas_price,
                fee_for(gas, cli.gas_price)
            );
            report["instantiate"] = json!({"msg": inst_json, "gas_used": gas});
        }
        if matches!(cli.stage, Stage::Propose | Stage::All) {
            let addr = cli
                .auction
                .clone()
                .or_else(|| state_address().ok().flatten())
                .unwrap_or_else(|| PLACEHOLDER.to_string());
            println!(
                "auction address for proposal: {addr}{}",
                if addr == PLACEHOLDER {
                    " (placeholder: not instantiated yet)"
                } else {
                    ""
                }
            );
            let (p_any, p_json) = propose_msg(&cli.granter, &addr)?;
            print_msg("propose", &p_json)?;
            let gas = simulate(&cli, &[p_any])?;
            println!(
                "simulate propose: gas_used={gas} (fee ~= {} uthiol)",
                fee_for(gas, cli.gas_price)
            );
            report["propose"] = json!({"auction": addr, "msg": p_json, "gas_used": gas});
        }
        let path = cli.out.join("dry-run.json");
        std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;
        println!("wrote {}", path.display());
        println!(
            "DRY RUN done. Re-run with --broadcast and AAA_TVL_BROADCAST=1 to sign via cw-orch-kms."
        );
        return Ok(());
    }
    broadcast(&cli, inst_any)
}

// ---------------- messages ----------------
fn instantiate_msg(sender: &str) -> Result<(Any, Value)> {
    let msg = InstantiateMsg {
        fee_manager: AAA_DAO.into(),
        trading_fee_percent: Decimal::zero(),
        min_bid_increment_percent: Decimal::from_str("0.05").map_err(|e| anyhow!(e.to_string()))?,
        min_duration: 3600,
        max_duration: 1_209_600,
        extend_duration: 600,
        create_auction_fee: Coin::new(0u128, "uthiol"),
        max_auctions_to_settle_per_block: 10,
        halt_duration_threshold: 3600,
        halt_buffer_duration: 60,
        halt_postpone_duration: 60,
        burn_bps: 0,
        min_reserve_prices: vec![Coin::new(Uint128::new(RESERVE), USDC)],
    };
    let bytes = serde_json::to_vec(&msg)?;
    // Must equal the handoff's drafts/01-instantiate.msg.json exactly (semantic compare).
    let expected: Value = serde_json::from_str(
        r#"{"fee_manager":"terp1j6yclnqvmm2l5kghduc0ag7zzssvurwzhct0zxfq82jfglkckc7qxs7dd5","trading_fee_percent":"0","min_bid_increment_percent":"0.05","min_duration":3600,"max_duration":1209600,"extend_duration":600,"create_auction_fee":{"denom":"uthiol","amount":"0"},"max_auctions_to_settle_per_block":10,"halt_duration_threshold":3600,"halt_buffer_duration":60,"halt_postpone_duration":60,"burn_bps":0,"min_reserve_prices":[{"denom":"ibc/422FAC8ED757E2DEA4C475B86684ED037C6F3DC0211647D83C92EE54EC072E4E","amount":"1000000"}]}"#,
    )?;
    let got: Value = serde_json::from_slice(&bytes)?;
    if got != expected {
        bail!("instantiate msg drifted from the handoff: {got}");
    }
    let m = MsgInstantiateContract {
        sender: sender.into(),
        admin: AAA_DAO.into(),
        code_id: CODE_ID,
        label: LABEL.into(),
        msg: bytes,
        funds: vec![],
    };
    Ok((
        any(T_INST, &m),
        json!({"@type": T_INST, "sender": sender, "admin": AAA_DAO, "code_id": CODE_ID,
        "label": LABEL, "msg": got, "funds": []}),
    ))
}

fn hook() -> Value {
    json!({"create_auction": {"denom": USDC, "reserve_price": RESERVE.to_string(), "duration": DURATION, "is_instant_sale": false}})
}

fn propose_msg(sender: &str, auction: &str) -> Result<(Any, Value)> {
    let hook_b64 = Binary::from(serde_json::to_vec(&hook())?).to_base64();
    let msgs: Vec<Value> = TOKENS
        .iter()
        .map(|t| {
            let send = json!({"send_nft": {"contract": auction, "token_id": t, "msg": hook_b64}});
            json!({"wasm": {"execute": {"contract_addr": CW721,
            "msg": Binary::from(serde_json::to_vec(&send).unwrap()).to_base64(), "funds": []}}})
        })
        .collect();
    let exec = json!({"propose": {"msg": {"propose": {"title": TITLE,
        "description": DESCRIPTION.replace(PLACEHOLDER, auction), "msgs": msgs}}}});
    let m = MsgExecuteContract {
        sender: sender.into(),
        contract: PRE_PROPOSE.into(),
        msg: serde_json::to_vec(&exec)?,
        funds: vec![],
    };
    let decoded: Vec<Value> = TOKENS.iter().map(|t| json!({"cw721": CW721, "send_nft": {"contract": auction, "token_id": t, "msg(decoded)": hook()}})).collect();
    Ok((
        any(T_EXEC, &m),
        json!({"@type": T_EXEC, "sender": sender, "contract": PRE_PROPOSE, "funds": [],
        "msg": exec, "decoded_dao_msgs": decoded}),
    ))
}

fn print_msg(name: &str, v: &Value) -> Result<()> {
    println!(
        "== predicted msg [{name}]\n{}",
        serde_json::to_string_pretty(v)?
    );
    Ok(())
}

// ---------------- read-only chain validation ----------------
fn validate(cli: &Cli) -> Result<Value> {
    let lcd = cli.lcd.trim_end_matches('/');
    let mut fails = vec![];
    let mut ck = |ok: bool, what: String| {
        println!("[{}] {what}", if ok { " OK " } else { "FAIL" });
        if !ok {
            fails.push(what)
        }
    };

    let latest = get(lcd, "/cosmos/base/tendermint/v1beta1/blocks/latest")?;
    let chain = latest["block"]["header"]["chain_id"].as_str().unwrap_or("");
    ck(
        chain == CHAIN_ID,
        format!(
            "chain-id {chain} (height {})",
            latest["block"]["header"]["height"].as_str().unwrap_or("?")
        ),
    );
    let rpc = curl(&[&format!("{}/status", cli.rpc.trim_end_matches('/'))])
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    if let Some(r) = rpc {
        let net = r["result"]["node_info"]["network"]
            .as_str()
            .unwrap_or("")
            .to_string();
        ck(
            net == CHAIN_ID,
            format!(
                "rpc {} network {net}, height {}",
                cli.rpc,
                r["result"]["sync_info"]["latest_block_height"]
                    .as_str()
                    .unwrap_or("?")
            ),
        );
    }

    let code = get(lcd, &format!("/cosmwasm/wasm/v1/code/{CODE_ID}"))?;
    let hash = code["code_info"]["data_hash"]
        .as_str()
        .unwrap_or("")
        .to_uppercase();
    ck(
        hash == CODE_CHECKSUM,
        format!("code {CODE_ID} checksum {hash}"),
    );
    let perm = code["code_info"]["instantiate_permission"]["permission"]
        .as_str()
        .unwrap_or("")
        .to_string();
    ck(
        perm == "Everybody",
        format!("code {CODE_ID} instantiate permission {perm}"),
    );

    let mut owners = json!({});
    for t in TOKENS {
        let o = smart(lcd, CW721, &json!({"owner_of": {"token_id": t}}))?;
        let owner = o["data"]["owner"].as_str().unwrap_or("").to_string();
        ck(owner == AAA_DAO, format!("{t} owner {owner}"));
        owners[t] = json!(owner);
    }

    let well = USDC.len() == 68
        && USDC.starts_with("ibc/")
        && USDC[4..]
            .chars()
            .all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c));
    ck(well, format!("denom format {USDC}"));
    let trace = curl(&[
        "-s",
        &format!("{lcd}/ibc/apps/transfer/v1/denoms/{}", &USDC[4..]),
    ])
    .unwrap_or_default();
    let supply = get(
        lcd,
        &format!(
            "/cosmos/bank/v1beta1/supply/by_denom?denom={}",
            USDC.replace('/', "%2F")
        ),
    )
    .map(|s| s["amount"]["amount"].as_str().unwrap_or("?").to_string())
    .unwrap_or("?".into());
    let known = trace.contains("\"denom\"") && !trace.contains("not found");
    println!(
        "[{}] denom on-chain: trace {} / supply {supply}",
        if known { " OK " } else { "WARN" },
        if known {
            "found"
        } else {
            "NOT FOUND on morocco-1 (nobody on Terp holds this denom yet)"
        }
    );

    let modules = smart(lcd, AAA_DAO, &json!({"proposal_modules": {}}))?["data"].clone();
    let pp_dao = smart(lcd, PRE_PROPOSE, &json!({"dao": {}}))?["data"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let pp_mod = smart(lcd, PRE_PROPOSE, &json!({"proposal_module": {}}))?["data"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let pp_cfg = smart(lcd, PRE_PROPOSE, &json!({"config": {}}))?["data"].clone();
    ck(
        pp_dao == AAA_DAO,
        format!("pre-propose {PRE_PROPOSE} dao {pp_dao}"),
    );
    let enabled = modules
        .as_array()
        .map(|a| {
            a.iter()
                .any(|m| m["address"] == pp_mod.as_str() && m["status"] == "enabled")
        })
        .unwrap_or(false);
    ck(
        enabled,
        format!("pre-propose's proposal module {pp_mod} is an enabled AAA module"),
    );
    let policy = smart(lcd, &pp_mod, &json!({"proposal_creation_policy": {}}))?["data"].clone();
    ck(
        policy["module"]["addr"] == PRE_PROPOSE,
        format!(
            "proposal module creation policy -> {}",
            policy["module"]["addr"]
        ),
    );
    let deposit = pp_cfg["deposit_info"].clone();
    ck(
        deposit.is_null(),
        format!("pre-propose deposit_info {deposit} (none expected, funds [])"),
    );
    let submission = pp_cfg["submission_policy"].clone();
    println!("       submission_policy {submission}");
    let prop_cfg = smart(lcd, &pp_mod, &json!({"config": {}}))?["data"].clone();
    println!(
        "       voting: threshold {} max_voting_period {} only_members_execute {}",
        prop_cfg["threshold"], prop_cfg["max_voting_period"], prop_cfg["only_members_execute"]
    );

    let voting = smart(lcd, AAA_DAO, &json!({"voting_module": {}}))?["data"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let group = smart(lcd, &voting, &json!({"group_contract": {}}))?["data"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let members =
        smart(lcd, &group, &json!({"list_members": {"limit": 30}}))?["data"]["members"].clone();
    println!("       cw4 group {group} members {members}");
    let member = members
        .as_array()
        .map(|a| {
            a.iter()
                .any(|m| m["addr"] == cli.granter.as_str() && m["weight"].as_u64().unwrap_or(0) > 0)
        })
        .unwrap_or(false);
    let members_only = submission["specific"]["dao_members"] == true;
    ck(
        member || !members_only,
        format!("granter {} is a cw4 member: {member}", cli.granter),
    );

    let bal = get(
        lcd,
        &format!(
            "/cosmos/bank/v1beta1/balances/{}/by_denom?denom=uthiol",
            cli.granter
        ),
    )
    .map(|b| b["balance"]["amount"].as_str().unwrap_or("0").to_string())
    .unwrap_or("?".into());
    println!("       granter uthiol balance {bal} (pays the session feegrant)");

    if !fails.is_empty() {
        bail!("on-chain validation failed: {fails:?}");
    }
    Ok(
        json!({"chain_id": chain, "code_checksum": hash, "owners": owners, "denom_known_on_chain": known,
        "denom_supply": supply, "pre_propose": {"dao": pp_dao, "proposal_module": pp_mod, "config": pp_cfg},
        "proposal_config": prop_cfg, "cw4_group": group, "members": members, "granter_is_member": member,
        "granter_uthiol": bal}),
    )
}

fn curl(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("curl")
        .args(["-sf", "--max-time", "25"])
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!("curl failed: {}", args.last().unwrap_or(&""));
    }
    Ok(String::from_utf8(out.stdout)?)
}
fn get(lcd: &str, path: &str) -> Result<Value> {
    Ok(serde_json::from_str(&curl(&[&format!("{lcd}{path}")])?)?)
}
fn smart(lcd: &str, contract: &str, q: &Value) -> Result<Value> {
    let b64 = Binary::from(serde_json::to_vec(q)?).to_base64();
    get(
        lcd,
        &format!("/cosmwasm/wasm/v1/contract/{contract}/smart/{b64}"),
    )
    .with_context(|| format!("query {contract} {q}"))
}

// ---------------- simulation (dry run) ----------------
/// Unsigned tx with the granter as signer (no pubkey; simulate skips sig checks).
fn simulate(cli: &Cli, msgs: &[Any]) -> Result<u64> {
    let lcd = cli.lcd.trim_end_matches('/');
    let acct = get(
        lcd,
        &format!("/cosmos/auth/v1beta1/accounts/{}", cli.granter),
    )?;
    let seq: u64 = find_key(&acct["account"], "sequence")
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or_else(|| anyhow!("granter account has no sequence"))?
        .parse()?;
    let body = TxBody {
        messages: msgs.to_vec(),
        memo: "aaa-tvl-auction dry run".into(),
        ..Default::default()
    };
    let auth = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: None,
            sequence: seq,
            mode_info: Some(ModeInfo {
                sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
            }),
        }],
        fee: Some(Fee {
            amount: vec![ProtoCoin {
                denom: "uthiol".into(),
                amount: "0".into(),
            }],
            gas_limit: 0,
            payer: String::new(),
            granter: String::new(),
        }),
        ..Default::default()
    };
    let raw = TxRaw {
        body_bytes: body.encode_to_vec(),
        auth_info_bytes: auth.encode_to_vec(),
        signatures: vec![vec![]],
    };
    let req = json!({"tx_bytes": Binary::from(raw.encode_to_vec()).to_base64()}).to_string();
    let out = std::process::Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "30",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "--data",
            &req,
            &format!("{lcd}/cosmos/tx/v1beta1/simulate"),
        ])
        .output()?;
    let v: Value = serde_json::from_slice(&out.stdout).context("simulate response")?;
    match v["gas_info"]["gas_used"].as_str() {
        Some(g) => Ok(g.parse()?),
        None => bail!(
            "simulate failed: {}",
            v["message"].as_str().unwrap_or(&v.to_string())
        ),
    }
}
/// First `key` found depth-first (BaseAccount may be nested in vesting/module account types).
fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m
            .get(key)
            .or_else(|| m.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}
fn fee_for(gas: u64, price: f64) -> u64 {
    ((gas as f64) * 1.3 * price).ceil() as u64
}

// ---------------- cw-orch state ----------------
fn state_file() -> PathBuf {
    // Same resolution as cw-orch (STATE_FILE env, else ~/.cw-orchestrator/state.json).
    cw_orch::daemon::DaemonState::state_file_path()
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".cw-orchestrator/state.json")
        })
}
/// Read-only lookup of `revenue_reserve_auction` for morocco-1 in the cw-orch state file.
fn state_address() -> Result<Option<String>> {
    let Ok(raw) = std::fs::read(state_file()) else {
        return Ok(None);
    };
    let v: Value = serde_json::from_slice(&raw)?;
    let dep = std::env::var("CW_ORCH_DEPLOYMENT_ID").unwrap_or_else(|_| "default".into());
    Ok(v.get(CHAIN_ID)
        .and_then(|c| c.get(&dep))
        .and_then(|d| d.get(STATE_ID))
        .and_then(|a| a.as_str())
        .map(str::to_string))
}

// ---------------- broadcast (cw-orch-kms) ----------------
fn broadcast(cli: &Cli, inst_any: Any) -> Result<()> {
    use cw_orch::daemon::{DaemonBuilder, TxSender};
    use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
    use cw_orch::prelude::{ContractInstance, IndexResponse};
    use cw_orch_kms::progress::Status;
    if std::env::var("AAA_TVL_BROADCAST").ok().as_deref() != Some("1") {
        bail!("refusing to sign: --broadcast also needs AAA_TVL_BROADCAST=1");
    }
    let chain = ChainInfoOwned {
        chain_id: CHAIN_ID.into(),
        gas_denom: "uthiol".into(),
        gas_price: cli.gas_price,
        grpc_urls: vec![cli.grpc_url.clone()],
        rpc_urls: vec![],
        lcd_url: Some(cli.lcd.clone()),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: "terp".into(),
            coin_type: 118,
        },
        kind: ChainKind::Mainnet,
    };
    // SAFETY: single-threaded here, before cw-orch reads it.
    unsafe { std::env::set_var("CW_ORCH_WALLET_BALANCE_ASSERTION", "false") };
    let do_inst = matches!(cli.stage, Stage::Instantiate | Stage::All);
    let do_prop = matches!(cli.stage, Stage::Propose | Stage::All);
    let mut auction = cli.auction.clone().or(state_address()?);
    if do_prop && !do_inst && auction.is_none() {
        bail!("propose needs --auction or a recorded {STATE_ID} in cw-orch state");
    }
    // gRPC pre-flight BEFORE the approval page: a bad endpoint must not cost a Zigner signature.
    grpc_preflight(cli)?;
    // Build the very Daemon the session will use (ephemeral in-memory key, same chain info and
    // gRPC channel), so a connect failure in DaemonBuilder::build also surfaces here, unsigned.
    let mut builder = DaemonBuilder::new(chain);
    builder.load_network(false);
    cw_orch_kms::ensure_ephemeral_mnemonic(ChainKind::Mainnet).map_err(|e| anyhow!(e))?;
    {
        let d = builder
            .build()
            .map_err(|e| anyhow!("pre-flight Daemon build on {}: {e}", cli.grpc_url))?;
        use cw_orch::prelude::QueryHandler;
        let h = d
            .block_info()
            .map_err(|e| anyhow!("pre-flight Daemon query on {}: {e}", cli.grpc_url))?
            .height;
        println!(
            "[ OK ] cw-orch Daemon built on {} (session channel), height {h}",
            cli.grpc_url
        );
    }
    if cli.session_selftest {
        let page = cw_orch_kms::page_addr();
        println!(
            "selftest: page on port {} with a 45s challenge (do not sign)",
            page.port()
        );
        let r = cw_orch_kms::capture_granter_for_chain_opts_sync(
            page,
            cli.account_index,
            "aaa-tvl-selftest",
            CHAIN_ID,
            45,
            Some(&cli.granter),
        );
        println!(
            "selftest: page returned {:?}",
            r.as_ref().map(|_| "signed").map_err(|e| e.clone())
        );
        match builder.build() {
            Ok(_) => {
                println!("selftest: SECOND BUILD OK");
                return Ok(());
            }
            Err(e) => bail!("selftest: SECOND BUILD FAILED: {e}"),
        }
    }
    // Random loopback port (CW_ORCH_KMS_PORT overrides), reused for every page of this run.
    let page = cw_orch_kms::page_addr();
    println!(
        "sign the cw-orch-granter challenge on the approval page (port {})",
        page.port()
    );
    let granter = cw_orch_kms::capture_granter_for_chain_opts_sync(
        page,
        cli.account_index,
        "aaa-tvl-auction",
        CHAIN_ID,
        cli.ttl_secs,
        Some(&cli.granter),
    )
    .map_err(|e| anyhow!(e))?;
    if granter != cli.granter {
        bail!("Zigner granter {granter} != expected {}", cli.granter);
    }

    let mut session = cw_orch_kms::ApprovalSession::open(&granter);
    let mut txs = 1u64; // closing revoke
    if do_inst {
        session = session.allow_instantiate();
        txs += 1;
    }
    if do_prop {
        session = session
            .allow_execute(cw_orch_kms::ScopedCall {
                address: PRE_PROPOSE.into(),
                keys: vec!["propose".into()],
                calls: 1,
                funds: vec![],
            })
            .map_err(|e| anyhow!(e))?;
        txs += 1;
    }
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cw_orch_kms::planned_spend_limit(txs);
    let actions =
        vec![cw_orch_kms::SigningRequest::new("session-key").body("aaa-tvl-auction".to_string())];
    // Retry a gRPC connect failure without re-asking for the granter challenge.
    let mut attempt = 0;
    let run = loop {
        attempt += 1;
        match cw_orch_kms::enable_session(&builder, session.clone(), actions.clone()) {
            Ok(r) => break r,
            Err(e)
                if attempt < 5
                    && (e.contains("grpc") || e.contains("gRPC") || e.contains("transport")) =>
            {
                println!("[WARN] enable_session attempt {attempt}/5: {e}; retrying in 3s");
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
            Err(e) => bail!(e),
        }
    };
    let daemon = run.daemon.clone();
    println!("session key (grantee): {}", run.grantee());

    let send = |label: &str, msgs: Vec<Any>| -> Result<cw_orch::daemon::CosmTxResponse> {
        let memo = format!("aaa-tvl-auction {label}");
        let sim = vec![any(
            T_AUTHZ_EXEC,
            &cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec {
                grantee: run.grantee().into(),
                msgs: msgs.clone(),
            },
        )];
        run.step(label, Status::Running, None, None);
        let (gas, fee) = daemon
            .rt_handle
            .block_on(daemon.sender().simulate(sim, Some(&memo)))
            .map_err(|e| {
                run.step(label, Status::Error, None, Some(&e.to_string()));
                anyhow!(e)
            })?;
        println!("simulate {label}: gas={gas} fee={fee}");
        let resp = daemon
            .rt_handle
            .block_on(daemon.sender().commit_tx_any(msgs, Some(&memo)))
            .map_err(|e| {
                run.step(label, Status::Error, None, Some(&e.to_string()));
                anyhow!(e)
            })?;
        println!(
            "BROADCAST {label} txhash: {} code {}",
            resp.txhash, resp.code
        );
        if resp.code != 0 {
            run.step(
                label,
                Status::Error,
                Some(&resp.txhash),
                Some(&format!("code {}", resp.code)),
            );
            bail!("{label} failed: code {}", resp.code);
        }
        run.step(label, Status::Confirmed, Some(&resp.txhash), None);
        Ok(resp)
    };
    let result = (|| -> Result<()> {
        if do_inst {
            let resp = send("instantiate", vec![inst_any])?;
            let addr = resp
                .event_attr_value("instantiate", "_contract_address")
                .map_err(|e| anyhow!(e.to_string()))?;
            println!("AUCTION ADDRESS: {addr}");
            let c = revenue_reserve_auction::RevenueReserveAuction::new(daemon.clone());
            c.set_address(&cosmwasm_std::Addr::unchecked(&addr));
            println!(
                "recorded {STATE_ID} in cw-orch state {}",
                state_file().display()
            );
            std::fs::write(cli.out.join("auction-address.txt"), format!("{addr}\n"))?;
            auction = Some(addr);
        }
        if do_prop {
            let addr = auction
                .clone()
                .ok_or_else(|| anyhow!("no auction address"))?;
            let (p_any, p_json) = propose_msg(&granter, &addr)?;
            print_msg("propose", &p_json)?;
            send("propose", vec![p_any])?;
        }
        Ok(())
    })();
    run.close().map_err(|e| anyhow!(e))?;
    println!("session grant closed (revoked)");
    result
}

fn any<M: Message>(type_url: &str, m: &M) -> Any {
    Any {
        type_url: type_url.into(),
        value: m.encode_to_vec(),
    }
}

/// Connects through the exact cw-orch GrpcChannel the session Daemon uses, then checks
/// chain-id, that the node is at tip (vs the RPC height), and reads the granter balance.
fn grpc_preflight(cli: &Cli) -> Result<()> {
    use cw_orch::daemon::{
        GrpcChannel, RUNTIME,
        queriers::{Bank, Node},
    };
    let url = cli.grpc_url.clone();
    let (height, bal) = RUNTIME.block_on(async {
        let ch = tokio_timeout(GrpcChannel::connect(&[url.clone()], CHAIN_ID))
            .await
            .map_err(|e| anyhow!("gRPC pre-flight {url}: {e}"))?;
        let h = Node::new_async(ch.clone())
            ._block_height()
            .await
            .map_err(|e| anyhow!("gRPC pre-flight {url} block height: {e}"))?;
        let b = Bank::new_async(ch)
            ._balance(
                &cosmwasm_std::Addr::unchecked(cli.granter.clone()),
                Some("uthiol".to_string()),
            )
            .await
            .map_err(|e| anyhow!("gRPC pre-flight {url} bank balance: {e}"))?;
        Ok::<_, anyhow::Error>((
            h,
            b.first().map(|c| c.amount.to_string()).unwrap_or_default(),
        ))
    })?;
    let rpc_h: u64 = get(
        cli.lcd.trim_end_matches('/'),
        "/cosmos/base/tendermint/v1beta1/blocks/latest",
    )
    .ok()
    .and_then(|v| {
        v["block"]["header"]["height"]
            .as_str()
            .and_then(|h| h.parse().ok())
    })
    .unwrap_or(0);
    if rpc_h > 0 && height + 20 < rpc_h {
        bail!(
            "gRPC pre-flight {url}: node at {height} lags rpc tip {rpc_h}; pick a synced endpoint"
        );
    }
    println!(
        "[ OK ] grpc {url} (cw-orch channel) {CHAIN_ID} height {height}, granter uthiol {bal}"
    );
    Ok(())
}

async fn tokio_timeout<F: std::future::Future<Output = Result<T, E>>, T, E: std::fmt::Display>(
    f: F,
) -> Result<T, String> {
    match cw_orch::tokio::time::timeout(std::time::Duration::from_secs(20), f).await {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err("timed out after 20s".into()),
    }
}
