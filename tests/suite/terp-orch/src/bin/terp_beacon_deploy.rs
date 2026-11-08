//! terp-beacon rollout: store, instantiate the beacon, add attestors and config,
//! and point an nftfi raffle at it. Signed only through cw-orch-kms.
//!
//! DRY RUN BY DEFAULT. Every step builds the exact messages offline, prints them
//! with the signer (the cw-orch-kms granter), and writes
//! `<out>/cw-orch-preview/<step>.json`. No network, no session, no signature.
//!
//! `--broadcast` plus `TERP_BEACON_BROADCAST=1` (and `--grpc-url`) opens a
//! cw-orch-kms approval session scoped to the step's messages, simulates, then
//! broadcasts as MsgExec under that grant and closes (revokes) the session.
//!
//! Steps:
//!   store               MsgStoreCode for terp_beacon.wasm (or nftfi_raffle.wasm).
//!   instantiate-beacon  MsgInstantiateContract, drand network preset (quicknet/default).
//!   configure-beacon    add_attestor per --attestor, then update_config.
//!   instantiate-raffle  MsgInstantiateContract for nftfi-raffle with Callback{verifier: beacon}.
//!   wire-raffle         raffle update_config randomness = Callback{verifier: beacon}.
//!
//! Build: cargo build -p terp-orch --features gov --bin terp_beacon_deploy
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::cosmwasm::wasm::v1::{
    MsgExecuteContract, MsgInstantiateContract, MsgStoreCode,
};
use cosmos_sdk_proto::prost::Message;
use cosmwasm_std::{Coin, Decimal, HexBinary, Uint256};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::str::FromStr;
use terp_beacon::networks::{DEFAULT, DrandNetwork, QUICKNET};

const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const T_STORE: &str = "/cosmwasm.wasm.v1.MsgStoreCode";
const T_INST: &str = "/cosmwasm.wasm.v1.MsgInstantiateContract";
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
const GATE: &str = "TERP_BEACON_BROADCAST";

#[derive(Parser)]
#[command(about = "terp-beacon deploy + raffle wiring via cw-orch-kms (dry run by default)")]
struct Cli {
    /// Sign and broadcast through cw-orch-kms. Also needs TERP_BEACON_BROADCAST=1.
    #[arg(long)]
    broadcast: bool,
    /// Signer (cw-orch-kms granter). Every message's sender.
    #[arg(long)]
    granter: String,
    #[arg(long, default_value = "morocco-1")]
    chain_id: String,
    /// gRPC endpoint for --broadcast (required then; no default). Dry runs never connect.
    #[arg(long, env = "GRPC_URL")]
    grpc_url: Option<String>,
    #[arg(long, env = "TERP_LCD")]
    lcd: Option<String>,
    #[arg(long, default_value = "terp-beacon-out")]
    out: PathBuf,
    #[arg(long, default_value_t = 0u32)]
    account_index: u32,
    #[arg(long, default_value_t = 30 * 60)]
    ttl_secs: u64,
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
    #[command(subcommand)]
    step: Step,
}

#[derive(Clone, Copy, ValueEnum)]
enum Network {
    Quicknet,
    Default,
}
impl Network {
    fn preset(self) -> DrandNetwork {
        match self {
            Network::Quicknet => QUICKNET,
            Network::Default => DEFAULT,
        }
    }
}

#[derive(Subcommand)]
enum Step {
    /// 00: MsgStoreCode (terp_beacon.wasm, or nftfi_raffle.wasm when the raffle is not on chain).
    Store {
        #[arg(long)]
        wasm: PathBuf,
        /// Expected sha256 of the wasm (hex). Refuses on mismatch when set.
        #[arg(long)]
        sha256: Option<String>,
    },
    /// 01: instantiate terp-beacon with a drand network preset.
    InstantiateBeacon {
        #[arg(long)]
        code_id: u64,
        /// Beacon admin and wasm migrate admin. Defaults to --granter.
        #[arg(long)]
        admin: Option<String>,
        #[arg(long, value_enum, default_value = "quicknet")]
        network: Network,
        #[arg(long, default_value_t = 0u32)]
        required_attestations: u32,
        /// Request price, `<amount><denom>` (repeatable). Omit for free requests.
        #[arg(long)]
        price: Vec<String>,
        #[arg(long, default_value = "terp-beacon")]
        label: String,
    },
    /// 02: add_attestor per --attestor, then update_config when any config flag is set.
    ConfigureBeacon {
        #[arg(long)]
        beacon: String,
        /// Compressed secp256k1 attestor pubkey, 33-byte hex (repeatable).
        #[arg(long)]
        attestor: Vec<String>,
        #[arg(long)]
        required_attestations: Option<u32>,
        /// Replace the price list, `<amount><denom>` (repeatable).
        #[arg(long)]
        price: Vec<String>,
        /// Make requests free (empty price list).
        #[arg(long, conflicts_with = "price")]
        free: bool,
        #[arg(long)]
        callback_gas_limit: Option<u64>,
    },
    /// 03 (optional): instantiate nftfi-raffle already pointed at the beacon.
    InstantiateRaffle {
        #[arg(long)]
        code_id: u64,
        #[arg(long)]
        beacon: String,
        #[arg(long)]
        name: String,
        /// Raffle owner and wasm admin. Defaults to --granter.
        #[arg(long)]
        admin: Option<String>,
        #[arg(long)]
        fee_addr: String,
        #[arg(long, default_value = "0.1")]
        raffle_fee: String,
        #[arg(long, default_value = "nftfi-raffle")]
        label: String,
    },
    /// 04: raffle update_config randomness = Callback { verifier: beacon }. Signer = raffle owner.
    WireRaffle {
        #[arg(long)]
        raffle: String,
        #[arg(long)]
        beacon: String,
    },
}

enum Scope {
    Store(Vec<u8>),
    Instantiate,
    Execute {
        contract: String,
        keys: Vec<String>,
        calls: u64,
    },
}

struct Plan {
    step: String,
    note: String,
    session: Value,
    msgs: Vec<(Any, Value)>,
    scope: Scope,
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
    check_addr(&cli.granter, "--granter")?;
    let plan = plan(&cli)?;
    print_and_write(&cli, &plan)?;
    if !cli.broadcast {
        println!(
            "mode: DRY RUN (nothing signed). Broadcast needs --broadcast and {GATE}=1, GRPC_URL set."
        );
        return Ok(());
    }
    broadcast(&cli, plan)
}

fn plan(cli: &Cli) -> Result<Plan> {
    let sender = cli.granter.clone();
    Ok(match &cli.step {
        Step::Store { wasm, sha256 } => {
            let bytes = std::fs::read(wasm).with_context(|| format!("read {}", wasm.display()))?;
            if !bytes.starts_with(b"\0asm") {
                bail!("{} is not a wasm module", wasm.display());
            }
            let sum = hexenc(cw_orch_kms::wasm_checksum(&bytes).map_err(|e| anyhow!(e))?);
            if let Some(want) = sha256 {
                if !want.eq_ignore_ascii_case(&sum) {
                    bail!("wasm sha256 {sum} != --sha256 {want}");
                }
            }
            let name = wasm
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("wasm")
                .to_string();
            let msg = MsgStoreCode {
                sender: sender.clone(),
                wasm_byte_code: bytes.clone(),
                instantiate_permission: None,
            };
            let view = json!({"@type": T_STORE, "sender": sender,
                "wasm_byte_code": format!("<{} bytes, sha256 {sum}>", bytes.len()), "instantiate_permission": null});
            Plan {
                step: format!("00-store-{name}"),
                note: format!("upload {name} ({} bytes)", bytes.len()),
                session: json!({"stores": [sum], "type_urls": [T_STORE]}),
                msgs: vec![(any(T_STORE, &msg), view)],
                scope: Scope::Store(bytes),
            }
        }
        Step::InstantiateBeacon {
            code_id,
            admin,
            network,
            required_attestations,
            price,
            label,
        } => {
            let admin = admin.clone().unwrap_or_else(|| sender.clone());
            check_addr(&admin, "--admin")?;
            let prices = parse_coins(price)?;
            let net = network.preset();
            let init = net.instantiate_msg(&admin, prices.clone(), *required_attestations);
            let init_json = serde_json::to_value(&init)?;
            let msg = MsgInstantiateContract {
                sender: sender.clone(),
                admin: admin.clone(),
                code_id: *code_id,
                label: label.clone(),
                msg: serde_json::to_vec(&init)?,
                funds: vec![],
            };
            Plan {
                step: "01-instantiate-beacon".into(),
                note: format!(
                    "drand {} ({}), genesis {}, period {}s",
                    net.chain_hash_hex, net.info_url, net.genesis_time, net.period_seconds
                ),
                session: json!({"instantiate": true, "type_urls": [T_INST]}),
                msgs: vec![(
                    any(T_INST, &msg),
                    json!({"@type": T_INST, "sender": sender, "admin": admin,
                    "code_id": code_id, "label": label, "funds": [], "msg": init_json}),
                )],
                scope: Scope::Instantiate,
            }
        }
        Step::ConfigureBeacon {
            beacon,
            attestor,
            required_attestations,
            price,
            free,
            callback_gas_limit,
        } => {
            check_addr(beacon, "--beacon")?;
            let mut msgs = vec![];
            let mut keys = vec![];
            for pk in attestor {
                let pk = HexBinary::from_hex(pk).map_err(|e| anyhow!("--attestor {pk}: {e}"))?;
                if pk.len() != 33 || !matches!(pk[0], 2 | 3) {
                    bail!("--attestor must be a 33-byte compressed secp256k1 key");
                }
                msgs.push(exec(
                    &sender,
                    beacon,
                    &terp_beacon::msg::ExecuteMsg::AddAttestor { pubkey: pk },
                )?);
                if !keys.contains(&"add_attestor".to_string()) {
                    keys.push("add_attestor".to_string());
                }
            }
            let prices = if *free {
                Some(vec![])
            } else if price.is_empty() {
                None
            } else {
                Some(parse_coins(price)?)
            };
            if required_attestations.is_some() || prices.is_some() || callback_gas_limit.is_some() {
                msgs.push(exec(
                    &sender,
                    beacon,
                    &terp_beacon::msg::ExecuteMsg::UpdateConfig {
                        required_attestations: *required_attestations,
                        prices,
                        callback_gas_limit: *callback_gas_limit,
                    },
                )?);
                keys.push("update_config".to_string());
            }
            if msgs.is_empty() {
                bail!("nothing to do: pass --attestor and/or a config flag");
            }
            let calls = msgs.len() as u64;
            Plan {
                step: "02-configure-beacon".into(),
                note: format!(
                    "{} attestor(s), config update: {}",
                    attestor.len(),
                    keys.contains(&"update_config".to_string())
                ),
                session: json!({"execute": [{"contract": beacon, "keys": keys, "calls": calls}], "type_urls": [T_EXEC]}),
                msgs,
                scope: Scope::Execute {
                    contract: beacon.clone(),
                    keys,
                    calls,
                },
            }
        }
        Step::InstantiateRaffle {
            code_id,
            beacon,
            name,
            admin,
            fee_addr,
            raffle_fee,
            label,
        } => {
            check_addr(beacon, "--beacon")?;
            check_addr(fee_addr, "--fee-addr")?;
            let admin = admin.clone().unwrap_or_else(|| sender.clone());
            check_addr(&admin, "--admin")?;
            let init = nftfi_raffle::InstantiateMsg {
                name: name.clone(),
                owner: Some(admin.clone()),
                fee_addr: fee_addr.clone(),
                minimum_raffle_duration: None,
                max_tickets_per_raffle: None,
                raffle_fee: Decimal::from_str(raffle_fee)
                    .map_err(|e| anyhow!("--raffle-fee: {e}"))?,
                oracle_pricing: None,
                creation_coins: None,
                randomness: nftfi_randomness::RandomnessSourceMsg::Callback {
                    verifier: beacon.clone(),
                },
            };
            let msg = MsgInstantiateContract {
                sender: sender.clone(),
                admin: admin.clone(),
                code_id: *code_id,
                label: label.clone(),
                msg: serde_json::to_vec(&init)?,
                funds: vec![],
            };
            Plan {
                step: "03-instantiate-raffle".into(),
                note: format!("nftfi-raffle with randomness Callback{{verifier: {beacon}}}"),
                session: json!({"instantiate": true, "type_urls": [T_INST]}),
                msgs: vec![(
                    any(T_INST, &msg),
                    json!({"@type": T_INST, "sender": sender, "admin": admin,
                    "code_id": code_id, "label": label, "funds": [], "msg": serde_json::to_value(&init)?}),
                )],
                scope: Scope::Instantiate,
            }
        }
        Step::WireRaffle { raffle, beacon } => {
            check_addr(raffle, "--raffle")?;
            check_addr(beacon, "--beacon")?;
            let m = exec(
                &sender,
                raffle,
                &nftfi_raffle::msg::ExecuteMsg::UpdateConfig {
                    owner: None,
                    fee_addr: None,
                    minimum_raffle_duration: None,
                    max_tickets_per_raffle: None,
                    raffle_fee: None,
                    creation_coins: None,
                    randomness: Some(nftfi_randomness::RandomnessSourceMsg::Callback {
                        verifier: beacon.clone(),
                    }),
                },
            )?;
            Plan {
                step: "04-wire-raffle".into(),
                note:
                    "raffle owner points randomness at the beacon (RequestDraw -> beacon Provide)"
                        .into(),
                session: json!({"execute": [{"contract": raffle, "keys": ["update_config"], "calls": 1}], "type_urls": [T_EXEC]}),
                msgs: vec![m],
                scope: Scope::Execute {
                    contract: raffle.clone(),
                    keys: vec!["update_config".into()],
                    calls: 1,
                },
            }
        }
    })
}

fn exec<M: serde::Serialize>(sender: &str, contract: &str, msg: &M) -> Result<(Any, Value)> {
    let m = MsgExecuteContract {
        sender: sender.into(),
        contract: contract.into(),
        msg: serde_json::to_vec(msg)?,
        funds: vec![],
    };
    Ok((
        any(T_EXEC, &m),
        json!({"@type": T_EXEC, "sender": sender, "contract": contract, "funds": [], "msg": serde_json::to_value(msg)?}),
    ))
}

fn parse_coins(raw: &[String]) -> Result<Vec<Coin>> {
    raw.iter()
        .map(|s| {
            let i = s
                .find(|c: char| !c.is_ascii_digit())
                .ok_or_else(|| anyhow!("price {s}: want <amount><denom>"))?;
            if i == 0 {
                bail!("price {s}: want <amount><denom>");
            }
            Ok(Coin {
                amount: Uint256::from_str(&s[..i]).map_err(|e| anyhow!("price {s}: {e}"))?,
                denom: s[i..].to_string(),
            })
        })
        .collect()
}

fn check_addr(a: &str, flag: &str) -> Result<()> {
    if !a.starts_with("terp1") || a.len() < 39 || a.chars().any(|c| !c.is_ascii_alphanumeric()) {
        bail!("{flag} {a:?} is not a terp1... address");
    }
    Ok(())
}

fn print_and_write(cli: &Cli, plan: &Plan) -> Result<()> {
    let msgs: Vec<Value> = plan.msgs.iter().map(|(a, v)| {
        let shown = if a.type_url == T_STORE { "<omitted: contains the wasm>".to_string() } else { hexenc(&a.value) };
        json!({"type_url": a.type_url, "json": v, "proto_value_hex": shown, "proto_value_len": a.value.len()})
    }).collect();
    let out = json!({
        "_NOT_SIGNED": "cw-orch-kms preview; nothing was signed or broadcast",
        "chain_id": cli.chain_id,
        "step": plan.step,
        "signer_granter": cli.granter,
        "broadcaster": "ephemeral cw-orch-kms session key (generated at --broadcast time), wraps the messages in MsgExec(granter)",
        "note": plan.note,
        "kms_session_scope": plan.session,
        "messages": msgs,
    });
    let dir = cli.out.join("cw-orch-preview");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", plan.step));
    std::fs::write(&path, serde_json::to_string_pretty(&out)? + "\n")?;
    println!("== {} ({})", plan.step, plan.note);
    println!("signer (cw-orch-kms granter): {}", cli.granter);
    for (a, v) in &plan.msgs {
        println!(
            "message {}:\n{}",
            a.type_url,
            serde_json::to_string_pretty(v)?
        );
    }
    println!("wrote {}", path.display());
    Ok(())
}

fn broadcast(cli: &Cli, plan: Plan) -> Result<()> {
    use cw_orch::daemon::{DaemonBuilder, TxSender};
    use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
    use cw_orch_kms::progress::Status;
    if std::env::var(GATE).ok().as_deref() != Some("1") {
        bail!("refusing to sign: --broadcast also needs {GATE}=1");
    }
    let kind = if cli.chain_id.starts_with("morocco") {
        ChainKind::Mainnet
    } else {
        ChainKind::Testnet
    };
    let chain = ChainInfoOwned {
        chain_id: cli.chain_id.clone(),
        gas_denom: "uthiol".into(),
        gas_price: cli.gas_price,
        grpc_urls: vec![
            cli.grpc_url
                .clone()
                .ok_or_else(|| anyhow!("--broadcast needs --grpc-url or GRPC_URL"))?,
        ],
        rpc_urls: vec![],
        lcd_url: cli.lcd.clone(),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: "terp".into(),
            coin_type: 118,
        },
        kind,
    };
    // SAFETY: set before cw-orch reads it; single-threaded here.
    unsafe { std::env::set_var("CW_ORCH_WALLET_BALANCE_ASSERTION", "false") };
    let page = cw_orch_kms::page_addr();
    println!(
        "sign the cw-orch-granter challenge on the approval page (port {})",
        page.port()
    );
    let granter = cw_orch_kms::capture_granter_for_chain_opts_sync(
        page,
        cli.account_index,
        "terp-beacon-deploy",
        &cli.chain_id,
        cli.ttl_secs,
        Some(&cli.granter),
    )
    .map_err(|e| anyhow!(e))?;
    if granter != cli.granter {
        bail!("Zigner granter {granter} != expected {}", cli.granter);
    }
    let mut builder = DaemonBuilder::new(chain);
    builder.load_network(false);
    let mut session = cw_orch_kms::ApprovalSession::open(&granter);
    session = match &plan.scope {
        Scope::Store(wasm) => session.allow_store(wasm).map_err(|e| anyhow!(e))?,
        Scope::Instantiate => session.allow_instantiate(),
        Scope::Execute {
            contract,
            keys,
            calls,
        } => session
            .allow_execute(cw_orch_kms::ScopedCall {
                address: contract.clone(),
                keys: keys.clone(),
                calls: *calls,
                funds: vec![],
            })
            .map_err(|e| anyhow!(e))?,
    };
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cw_orch_kms::planned_spend_limit(2);
    let actions = vec![cw_orch_kms::SigningRequest::new("session-key").body(plan.step.clone())];
    let run = cw_orch_kms::enable_session(&builder, session, actions).map_err(|e| anyhow!(e))?;
    let daemon = run.daemon.clone();
    println!("session key (grantee): {}", run.grantee());
    let anys: Vec<Any> = plan.msgs.iter().map(|(a, _)| a.clone()).collect();
    let memo = format!("terp-beacon-deploy {}", plan.step);
    let result = (|| -> Result<()> {
        // simulate does not add the outer MsgExec(session) that commit_tx_any adds.
        let sim = vec![any(
            T_AUTHZ_EXEC,
            &cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec {
                grantee: run.grantee().into(),
                msgs: anys.clone(),
            },
        )];
        run.step("simulate", Status::Running, None, None);
        let (gas, fee) = daemon
            .rt_handle
            .block_on(daemon.sender().simulate(sim, Some(&memo)))
            .map_err(|e| anyhow!(e))?;
        println!("simulate OK gas={gas} fee={fee}");
        run.step(
            "simulate",
            Status::Confirmed,
            None,
            Some(&format!("gas {gas}, fee {fee}")),
        );
        run.step("broadcast", Status::Running, None, None);
        let resp = daemon
            .rt_handle
            .block_on(daemon.sender().commit_tx_any(anys, Some(&memo)))
            .map_err(|e| anyhow!(e))?;
        println!("BROADCAST txhash: {} code {}", resp.txhash, resp.code);
        if resp.code != 0 {
            run.step(
                "broadcast",
                Status::Error,
                Some(&resp.txhash),
                Some(&format!("code {}", resp.code)),
            );
            bail!("code {}", resp.code);
        }
        run.step("broadcast", Status::Confirmed, Some(&resp.txhash), None);
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

fn hexenc<T: AsRef<[u8]>>(b: T) -> String {
    b.as_ref().iter().map(|x| format!("{x:02x}")).collect()
}
