//! Day-2 operations on owner-gated contracts (svg-minter factory, series) through cw-orch-kms.
//! Every step is a cw-orch command; no `terpd tx` / `terpd q` step is needed.
//!
//!   exec             granter acts: a one-shot session key sends MsgExec{MsgExecuteContract{sender: granter}};
//!                    the session grants are revoked after.
//!   grant-operator   granter's cold key grants --operator (another wallet) a retained, scoped
//!                    ContractExecutionAuthorization (--keys, --calls, --ttl-secs) + feegrant. Nothing is executed.
//!   operator-exec    the operator uses that grant through its own short session. Its cold key signs ONE
//!                    session grant (on --bind) to an in-memory session key: MsgExec (nested authz) + a fee
//!                    allowance, ttl <= 900s. The session key then sends
//!                    MsgExec{MsgExec{MsgExecuteContract{sender: granter}}} for each --msg/--msg-file, and the
//!                    session grants are revoked at close. The cold key never signs the messages themselves.
//!                    The operator grant is queried and printed first; a missing, expired, used-up or
//!                    non-matching grant (every message checked) is refused before anything is signed.
//!                    The session key's fees come from the operator's session allowance (the operator pays).
//!                    --signer env: explicit fallback, no phone: MAIN_MNEMONIC from the operator's own shell
//!                                (never read from the repo .env) sends MsgExec{MsgExecuteContract}, fee from
//!                                the granter's allowance.
//!   revoke-operator  granter revokes --operator: MsgRevoke(MsgExecuteContract) + MsgRevokeAllowance.
//!   grants           read-only: print the granter -> --operator authz + feegrant state.
//!
//! Dry-run by default; --broadcast also needs TERP_SCRIPT_BROADCAST=1 (and
//! TERP_SCRIPT_BROADCAST_CHAIN=<id> off morocco-1) and a matching --chain-id.
use anyhow::{Result, anyhow, bail};
use clap::Parser;
use cw_orch::daemon::TxSender;
use cw_orch::environment::ChainKind;
use cw_orch::prelude::*;
use cw_orch_kms::{
    ApprovalSession, GrantScope, GranteeState, ScopedCall, enable_session, grant_to,
    planned_spend_limit,
};
use dao_scripts::kms_gate;
use serde_json::{Value, json};

const EXEC_URL: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";

#[derive(Parser, Debug)]
#[command(
    name = "svg_ops",
    about = "svg factory / series day-2 ops through cw-orch-kms (exec | grant-operator | operator-exec | revoke-operator | grants)"
)]
struct Cli {
    /// exec | grant-operator | operator-exec | revoke-operator | grants
    cmd: String,
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long)]
    grpc: Option<String>,
    /// CometBFT RPC fallback for broadcast (comma-separated). Default: env MOROCCO_RPC on morocco-1.
    #[arg(long)]
    rpc: Option<String>,
    /// LCD/REST fallback for broadcast, after --rpc. Default: env MOROCCO_LCD on morocco-1.
    #[arg(long)]
    lcd: Option<String>,
    #[arg(long, default_value = "morocco-1")]
    network: String,
    #[arg(long, default_value = "uthiol")]
    gas_denom: String,
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
    /// Cold key (factory owner). exec/grant/revoke: empty = whichever key signs the granter challenge.
    /// operator-exec/grants: required.
    #[arg(long, env = cw_orch_kms::CW_ORCH_GRANTER_ENV, default_value = "")]
    granter: String,
    /// Zigner account index of the key that signs (granter, or the operator for operator-exec).
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    /// Contract to execute / scope the operator grant to.
    #[arg(long, default_value = "")]
    contract: String,
    /// Execute msg JSON (exec, operator-exec).
    #[arg(long)]
    msg: Option<String>,
    /// Read an execute msg JSON from a file. operator-exec: repeat to send several in one session.
    #[arg(long)]
    msg_file: Vec<std::path::PathBuf>,
    /// grant-operator: allowed top-level execute keys (comma separated).
    #[arg(long, default_value = "create_series_collection,create_svg_collection")]
    keys: String,
    /// grant-operator: number of operator calls the grant allows.
    #[arg(long, default_value_t = 10)]
    calls: u64,
    /// Session/grant lifetime. For grant-operator this is how long the operator grant lives.
    #[arg(long, default_value_t = 3600)]
    ttl_secs: u64,
    /// Operator wallet address (grant-operator, operator-exec, revoke-operator, grants).
    #[arg(long, alias = "grantee", default_value = "")]
    operator: String,
    /// operator-exec signer: `kms` (default; the operator's cold key signs one session grant on --bind)
    /// or `env` (explicit fallback; MAIN_MNEMONIC from the operator's own shell, never the repo .env).
    #[arg(long, default_value = "kms")]
    signer: String,
    /// Loopback approval page. `auto` (default) picks a free random 127.0.0.1 port, retrying if busy.
    #[arg(long, default_value = "auto")]
    bind: String,
    #[arg(long)]
    broadcast: bool,
    /// instantiate: existing code id (no upload).
    #[arg(long, default_value_t = 0)]
    code_id: u64,
    /// instantiate: label.
    #[arg(long, default_value = "")]
    label: String,
    /// instantiate: wasm admin of the new contract (empty = none).
    #[arg(long, default_value = "")]
    admin: String,
    /// Dry-run only: simulate the planned msgs as signed by --granter (no key, no session;
    /// the tx carries no pubkey/signature, which Simulate accepts). Real gas adds the
    /// MsgExec wrapper (~+40k).
    #[arg(long)]
    simulate: bool,
}

fn load_msgs(cli: &Cli) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    if let Some(m) = &cli.msg {
        out.push(serde_json::from_str(m)?);
    }
    for f in &cli.msg_file {
        out.push(serde_json::from_str(&std::fs::read_to_string(f)?)?);
    }
    Ok(out)
}

/// Longest operator session: the nested MsgExec grant lets the session key use every
/// grant the operator holds, so it lives minutes, not hours.
const OPERATOR_SESSION_MAX_SECS: u64 = 900;

fn top_key(msg: &Value) -> Result<String> {
    match msg.as_object() {
        Some(o) if o.len() == 1 => Ok(o.keys().next().cloned().unwrap()),
        _ => bail!("--msg must be a JSON object with one key"),
    }
}

/// What the operator's grant allows right now.
#[derive(Debug, PartialEq)]
struct GrantCheck {
    calls_left: u64,
    keys: Vec<String>,
    authz_expires: Option<u64>,
    fee_expires: Option<u64>,
    fee_limit: Option<u128>,
}

/// Refuse before signing when the grant cannot carry `key` on `contract`.
/// x/wasm deletes a MaxCalls grant when it reaches 0, and grant-operator adds one
/// spare call, so the usable calls are the on-chain count minus one.
fn check_grant(state: &GranteeState, contract: &str, key: &str, now: u64) -> Result<GrantCheck> {
    let authz = state.authz.iter().find(|a| a.type_url == EXEC_URL)
        .ok_or_else(|| anyhow!("refused: no MsgExecuteContract grant from the granter to {} (missing, revoked or used up)", state.grantee))?;
    if let Some(exp) = authz.expiration_unix
        && exp <= now
    {
        bail!("refused: the operator grant expired at unix {exp}");
    }
    let contracts = match &authz.scope {
        GrantScope::Contracts(c) => c,
        _ => bail!(
            "refused: the operator grant is not a contract-scoped ContractExecutionAuthorization"
        ),
    };
    let c = contracts
        .iter()
        .find(|c| c.address == contract)
        .ok_or_else(|| anyhow!("refused: the operator grant does not cover contract {contract}"))?;
    if !c.keys.is_empty() && !c.keys.iter().any(|k| k == key) {
        bail!(
            "refused: message `{key}` is not allowed by the operator grant (allowed: {})",
            c.keys.join(",")
        );
    }
    let calls_left = c.calls.saturating_sub(1);
    if calls_left == 0 {
        bail!("refused: the operator grant has no calls left");
    }
    let fee = state.feegrant.as_ref().ok_or_else(|| {
        anyhow!(
            "refused: no fee allowance from the granter to {}",
            state.grantee
        )
    })?;
    if let Some(exp) = fee.expiration_unix
        && exp <= now
    {
        bail!("refused: the fee allowance expired at unix {exp}");
    }
    Ok(GrantCheck {
        calls_left,
        keys: c.keys.clone(),
        authz_expires: authz.expiration_unix,
        fee_expires: fee.expiration_unix,
        fee_limit: fee.limit,
    })
}

fn print_state(state: &GranteeState, granter: &str) {
    println!(
        "grants {granter} -> {}: {} authz, feegrant {}",
        state.grantee,
        state.authz.len(),
        if state.feegrant.is_some() {
            "yes"
        } else {
            "none"
        }
    );
    for a in &state.authz {
        let scope = match &a.scope {
            GrantScope::Contracts(cs) => cs
                .iter()
                .map(|c| {
                    format!(
                        "{} keys=[{}] calls_on_chain={}",
                        c.address,
                        c.keys.join(","),
                        c.calls
                    )
                })
                .collect::<Vec<_>>()
                .join("; "),
            GrantScope::Generic => "generic".into(),
            GrantScope::Codes(c) => format!("codes {}", c.join(",")),
        };
        println!(
            "  authz {} expires {:?}: {scope}",
            a.type_url, a.expiration_unix
        );
    }
    if let Some(f) = &state.feegrant {
        println!(
            "  feegrant expires {:?} limit {:?}{}",
            f.expiration_unix, f.limit, f.denom
        );
    }
}

/// Resolve `--bind`: `auto` probes random loopback ports (20000..60000) until one is free;
/// an explicit addr must be loopback and falls back to a random port if busy.
fn resolve_bind(bind: &str) -> Result<std::net::SocketAddr> {
    use rand08::Rng;
    let free = |a: std::net::SocketAddr| std::net::TcpListener::bind(a).is_ok();
    if bind != "auto" {
        let a: std::net::SocketAddr = bind.parse()?;
        if !a.ip().is_loopback() {
            bail!("--bind {a} is not loopback");
        }
        if a.port() != 0 && free(a) {
            return Ok(a);
        }
        eprintln!("--bind {a} busy; picking a random loopback port");
    }
    let mut rng = rand08::thread_rng();
    for _ in 0..50 {
        let a: std::net::SocketAddr = ([127, 0, 0, 1], rng.gen_range(20000u16..60000)).into();
        if free(a) {
            return Ok(a);
        }
    }
    bail!("no free loopback port after 50 tries")
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    kms_gate::load_dotenv_public();
    let mut cli = Cli::parse();
    let info = match &cli.grpc {
        Some(g) => kms_gate::chain_info(
            &cli.network,
            g,
            &cli.gas_denom,
            cli.gas_price,
            "terp",
            ChainKind::Mainnet,
        ),
        None => kms_gate::morocco_info(),
    };
    let info = kms_gate::with_http_fallback(info, cli.rpc.as_deref(), cli.lcd.as_deref());
    println!(
        "broadcast fallback: rpc {:?} lcd {:?}",
        info.rpc_urls, info.lcd_url
    );
    let msgs = load_msgs(&cli)?;
    let msg = msgs.first().cloned();
    let plan = json!({"cmd": cli.cmd, "chain_id": info.chain_id, "granter": cli.granter, "contract": cli.contract,
        "msg_key": msg.as_ref().and_then(|m| top_key(m).ok()), "keys": cli.keys, "calls": cli.calls, "ttl_secs": cli.ttl_secs,
        "operator": cli.operator, "signer": cli.signer, "msgs": msgs.len()});
    println!("plan {}", serde_json::to_string_pretty(&plan)?);
    match cli.cmd.as_str() {
        "operator-exec" => return operator_exec(cli, info, msgs),
        "grants" => return grants(cli, info),
        "exec" | "grant-operator" | "revoke-operator" | "instantiate" => {}
        other => bail!("unknown cmd {other}"),
    }
    if !cli.broadcast {
        if cli.simulate {
            cw_orch_kms::validate_address(&cli.granter, "terp")
                .map_err(|e| anyhow!("--simulate needs --granter: {e}"))?;
            let anys = match cli.cmd.as_str() {
                // simulate: every --msg-file in one tx (e.g. set_series + a dependent create)
                "exec" => msgs
                    .iter()
                    .map(|m| kms_gate::execute_any(&cli.granter, &cli.contract, m))
                    .collect::<Result<Vec<_>>>()?,
                "instantiate" => vec![instantiate_any(
                    &cli,
                    msg.as_ref()
                        .ok_or_else(|| anyhow!("instantiate needs --msg"))?,
                )?],
                other => bail!("--simulate supports exec | instantiate, not {other}"),
            };
            let gas = simulate_as(&info, &cli.granter, anys)?;
            println!(
                "simulate ok: gas_used {gas} (as {}; nothing signed)",
                cli.granter
            );
        }
        println!("dry-run: nothing signed");
        return Ok(());
    }
    broadcast_gate(&cli, &info)?;
    cw_orch_kms::validate_ttl(cli.ttl_secs).map_err(|e| anyhow!(e))?;
    cw_orch_kms::set_approval_chain(kms_gate::kms_profile(&info)).map_err(|e| anyhow!(e))?;
    if cli.granter.is_empty() {
        println!(
            "sign the granter challenge at http://{}",
            kms_gate::granter_page()
        );
        cli.granter = cw_orch_kms::capture_granter_sync(
            kms_gate::granter_page(),
            cli.account_index,
            "svg-ops",
        )
        .map_err(|e| anyhow!(e))?;
    }
    cw_orch_kms::validate_address(&cli.granter, "terp").map_err(|e| anyhow!("granter: {e}"))?;
    // The session key is in-memory; it only broadcasts.
    cw_orch_kms::ensure_ephemeral_mnemonic(info.kind.clone()).map_err(|e| anyhow!(e))?;
    let mut b = Daemon::builder(info.clone());
    b.load_network(false);
    if let Some(g) = &cli.grpc {
        b.grpc_url(g);
    }
    let granter = cli.granter.clone();
    let mut session = ApprovalSession::open(&granter);
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.bind = resolve_bind(&cli.bind)?;
    println!("approval page will bind http://{}", session.bind);
    session.allowance_limit = planned_spend_limit(4 + msgs.len() as u64);
    let reqs = vec![
        cw_orch_kms::SigningRequest::new("session-key")
            .body(format!("svg_ops {}: {}", cli.cmd, plan)),
    ];
    let anys: Vec<cosmrs::Any> = match cli.cmd.as_str() {
        "exec" => {
            // every --msg/--msg-file goes into ONE tx; the grant covers exactly those calls (+1: x/wasm deletes at 0)
            if msgs.is_empty() {
                bail!("exec needs --msg or --msg-file");
            }
            if cli.contract.is_empty() {
                bail!("exec needs --contract");
            }
            let mut keys: Vec<String> = msgs.iter().map(top_key).collect::<Result<_>>()?;
            keys.sort();
            keys.dedup();
            session = session
                .allow_execute(ScopedCall {
                    address: cli.contract.clone(),
                    keys,
                    calls: msgs.len() as u64 + 1,
                    funds: vec![],
                })
                .map_err(|e| anyhow!(e))?;
            msgs.iter()
                .map(|m| kms_gate::execute_any(&granter, &cli.contract, m))
                .collect::<Result<Vec<_>>>()?
        }
        "instantiate" => {
            let m = msg
                .clone()
                .ok_or_else(|| anyhow!("instantiate needs --msg"))?;
            session = session.allow_instantiate();
            vec![instantiate_any(&cli, &m)?]
        }
        "grant-operator" => {
            if cli.contract.is_empty() {
                bail!("grant-operator needs --contract");
            }
            cw_orch_kms::validate_address(&cli.operator, "terp")
                .map_err(|e| anyhow!("--operator: {e}"))?;
            if msg.is_some() {
                bail!(
                    "grant-operator executes nothing; run operator-exec with the operator's signer"
                );
            }
            let keys: Vec<String> = cli
                .keys
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            // +1: x/wasm deletes a MaxCallsLimit grant at 0.
            session = session
                .allow_execute(ScopedCall {
                    address: cli.contract.clone(),
                    keys,
                    calls: cli.calls + 1,
                    funds: vec![],
                })
                .map_err(|e| anyhow!(e))?;
            session.retain = true;
            session.operator_grant = true;
            session.allowance_limit = planned_spend_limit(cli.calls + 1);
            grant_to(&b, session, &cli.operator, reqs).map_err(|e| anyhow!(e))?;
            println!(
                "operator {} holds a ContractExecutionAuthorization on {} ({} calls, ttl {}s) + fee allowance from {}",
                cli.operator, cli.contract, cli.calls, cli.ttl_secs, granter
            );
            return Ok(());
        }
        "revoke-operator" => {
            cw_orch_kms::validate_address(&cli.operator, "terp")
                .map_err(|e| anyhow!("--operator: {e}"))?;
            session = session
                .allow_sdk("/cosmos.authz.v1beta1.MsgRevoke")
                .map_err(|e| anyhow!(e))?
                .allow_sdk("/cosmos.feegrant.v1beta1.MsgRevokeAllowance")
                .map_err(|e| anyhow!(e))?;
            use cosmos_sdk_proto::traits::Message;
            let r = cosmos_sdk_proto::cosmos::authz::v1beta1::MsgRevoke {
                granter: granter.clone(),
                grantee: cli.operator.clone(),
                msg_type_url: EXEC_URL.into(),
            };
            let f = cosmos_sdk_proto::cosmos::feegrant::v1beta1::MsgRevokeAllowance {
                granter: granter.clone(),
                grantee: cli.operator.clone(),
            };
            vec![
                cosmrs::Any {
                    type_url: "/cosmos.authz.v1beta1.MsgRevoke".into(),
                    value: r.encode_to_vec(),
                },
                cosmrs::Any {
                    type_url: "/cosmos.feegrant.v1beta1.MsgRevokeAllowance".into(),
                    value: f.encode_to_vec(),
                },
            ]
        }
        _ => unreachable!(),
    };
    let run = enable_session(&b, session, reqs).map_err(|e| anyhow!(e))?;
    println!(
        "session key {} (granter {})",
        run.daemon.sender_addr(),
        granter
    );
    let d = &run.daemon;
    let res = d
        .rt_handle
        .block_on(d.sender().commit_tx_any(anys, Some("svg_ops")))
        .map(|r| {
            println!("tx {} code {} gas {}", r.txhash, r.code, r.gas_used);
            for a in r.event_attr_values("wasm", "contract") {
                println!("created {a}");
            }
        })
        .map_err(|e| anyhow!("{e}"));
    let closed = run.close().map_err(|e| anyhow!(e));
    res?;
    closed?;
    Ok(())
}

fn broadcast_gate(cli: &Cli, info: &cw_orch::environment::ChainInfoOwned) -> Result<()> {
    if cli.chain_id != info.chain_id {
        bail!(
            "broadcast refused: --chain-id {:?} != {}",
            cli.chain_id,
            info.chain_id
        );
    }
    if std::env::var(kms_gate::BROADCAST_ENV).ok().as_deref() != Some("1") {
        bail!("broadcast refused: set {}=1", kms_gate::BROADCAST_ENV);
    }
    if info.chain_id != "morocco-1"
        && std::env::var(kms_gate::BROADCAST_CHAIN_ENV).ok().as_deref()
            != Some(info.chain_id.as_str())
    {
        bail!(
            "broadcast refused: also set {}={}",
            kms_gate::BROADCAST_CHAIN_ENV,
            info.chain_id
        );
    }
    Ok(())
}

/// A daemon for queries/broadcast. kms mode: an in-memory key that never signs.
fn query_daemon(cli: &Cli, info: &cw_orch::environment::ChainInfoOwned) -> Result<Daemon> {
    cw_orch_kms::ensure_ephemeral_mnemonic(info.kind.clone()).map_err(|e| anyhow!(e))?;
    let mut b = Daemon::builder(info.clone());
    b.load_network(false);
    if let Some(g) = &cli.grpc {
        b.grpc_url(g);
    }
    b.build().map_err(|e| anyhow!("{e}"))
}

fn grants(cli: Cli, info: cw_orch::environment::ChainInfoOwned) -> Result<()> {
    cw_orch_kms::validate_address(&cli.granter, "terp").map_err(|e| anyhow!("--granter: {e}"))?;
    cw_orch_kms::validate_address(&cli.operator, "terp").map_err(|e| anyhow!("--operator: {e}"))?;
    if !cli.chain_id.is_empty() && cli.chain_id != info.chain_id {
        bail!(
            "refused: --chain-id {:?} != {}",
            cli.chain_id,
            info.chain_id
        );
    }
    let d = query_daemon(&cli, &info)?;
    let state = cw_orch_kms::query_grantee_state(&d, &cli.granter, &cli.operator)
        .map_err(|e| anyhow!(e))?;
    print_state(&state, &cli.granter);
    Ok(())
}

fn operator_exec(
    mut cli: Cli,
    info: cw_orch::environment::ChainInfoOwned,
    msgs: Vec<Value>,
) -> Result<()> {
    if msgs.is_empty() {
        bail!("operator-exec needs --msg or --msg-file");
    }
    let keys = msgs.iter().map(top_key).collect::<Result<Vec<_>>>()?;
    if cli.contract.is_empty() {
        bail!("operator-exec needs --contract");
    }
    cw_orch_kms::validate_address(&cli.contract, "terp").map_err(|e| anyhow!("--contract: {e}"))?;
    cw_orch_kms::validate_address(&cli.granter, "terp")
        .map_err(|e| anyhow!("--granter (the grant's owner) is required: {e}"))?;
    let env_signer = match cli.signer.as_str() {
        "kms" => false,
        "env" => true,
        s => bail!("--signer {s}: use kms (default) or env"),
    };
    if env_signer {
        if std::env::var("MAIN_MNEMONIC")
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
        {
            bail!(
                "--signer env needs MAIN_MNEMONIC exported in the operator's own shell (the repo .env is never read for it)"
            );
        }
    } else {
        // The operator address is needed to check its grant before the phone signs anything.
        cw_orch_kms::validate_address(&cli.operator, "terp")
            .map_err(|e| anyhow!("--operator is required with --signer kms: {e}"))?;
    }
    if cli.broadcast {
        broadcast_gate(&cli, &info)?;
    } else if !cli.chain_id.is_empty() && cli.chain_id != info.chain_id {
        bail!(
            "refused: --chain-id {:?} != {}",
            cli.chain_id,
            info.chain_id
        );
    }
    cw_orch_kms::set_approval_chain(kms_gate::kms_profile(&info)).map_err(|e| anyhow!(e))?;
    let mut d = query_daemon(&cli, &info)?;
    if env_signer {
        let addr = d.sender_addr().to_string();
        if !cli.operator.is_empty() && cli.operator != addr {
            bail!(
                "refused: MAIN_MNEMONIC is {addr}, not --operator {}",
                cli.operator
            );
        }
        cli.operator = addr;
    }
    if cli.operator == cli.granter {
        bail!("refused: the operator is the granter; use `svg_ops exec`");
    }
    let state = cw_orch_kms::query_grantee_state(&d, &cli.granter, &cli.operator)
        .map_err(|e| anyhow!(e))?;
    print_state(&state, &cli.granter);
    let mut left = u64::MAX;
    for key in &keys {
        let g = check_grant(&state, &cli.contract, key, now_unix())?;
        left = g.calls_left;
        println!(
            "grant ok: `{key}` allowed (keys [{}]), {} call(s) left, authz expires {:?}, fee allowance expires {:?} limit {:?}",
            g.keys.join(","),
            g.calls_left,
            g.authz_expires,
            g.fee_expires,
            g.fee_limit
        );
    }
    if (msgs.len() as u64) > left {
        bail!(
            "refused: {} message(s) but the operator grant has {left} call(s) left",
            msgs.len()
        );
    }
    if !cli.broadcast {
        println!("dry-run: nothing signed");
        return Ok(());
    }
    if env_signer {
        let granter = Addr::unchecked(&cli.granter);
        d.sender_mut().set_authz_granter(&granter);
        d.sender_mut().set_fee_granter(&granter);
        for m in &msgs {
            let any = kms_gate::execute_any(&cli.granter, &cli.contract, m)?;
            let r = d
                .rt_handle
                .block_on(
                    d.sender()
                        .commit_tx_any(vec![any], Some("svg_ops operator-exec")),
                )
                .map_err(|e| anyhow!("{e}"))?;
            report_tx(&r);
        }
    } else {
        // The operator's cold key signs only this session grant; the session key relays the operator's grant.
        let ttl = cli.ttl_secs.min(OPERATOR_SESSION_MAX_SECS);
        cw_orch_kms::validate_ttl(ttl).map_err(|e| anyhow!(e))?;
        let mut session = ApprovalSession::open(&cli.operator).allow_nested_exec();
        session.ttl_secs = ttl;
        session.account_index = cli.account_index;
        session.bind = resolve_bind(&cli.bind)?;
        session.allowance_limit = planned_spend_limit(msgs.len() as u64 + 1);
        let reqs = vec![cw_orch_kms::SigningRequest::new("operator-session").body(format!(
            "operator {} opens a {ttl}s session key to relay its grant from {}: {} message(s) [{}] on {}. The session key may use every grant this operator holds until close.",
            cli.operator, cli.granter, msgs.len(), keys.join(","), cli.contract))];
        cw_orch_kms::ensure_ephemeral_mnemonic(info.kind.clone()).map_err(|e| anyhow!(e))?;
        let mut b = Daemon::builder(info.clone());
        b.load_network(false);
        if let Some(g) = &cli.grpc {
            b.grpc_url(g);
        }
        // Share the query daemon's state; a second daemon would hit the state-file lock.
        b.state(d.state());
        let run = enable_session(&b, session, reqs).map_err(|e| anyhow!(e))?;
        println!(
            "session key {} (operator {}, granter {})",
            run.daemon.sender_addr(),
            cli.operator,
            cli.granter
        );
        let sd = &run.daemon;
        let mut res = Ok(());
        for m in &msgs {
            let inner = match cw_orch_kms::exec_contract_as(
                &cli.operator,
                &cli.granter,
                &cli.contract,
                m,
                &[],
            ) {
                Ok(x) => x.any,
                Err(e) => {
                    res = Err(anyhow!(e));
                    break;
                }
            };
            // The session daemon wraps this in MsgExec{grantee: session key}.
            match sd.rt_handle.block_on(
                sd.sender()
                    .commit_tx_any(vec![inner], Some("svg_ops operator-exec")),
            ) {
                Ok(r) => report_tx(&r),
                Err(e) => {
                    res = Err(anyhow!("{e}"));
                    break;
                }
            }
        }
        let closed = run.close().map_err(|e| anyhow!(e));
        res?;
        closed?;
        println!("session closed: session grants revoked");
    }
    let after = cw_orch_kms::query_grantee_state(&d, &cli.granter, &cli.operator)
        .map_err(|e| anyhow!(e))?;
    match check_grant(&after, &cli.contract, &keys[0], now_unix()) {
        Ok(g) => println!("after: {} call(s) left", g.calls_left),
        Err(e) => println!("after: {e}"),
    }
    Ok(())
}

fn report_tx(r: &cw_orch::daemon::CosmTxResponse) {
    println!("tx {} code {} gas {}", r.txhash, r.code, r.gas_used);
    if let Ok(a) = r.event_attr_value("wasm", "contract") {
        println!("created {a}");
    }
}

/// `MsgInstantiateContract` of an existing code from the granter (under MsgExec).
fn instantiate_any(cli: &Cli, msg: &Value) -> Result<cosmrs::Any> {
    use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgInstantiateContract;
    if cli.code_id == 0 || cli.label.is_empty() {
        bail!("instantiate needs --code-id and --label");
    }
    if !cli.admin.is_empty() {
        cw_orch_kms::validate_address(&cli.admin, "terp").map_err(|e| anyhow!("--admin: {e}"))?;
    }
    Ok(kms_gate::any(
        "/cosmwasm.wasm.v1.MsgInstantiateContract",
        &MsgInstantiateContract {
            sender: cli.granter.clone(),
            admin: cli.admin.clone(),
            code_id: cli.code_id,
            label: cli.label.clone(),
            msg: serde_json::to_vec(msg)?,
            funds: vec![],
        },
    ))
}

/// Gas for `anys` signed by `signer`, via gRPC Simulate. No key: the signer info has no
/// pubkey and the signature is empty (the ante handler skips verification in simulate).
fn simulate_as(
    info: &cw_orch::environment::ChainInfoOwned,
    signer: &str,
    anys: Vec<cosmrs::Any>,
) -> Result<u64> {
    use cosmos_sdk_proto::cosmos::auth::v1beta1::{
        QueryAccountInfoRequest, query_client::QueryClient as AuthQ,
    };
    use cosmos_sdk_proto::cosmos::tx::v1beta1::{
        AuthInfo, Fee, ModeInfo, SignerInfo, SimulateRequest, TxBody, TxRaw, mode_info,
        service_client::ServiceClient,
    };
    use cosmos_sdk_proto::prost::Message;
    let d = kms_gate::query_only(info)?;
    let ch = d.channel();
    d.rt_handle.block_on(async move {
        let seq = AuthQ::new(ch.clone())
            .account_info(QueryAccountInfoRequest {
                address: signer.into(),
            })
            .await
            .map_err(|e| anyhow!("account {signer}: {}", e.message()))?
            .into_inner()
            .info
            .ok_or_else(|| anyhow!("no account"))?
            .sequence;
        let body = TxBody {
            messages: anys
                .into_iter()
                .map(|a| cosmos_sdk_proto::Any {
                    type_url: a.type_url,
                    value: a.value,
                })
                .collect(),
            memo: "svg_ops simulate".into(),
            ..Default::default()
        };
        let auth = AuthInfo {
            signer_infos: vec![SignerInfo {
                public_key: None,
                mode_info: Some(ModeInfo {
                    sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
                }),
                sequence: seq,
            }],
            fee: Some(Fee {
                amount: vec![],
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
        #[allow(deprecated)]
        let r = ServiceClient::new(ch)
            .simulate(SimulateRequest {
                tx: None,
                tx_bytes: raw.encode_to_vec(),
            })
            .await
            .map_err(|e| anyhow!("simulate: {}", e.message()))?
            .into_inner();
        Ok(r.gas_info.map(|g| g.gas_used).unwrap_or(0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cw_orch_kms::{LiveAuthz, LiveContract, LiveFeegrant};
    const C: &str = "terp1contract";
    fn state(calls: u64, exp: u64, fee: bool) -> GranteeState {
        GranteeState {
            grantee: "terp1op".into(),
            authz: vec![LiveAuthz {
                type_url: EXEC_URL.into(),
                expiration_unix: Some(exp),
                scope: GrantScope::Contracts(vec![LiveContract {
                    address: C.into(),
                    keys: vec!["create_svg_collection".into()],
                    calls,
                }]),
            }],
            feegrant: fee.then(|| LiveFeegrant {
                expiration_unix: Some(exp),
                denom: "uthiol".into(),
                limit: Some(10),
            }),
        }
    }
    #[test]
    fn grant_checks() {
        let ok = check_grant(&state(3, 200, true), C, "create_svg_collection", 100).unwrap();
        assert_eq!(ok.calls_left, 2);
        let err =
            |s: GranteeState, c: &str, k: &str| check_grant(&s, c, k, 100).unwrap_err().to_string();
        assert!(err(state(3, 200, true), C, "set_series").contains("not allowed"));
        assert!(err(state(1, 200, true), C, "create_svg_collection").contains("no calls left"));
        assert!(err(state(3, 50, true), C, "create_svg_collection").contains("expired"));
        assert!(err(state(3, 200, false), C, "create_svg_collection").contains("fee allowance"));
        assert!(
            err(state(3, 200, true), "terp1other", "create_svg_collection")
                .contains("does not cover")
        );
        let empty = GranteeState {
            grantee: "terp1op".into(),
            authz: vec![],
            feegrant: None,
        };
        assert!(err(empty, C, "create_svg_collection").contains("no MsgExecuteContract grant"));
    }
    #[test]
    fn top_key_needs_one_key() {
        assert!(top_key(&json!({"a":{},"b":{}})).is_err());
        assert_eq!(top_key(&json!({"a":{}})).unwrap(), "a");
    }
}
