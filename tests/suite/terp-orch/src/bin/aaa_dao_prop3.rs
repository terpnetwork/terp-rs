//! AAA DAO proposal #3 (list AAA_TVL_00/001/002 in the reserve auction): `vote yes` and `execute`.
//!
//! DRY RUN BY DEFAULT: read-only LCD preflight plus an unsigned gas simulation as the signer.
//!   --vote     (+ AAA_DAO_PROP3_VOTE=1)    sign `vote yes` through cw-orch-kms.
//!   --execute  (+ AAA_DAO_PROP3_EXECUTE=1) sign `execute` through cw-orch-kms.
//! Preflight (both): DAO wiring (proposal module + cw4 voting), prop #3 msgs byte-for-byte equal to
//! the intended 3 send_nft -> create_auction msgs, the DAO still owns the 3 tokens.
//! Vote: signer is a cw4 member with weight > 0, has not voted, status open, not expired.
//! Execute: status passed.
//! The cw-orch-kms grant is scoped to the proposal module, one call, key `vote` or `execute`.
//!
//! Build: cargo build -p terp-orch --features gov --bin aaa_dao_prop3
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
use cosmos_sdk_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignerInfo, TxBody, TxRaw, mode_info,
};
use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgExecuteContract;
use cosmos_sdk_proto::prost::Message;
use cosmwasm_std::Binary;
use serde_json::{Value, json};
use std::path::PathBuf;

const CHAIN_ID: &str = "morocco-1";
const AAA_DAO: &str = "terp1j6yclnqvmm2l5kghduc0ag7zzssvurwzhct0zxfq82jfglkckc7qxs7dd5";
const PROP_MODULE: &str = "terp1j5h5zftg5su7ytz74f7rryl4f6x3p78lh907fw39eqhax75r94jsv7qy7h";
const VOTING: &str = "terp1028e0vmjtz9ccrueww76pyl76yak480tu0hh5d5tjlu4z9psh0zse2e863";
const PROP_ID: u64 = 3;
const CW721: &str = "terp1vansx03g9sxntv86y2f38zu28ddyjc36ful8a3fapdrl7xrmtsmq0jv4jv";
const AUCTION: &str = "terp1slpz5656zy2ghajmpc09dw2fd2mcm66g8z5c4vxpl2wl0436tw8qn8ua0h";
const TOKENS: [&str; 3] = ["AAA_TVL_00", "AAA_TVL_001", "AAA_TVL_002"];
const USDC: &str = "ibc/422FAC8ED757E2DEA4C475B86684ED037C6F3DC0211647D83C92EE54EC072E4E";
const RESERVE: &str = "1000000";
const DURATION: u64 = 604_800;
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Action {
    Vote,
    Execute,
}
impl Action {
    fn key(self) -> &'static str {
        match self {
            Action::Vote => "vote",
            Action::Execute => "execute",
        }
    }
    fn env(self) -> &'static str {
        match self {
            Action::Vote => "AAA_DAO_PROP3_VOTE",
            Action::Execute => "AAA_DAO_PROP3_EXECUTE",
        }
    }
    fn flag(self) -> &'static str {
        match self {
            Action::Vote => "--vote",
            Action::Execute => "--execute",
        }
    }
    fn msg(self) -> Value {
        match self {
            Action::Vote => {
                json!({"vote": {"proposal_id": PROP_ID, "vote": "yes", "rationale": null}})
            }
            Action::Execute => json!({"execute": {"proposal_id": PROP_ID}}),
        }
    }
}

#[derive(Parser)]
#[command(about = "AAA DAO prop #3 vote yes / execute via cw-orch-kms (dry run by default)")]
struct Cli {
    /// Sign and broadcast `vote yes`. Also needs AAA_DAO_PROP3_VOTE=1.
    #[arg(long, conflicts_with = "execute")]
    vote: bool,
    /// Sign and broadcast `execute`. Also needs AAA_DAO_PROP3_EXECUTE=1.
    #[arg(long)]
    execute: bool,
    /// What to simulate in dry run (default vote).
    #[arg(long, value_enum, default_value = "vote")]
    simulate: Action,
    /// Signer (cw4 member). Required: pick the member who signs.
    #[arg(long)]
    granter: String,
    #[arg(long, env = "TERP_LCD", default_value = "https://api.terp.network")]
    lcd: String,
    #[arg(long, env = "GRPC_URL", default_value = "http://192.168.10.101:9090")]
    grpc_url: String,
    #[arg(long, default_value = "aaa-dao-prop3-out")]
    out: PathBuf,
    #[arg(long, default_value_t = 0u32)]
    account_index: u32,
    #[arg(long, default_value_t = 30 * 60)]
    ttl_secs: u64,
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.out)?;
    let live = if cli.vote {
        Some(Action::Vote)
    } else if cli.execute {
        Some(Action::Execute)
    } else {
        None
    };
    let action = live.unwrap_or(cli.simulate);
    println!(
        "mode: {} ({})",
        if live.is_some() {
            "BROADCAST (cw-orch-kms)"
        } else {
            "DRY RUN (simulate only, nothing signed)"
        },
        action.key()
    );
    let (v, fails) = preflight(&cli, action)?;
    let (anys, msg_json) = exec_msg(&cli.granter, action)?;
    println!("== msg\n{}", serde_json::to_string_pretty(&msg_json)?);
    if live.is_none() {
        if let Err(e) = grpc_preflight(&cli) {
            println!("[WARN] {e:#}");
        }
        let gas = match simulate(&cli, &anys) {
            Ok(g) => Some(g),
            Err(e) => {
                println!("[WARN] {e:#}");
                None
            }
        };
        if let Some(gas) = gas {
            println!(
                "simulate {} (sender {}): gas_used={gas} (fee @ {} uthiol/gas x1.3 ~= {} uthiol)",
                action.key(),
                cli.granter,
                cli.gas_price,
                fee_for(gas, cli.gas_price)
            );
        }
        let report = json!({"_NOT_SIGNED": "dry run", "action": action.key(), "preflight": v, "preflight_failures": fails, "msg": msg_json, "gas_used": gas});
        let path = cli.out.join(format!("dry-run-{}.json", action.key()));
        std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;
        println!("wrote {}\nDRY RUN done.", path.display());
        return Ok(());
    }
    if !fails.is_empty() {
        bail!("refusing to sign: preflight failed: {fails:?}");
    }
    broadcast(&cli, action, anys)
}

fn exec_msg(sender: &str, action: Action) -> Result<(Vec<Any>, Value)> {
    let exec = action.msg();
    let m = MsgExecuteContract {
        sender: sender.into(),
        contract: PROP_MODULE.into(),
        msg: serde_json::to_vec(&exec)?,
        funds: vec![],
    };
    Ok((
        vec![any(T_EXEC, &m)],
        json!({"@type": T_EXEC, "sender": sender, "contract": PROP_MODULE, "funds": [], "msg": exec}),
    ))
}

/// The 3 msgs prop #3 is meant to carry, decoded.
fn expected_msgs() -> Vec<Value> {
    TOKENS.iter().map(|t| json!({"contract_addr": CW721, "funds": [], "msg": {"send_nft": {
        "contract": AUCTION, "token_id": t,
        "msg": {"create_auction": {"denom": USDC, "duration": DURATION, "is_instant_sale": false, "reserve_price": RESERVE}}}}})).collect()
}

fn b64json(v: &Value) -> Result<Value> {
    let s = v
        .as_str()
        .ok_or_else(|| anyhow!("expected base64 string"))?;
    Ok(serde_json::from_slice(
        Binary::from_base64(s)
            .map_err(|e| anyhow!("{e}"))?
            .as_slice(),
    )?)
}

/// Decode on-chain wasm.execute msgs (outer msg and send_nft hook are base64).
fn decode_msgs(msgs: &Value) -> Result<Vec<Value>> {
    let mut out = vec![];
    for m in msgs
        .as_array()
        .ok_or_else(|| anyhow!("msgs not an array"))?
    {
        let e = &m["wasm"]["execute"];
        if e.is_null() {
            bail!("unexpected msg kind: {m}");
        }
        let mut inner = b64json(&e["msg"])?;
        if let Some(h) = inner.get("send_nft").and_then(|s| s.get("msg")).cloned() {
            inner["send_nft"]["msg"] = b64json(&h)?;
        }
        out.push(json!({"contract_addr": e["contract_addr"], "funds": e["funds"], "msg": inner}));
    }
    Ok(out)
}

fn preflight(cli: &Cli, action: Action) -> Result<(Value, Vec<String>)> {
    let lcd = cli.lcd.trim_end_matches('/');
    let mut fails = vec![];
    let mut ck = |ok: bool, what: String| {
        println!("[{}] {what}", if ok { " OK " } else { "FAIL" });
        if !ok {
            fails.push(what)
        }
    };
    let latest = get(lcd, "/cosmos/base/tendermint/v1beta1/blocks/latest")?;
    let chain = latest["block"]["header"]["chain_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    ck(chain == CHAIN_ID, format!("chain-id {chain}"));
    let mods = smart(lcd, AAA_DAO, &json!({"proposal_modules": {}}))?["data"].clone();
    let has = mods
        .as_array()
        .map(|a| {
            a.iter()
                .any(|m| m["address"] == PROP_MODULE && m["status"] == "enabled")
        })
        .unwrap_or(false);
    ck(
        has,
        format!("{PROP_MODULE} is an enabled proposal module of the AAA DAO"),
    );
    let vm = smart(lcd, AAA_DAO, &json!({"voting_module": {}}))?["data"].clone();
    ck(vm == VOTING, format!("voting module {vm}"));
    let group = smart(lcd, VOTING, &json!({"group_contract": {}}))?["data"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let p = smart(
        lcd,
        PROP_MODULE,
        &json!({"proposal": {"proposal_id": PROP_ID}}),
    )?["data"]
        .clone();
    let prop = &p["proposal"];
    let status = prop["status"].as_str().unwrap_or("").to_string();
    println!(
        "prop #{PROP_ID} \"{}\" status={status} votes={} total_power={} expiration={}",
        prop["title"].as_str().unwrap_or(""),
        prop["votes"],
        prop["total_power"],
        prop["expiration"]
    );
    let got = decode_msgs(&prop["msgs"])?;
    let want = expected_msgs();
    ck(
        got == want,
        format!(
            "prop #{PROP_ID} msgs equal the 3 intended send_nft -> {AUCTION} create_auction ({USDC} {RESERVE}, {DURATION}s)"
        ),
    );
    if got != want {
        println!(
            "on-chain: {}\nexpected: {}",
            serde_json::to_string(&got)?,
            serde_json::to_string(&want)?
        );
    }
    for t in TOKENS {
        let o = smart(lcd, CW721, &json!({"owner_of": {"token_id": t}}))?["data"]["owner"].clone();
        ck(o == AAA_DAO, format!("{t} owner {o}"));
    }
    let member =
        smart(lcd, &group, &json!({"member": {"addr": cli.granter}}))?["data"]["weight"].clone();
    match action {
        Action::Vote => {
            ck(
                member.as_u64().unwrap_or(0) > 0,
                format!(
                    "signer {} is a cw4 member of {group} (weight {member})",
                    cli.granter
                ),
            );
            let vote = smart(
                lcd,
                PROP_MODULE,
                &json!({"get_vote": {"proposal_id": PROP_ID, "voter": cli.granter}}),
            )?["data"]["vote"]
                .clone();
            ck(vote.is_null(), format!("signer has not voted yet ({vote})"));
            ck(status == "open", format!("status {status} is open"));
            let now: u128 = latest["block"]["header"]["time"]
                .as_str()
                .and_then(|t| chrono_ns(t))
                .unwrap_or(0);
            let exp: u128 = prop["expiration"]["at_time"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            ck(
                exp == 0 || now < exp,
                format!("not expired (block time ns {now} < {exp})"),
            );
        }
        Action::Execute => {
            ck(status == "passed", format!("status {status} is passed"));
        }
    }
    let members = smart(lcd, &group, &json!({"list_members": {}}))?["data"]["members"].clone();
    Ok((
        json!({"chain_id": chain, "proposal_module": PROP_MODULE, "status": status, "votes": prop["votes"], "threshold": prop["threshold"],
        "total_power": prop["total_power"], "expiration": prop["expiration"], "cw4_group": group, "members": members, "msgs_decoded": got}),
        fails,
    ))
}

/// RFC3339 (UTC, Z) block time to unix nanos, no extra deps.
fn chrono_ns(t: &str) -> Option<u128> {
    let (date, rest) = t.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, dd) = (d.next()??, d.next()??, d.next()??);
    let rest = rest.trim_end_matches('Z');
    let (hms, frac) = rest.split_once('.').unwrap_or((rest, "0"));
    let mut h = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (hh, mi, ss) = (h.next()??, h.next()??, h.next()??);
    let (yy, mm) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hh * 3600 + mi * 60 + ss;
    let ns: u128 = format!("{:0<9}", &frac[..frac.len().min(9)]).parse().ok()?;
    Some(secs as u128 * 1_000_000_000 + ns)
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

fn simulate(cli: &Cli, msgs: &[Any]) -> Result<u64> {
    let lcd = cli.lcd.trim_end_matches('/');
    let acct = get(
        lcd,
        &format!("/cosmos/auth/v1beta1/accounts/{}", cli.granter),
    )?;
    let seq: u64 = find_key(&acct["account"], "sequence")
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or_else(|| anyhow!("signer account has no sequence"))?
        .parse()?;
    let body = TxBody {
        messages: msgs.to_vec(),
        memo: "aaa-dao-prop3 dry run".into(),
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
fn any<M: Message>(type_url: &str, m: &M) -> Any {
    Any {
        type_url: type_url.into(),
        value: m.encode_to_vec(),
    }
}

fn broadcast(cli: &Cli, action: Action, msgs: Vec<Any>) -> Result<()> {
    use cw_orch::daemon::{DaemonBuilder, TxSender};
    use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
    use cw_orch_kms::progress::Status;
    if std::env::var(action.env()).ok().as_deref() != Some("1") {
        bail!(
            "refusing to sign: {} also needs {}=1",
            action.flag(),
            action.env()
        );
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
    grpc_preflight(cli)?;
    let mut builder = DaemonBuilder::new(chain);
    builder.load_network(false);
    cw_orch_kms::ensure_ephemeral_mnemonic(ChainKind::Mainnet).map_err(|e| anyhow!(e))?;
    builder
        .build()
        .map_err(|e| anyhow!("pre-flight Daemon build on {}: {e}", cli.grpc_url))?;
    let page = cw_orch_kms::page_addr();
    println!(
        "sign the cw-orch-granter challenge on the approval page (port {})",
        page.port()
    );
    let granter = cw_orch_kms::capture_granter_for_chain_opts_sync(
        page,
        cli.account_index,
        "aaa-dao-prop3",
        CHAIN_ID,
        cli.ttl_secs,
        Some(&cli.granter),
    )
    .map_err(|e| anyhow!(e))?;
    if granter != cli.granter {
        bail!("Zigner granter {granter} != expected {}", cli.granter);
    }
    let mut session = cw_orch_kms::ApprovalSession::open(&granter)
        .allow_execute(cw_orch_kms::ScopedCall {
            address: PROP_MODULE.into(),
            keys: vec![action.key().into()],
            calls: 1,
            funds: vec![],
        })
        .map_err(|e| anyhow!(e))?;
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cw_orch_kms::planned_spend_limit(2);
    let actions = vec![
        cw_orch_kms::SigningRequest::new("session-key")
            .body(format!("aaa-dao-prop3 {}", action.key())),
    ];
    let run = cw_orch_kms::enable_session(&builder, session, actions).map_err(|e| anyhow!(e))?;
    let daemon = run.daemon.clone();
    let label = action.key();
    let result = (|| -> Result<()> {
        let memo = format!("aaa-dao prop {PROP_ID} {label}");
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
            .map_err(|e| anyhow!(e))?;
        println!("simulate {label}: gas={gas} fee={fee}");
        let resp = daemon
            .rt_handle
            .block_on(daemon.sender().commit_tx_any(msgs, Some(&memo)))
            .map_err(|e| anyhow!(e))?;
        println!(
            "BROADCAST {label} txhash: {} code {}",
            resp.txhash, resp.code
        );
        if resp.code != 0 {
            run.step(label, Status::Error, Some(&resp.txhash), None);
            bail!("code {}", resp.code);
        }
        run.step(label, Status::Confirmed, Some(&resp.txhash), None);
        Ok(())
    })();
    run.close().map_err(|e| anyhow!(e))?;
    println!("session grant closed (revoked)");
    result
}

fn grpc_preflight(cli: &Cli) -> Result<()> {
    use cw_orch::daemon::{GrpcChannel, RUNTIME, queriers::Node};
    let url = cli.grpc_url.clone();
    let h = RUNTIME.block_on(async {
        let ch = cw_orch::tokio::time::timeout(
            std::time::Duration::from_secs(20),
            GrpcChannel::connect(&[url.clone()], CHAIN_ID),
        )
        .await
        .map_err(|_| anyhow!("gRPC {url}: timed out"))?
        .map_err(|e| anyhow!("gRPC {url}: {e}"))?;
        Node::new_async(ch)
            ._block_height()
            .await
            .map_err(|e| anyhow!("gRPC {url} height: {e}"))
    })?;
    println!("[ OK ] grpc {url} {CHAIN_ID} height {h}");
    Ok(())
}
