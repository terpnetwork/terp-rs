//! silo-cap-root self-deploy (morocco-1) through cw-orch-kms, signed by OUR wallet.
//! No DAO, no governance. Steps: store, instantiate, update-root, query.
//!
//! Default = PREVIEW: prints the exact messages and writes
//! `<pack>/cw-orch-preview/<step>.json`. Nothing is signed.
//! Signing requires BOTH `--execute` and `SILO_CAP_ROOT_EXECUTE=1`; then a
//! cw-orch-kms approval session is opened for the granter (the deployer
//! wallet), the tx is simulated, broadcast via authz MsgExec, and the session
//! grant is revoked. `--simulate-only` stops after simulate (still needs the
//! session, i.e. a signature; do not use it where signing is off-limits).
//!
//! The contract admin defaults to the deployer. Hand it to the Foundation DAO
//! (terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w) later with
//! an `update_admin` execute; this bin does not do that.
use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use cosmos_sdk_proto::cosmwasm::wasm::v1::{AccessConfig, MsgExecuteContract, MsgInstantiateContract, MsgStoreCode};
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::prost::Message;
use cosmwasm_std::Binary;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const EXECUTE_ENV: &str = "SILO_CAP_ROOT_EXECUTE";
const CHAIN_ID: &str = "morocco-1";
const T_STORE: &str = "/cosmwasm.wasm.v1.MsgStoreCode";
const T_INST: &str = "/cosmwasm.wasm.v1.MsgInstantiateContract";
const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
const ACCESS_ANY_OF_ADDRESSES: i32 = 4;
pub const FOUNDATION_DAO: &str = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w";

#[derive(Parser)]
#[command(about = "silo-cap-root self-deploy via cw-orch-kms (preview by default)")]
struct Cli {
    /// Deploy pack dir holding config.json; previews go to <pack>/cw-orch-preview/.
    #[arg(long, default_value = "tests/suite/terp-orch/silo-cap-root")]
    pack: PathBuf,
    /// Sign and broadcast (also needs SILO_CAP_ROOT_EXECUTE=1).
    #[arg(long)]
    execute: bool,
    /// With --execute: stop after simulate.
    #[arg(long)]
    simulate_only: bool,
    /// Granter (deployer) address if already known; otherwise captured in the cw-orch-granter page.
    #[arg(long, default_value = "")]
    granter: String,
    #[arg(long, env = "GRPC_URL")]
    grpc_url: Option<String>,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 900)]
    ttl_secs: u64,
    #[command(subcommand)]
    step: Step,
}

#[derive(Subcommand, Clone)]
enum Step {
    /// MsgStoreCode; wasm sha256 must equal config.wasm_sha256. Instantiate permission = deployer only.
    Store { #[arg(long)] wasm: PathBuf, #[arg(long)] deployer: String },
    /// MsgInstantiateContract. --admin defaults to --deployer (contract admin and wasm admin).
    Instantiate {
        #[arg(long)] code_id: u64,
        #[arg(long)] root: String,
        #[arg(long)] deployer: String,
        #[arg(long)] admin: Option<String>,
        #[arg(long)] metadata: Option<String>,
    },
    /// Execute update_root (sender must be the contract admin).
    UpdateRoot { #[arg(long)] contract: String, #[arg(long)] root: String, #[arg(long)] admin: String },
    /// Print the smart-query bodies / LCD URLs for root and config (no signing, no network).
    Query { #[arg(long)] contract: String },
}

enum SessionKind { Store(Vec<u8>), Instantiate { code_id: u64 }, Execute { contract: String, key: String } }

struct Plan {
    step: String,
    signer: String,
    note: String,
    session: Value,
    msgs: Vec<(Any, Value)>,
    kind: SessionKind,
}

struct Cfg { raw: Value }

fn load_cfg(pack: &Path) -> Result<Cfg> {
    let raw: Value = serde_json::from_slice(&std::fs::read(pack.join("config.json")).context("read <pack>/config.json")?)?;
    if raw["chain_id"].as_str() != Some(CHAIN_ID) {
        bail!("config.chain_id must be {CHAIN_ID}");
    }
    Ok(Cfg { raw })
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let cfg = load_cfg(&cli.pack)?;
    if let Step::Query { contract } = &cli.step {
        return print_query(&cfg, contract);
    }
    let plan = build_plan(&cfg, &cli.step)?;
    print_and_write(&cli.pack, &plan)?;
    if !cli.execute {
        println!("mode: PREVIEW (nothing signed). Re-run with --execute and {EXECUTE_ENV}=1 to sign via cw-orch-kms.");
        return Ok(());
    }
    execute(&cli, &cfg, plan)
}

fn check_addr(a: &str) -> Result<()> {
    bech32_data(a).with_context(|| format!("bad address {a}"))?;
    if !a.starts_with("terp1") { bail!("{a}: not a terp1 address"); }
    Ok(())
}

fn check_root(root: &str) -> Result<()> {
    if root.len() != 64 || !root.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        bail!("root must be 64 lowercase hex chars (blake3 sorted-pair root from silo-cap-tree)");
    }
    Ok(())
}

fn build_plan(cfg: &Cfg, step: &Step) -> Result<Plan> {
    Ok(match step {
        Step::Store { wasm, deployer } => {
            check_addr(deployer)?;
            let bytes = std::fs::read(wasm).with_context(|| format!("read {}", wasm.display()))?;
            let sum = hexenc(cw_orch_kms::wasm_checksum(&bytes).map_err(|e| anyhow!(e))?);
            let want = cfg.raw["wasm_sha256"].as_str().ok_or_else(|| anyhow!("config.wasm_sha256 missing"))?;
            if sum != want {
                bail!("wasm sha256 {sum} != pinned config.wasm_sha256 {want}");
            }
            let msg = MsgStoreCode {
                sender: deployer.clone(),
                wasm_byte_code: bytes.clone(),
                instantiate_permission: Some(AccessConfig { permission: ACCESS_ANY_OF_ADDRESSES, addresses: vec![deployer.clone()] }),
            };
            let view = json!({"@type": T_STORE, "sender": deployer,
                "wasm_byte_code": format!("<{} bytes, sha256 {sum}>", bytes.len()),
                "instantiate_permission": {"permission": "AnyOfAddresses", "addresses": [deployer]}});
            Plan {
                step: "00-store".into(),
                signer: deployer.clone(),
                note: "deployer stores the code; only the deployer may instantiate it".into(),
                session: json!({"stores": [sum], "type_urls": [T_STORE]}),
                msgs: vec![(any(T_STORE, &msg), view)],
                kind: SessionKind::Store(bytes),
            }
        }
        Step::Instantiate { code_id, root, deployer, admin, metadata } => {
            check_addr(deployer)?;
            check_root(root)?;
            let admin = admin.clone().unwrap_or_else(|| deployer.clone());
            check_addr(&admin)?;
            let label = cfg.raw["label"].as_str().unwrap_or("silo-cap-root").to_string();
            let inner = json!({"admin": admin, "merkle_root": root, "metadata": metadata});
            let msg = MsgInstantiateContract {
                sender: deployer.clone(),
                admin: admin.clone(),
                code_id: *code_id,
                label: label.clone(),
                msg: serde_json::to_vec(&inner)?,
                funds: vec![],
            };
            Plan {
                step: "01-instantiate".into(),
                signer: deployer.clone(),
                note: format!("contract admin + wasm admin = {admin}; hand to the Foundation DAO later via update_admin"),
                session: json!({"instantiates": [{"code_id": code_id, "funds": []}], "type_urls": [T_INST]}),
                msgs: vec![(any(T_INST, &msg), json!({"@type": T_INST, "sender": deployer, "admin": admin,
                    "code_id": code_id, "label": label, "msg": inner, "funds": []}))],
                kind: SessionKind::Instantiate { code_id: *code_id },
            }
        }
        Step::UpdateRoot { contract, root, admin } => {
            check_addr(contract)?;
            check_addr(admin)?;
            check_root(root)?;
            exec_plan("02-update-root", contract, admin, "update_root", json!({"update_root": {"root": root}}))
        }
        Step::Query { .. } => unreachable!(),
    })
}

fn print_query(cfg: &Cfg, contract: &str) -> Result<()> {
    check_addr(contract)?;
    let lcd = cfg.raw["lcd"].as_str().unwrap_or("<LCD>");
    for q in [json!({"root": {}}), json!({"config": {}})] {
        let b64 = Binary::from(serde_json::to_vec(&q)?).to_base64();
        println!("smart query {q}\n  {lcd}/cosmwasm/wasm/v1/contract/{contract}/smart/{b64}");
    }
    // raw key silo-auth proves: 0x03 || contract || "merkle_root"; LCD raw path takes the key without the 0x03 prefix.
    println!("raw key (silo-auth proof): {lcd}/cosmwasm/wasm/v1/contract/{contract}/raw/{}", Binary::from(b"merkle_root".to_vec()).to_base64());
    Ok(())
}

fn exec_plan(step: &str, contract: &str, sender: &str, key: &str, msg: Value) -> Plan {
    let m = MsgExecuteContract { sender: sender.into(), contract: contract.into(), msg: serde_json::to_vec(&msg).expect("json"), funds: vec![] };
    Plan {
        step: step.into(),
        signer: sender.into(),
        note: format!("contract admin calls {key} on {contract}"),
        session: json!({"executes": [{"address": contract, "keys": [key], "calls": 1, "funds": []}], "type_urls": [T_EXEC]}),
        msgs: vec![(any(T_EXEC, &m), json!({"@type": T_EXEC, "sender": sender, "contract": contract, "msg": msg, "funds": []}))],
        kind: SessionKind::Execute { contract: contract.into(), key: key.into() },
    }
}

fn print_and_write(pack: &Path, plan: &Plan) -> Result<()> {
    let msgs: Vec<Value> = plan.msgs.iter().map(|(a, v)| {
        let shown = if a.type_url == T_STORE { "<omitted: contains the wasm>".to_string() } else { hexenc(&a.value) };
        json!({"type_url": a.type_url, "json": v, "proto_value_hex": shown, "proto_value_len": a.value.len()})
    }).collect();
    let out = json!({
        "_NOT_SIGNED": "cw-orch-kms preview; nothing was signed or broadcast",
        "step": plan.step,
        "signer_granter": plan.signer,
        "broadcaster": "ephemeral cw-orch-kms session key (generated at --execute time), wraps the messages in MsgExec(granter)",
        "note": plan.note,
        "kms_session_scope": plan.session,
        "messages": msgs,
    });
    let dir = pack.join("cw-orch-preview");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", plan.step));
    std::fs::write(&path, serde_json::to_string_pretty(&out)? + "\n")?;
    println!("== {} ({})", plan.step, plan.note);
    println!("signer (cw-orch-kms granter): {}", plan.signer);
    for (a, v) in &plan.msgs {
        println!("message {}:\n{}", a.type_url, serde_json::to_string_pretty(v)?);
    }
    println!("wrote {}", path.display());
    Ok(())
}

fn execute(cli: &Cli, cfg: &Cfg, plan: Plan) -> Result<()> {
    use cw_orch::daemon::{DaemonBuilder, TxSender};
    use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
    if std::env::var(EXECUTE_ENV).ok().as_deref() != Some("1") {
        bail!("refusing to sign: --execute also needs {EXECUTE_ENV}=1");
    }
    let chain_id = CHAIN_ID.to_string();
    let chain = ChainInfoOwned {
        chain_id: chain_id.clone(),
        gas_denom: "uthiol".into(),
        gas_price: 0.05,
        grpc_urls: vec![cli.grpc_url.clone()
            .ok_or_else(|| anyhow!("--execute needs --grpc-url or GRPC_URL (a morocco-1 gRPC endpoint)"))?],
        rpc_urls: vec![],
        lcd_url: cfg.raw["lcd"].as_str().map(str::to_string),
        fcd_url: None,
        network_info: NetworkInfoOwned { chain_name: "terp".into(), pub_address_prefix: "terp".into(), coin_type: 118 },
        kind: ChainKind::Mainnet,
    };
    // SAFETY: set before cw-orch reads it; nothing else races on the env.
    unsafe { std::env::set_var("CW_ORCH_WALLET_BALANCE_ASSERTION", "false") };
    let granter = if cli.granter.is_empty() {
        println!("sign the cw-orch-granter challenge at http://127.0.0.1:8741");
        cw_orch_kms::capture_granter_for_chain_opts_sync("127.0.0.1:8741".parse()?, cli.account_index, "silo-cap-root", &chain_id, cli.ttl_secs, Some(&plan.signer))
            .map_err(|e| anyhow!(e))?
    } else {
        cw_orch_kms::set_approval_chain_id(&chain_id).map_err(|e| anyhow!(e))?;
        cli.granter.clone()
    };
    if granter != plan.signer {
        bail!("granter {granter} is not this step's signer {}", plan.signer);
    }
    let mut builder = DaemonBuilder::new(chain);
    builder.load_network(false);
    let mut session = cw_orch_kms::ApprovalSession::open(&granter);
    session = match &plan.kind {
        SessionKind::Store(wasm) => session.allow_store(wasm).map_err(|e| anyhow!(e))?,
        // MsgInstantiateContract authz is type-wide on chain (no code-id scope); the TTL bounds it.
        SessionKind::Instantiate { code_id } => {
            println!("session: MsgInstantiateContract authz is type-wide; this run instantiates code {code_id} only");
            session.allow_instantiate()
        }
        SessionKind::Execute { contract, key } => session
            .allow_execute(cw_orch_kms::ScopedCall { address: contract.clone(), keys: vec![key.clone()], calls: 1, funds: vec![] })
            .map_err(|e| anyhow!(e))?,
    };
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cw_orch_kms::planned_spend_limit(4);
    let actions = vec![cw_orch_kms::SigningRequest::new("session-key").body(plan.step.clone())];
    let run = cw_orch_kms::enable_session(&builder, session, actions).map_err(|e| anyhow!(e))?;
    let daemon = run.daemon.clone();
    println!("session key (grantee): {}", run.grantee());
    let anys: Vec<Any> = plan.msgs.iter().map(|(a, _)| a.clone()).collect();
    let memo = format!("silo-cap-root {}", plan.step);
    let result = (|| -> Result<()> {
        let sim = session_wrap(run.grantee(), anys.clone());
        use cw_orch_kms::progress::Status;
        run.step("simulate", Status::Running, None, None);
        let (gas, fee) = match daemon.rt_handle.block_on(daemon.sender().simulate(sim, Some(&memo))) {
            Ok(v) => v,
            Err(e) => { run.step("simulate", Status::Error, None, Some(&e.to_string())); return Err(e.into()); }
        };
        println!("simulate OK gas={gas} fee={fee}");
        run.step("simulate", Status::Confirmed, None, Some(&format!("gas {gas}, fee {fee}")));
        if cli.simulate_only {
            println!("--simulate-only: not broadcasting");
            return Ok(());
        }
        run.step("broadcast", Status::Running, None, None);
        let resp = match daemon.rt_handle.block_on(daemon.sender().commit_tx_any(anys, Some(&memo))) {
            Ok(r) => r,
            Err(e) => { run.step("broadcast", Status::Error, None, Some(&e.to_string())); return Err(e.into()); }
        };
        println!("BROADCAST txhash: {}", resp.txhash);
        if resp.code == 0 {
            run.step("broadcast", Status::Confirmed, Some(&resp.txhash), None);
        } else {
            run.step("broadcast", Status::Error, Some(&resp.txhash), Some(&format!("code {}", resp.code)));
        }
        Ok(())
    })();
    run.close().map_err(|e| anyhow!(e))?;
    println!("session grant closed");
    result
}

/// The outer MsgExec(session) that commit_tx_any adds when authz_granter is set; simulate needs it explicitly.
fn session_wrap(session_grantee: &str, msgs: Vec<Any>) -> Vec<Any> {
    use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec;
    vec![any(T_AUTHZ_EXEC, &MsgExec { grantee: session_grantee.into(), msgs })]
}

fn any<M: Message>(type_url: &str, m: &M) -> Any {
    Any { type_url: type_url.into(), value: m.encode_to_vec() }
}


/// Minimal bech32 (BIP-173) decode to data bytes, checksum verified.
fn bech32_data(addr: &str) -> Result<Vec<u8>> {
    const CS: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let pos = addr.rfind('1').ok_or_else(|| anyhow!("bech32: no separator"))?;
    let (hrp, data) = (&addr[..pos], &addr[pos + 1..]);
    if data.len() < 6 { bail!("bech32: too short"); }
    let vals: Vec<u8> = data.chars().map(|c| CS.find(c).map(|i| i as u8)).collect::<Option<_>>()
        .ok_or_else(|| anyhow!("bech32: bad char"))?;
    let mut chk: Vec<u8> = hrp.bytes().map(|b| b >> 5).collect();
    chk.push(0);
    chk.extend(hrp.bytes().map(|b| b & 31));
    chk.extend(&vals);
    let gens = [0x3b6a57b2u32, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut p = 1u32;
    for v in chk {
        let b = p >> 25;
        p = ((p & 0x1ffffff) << 5) ^ v as u32;
        for (i, g) in gens.iter().enumerate() {
            if (b >> i) & 1 == 1 { p ^= g; }
        }
    }
    if p != 1 { bail!("bech32: bad checksum"); }
    let (mut acc, mut bits, mut out) = (0u32, 0u32, vec![]);
    for v in &vals[..vals.len() - 6] {
        acc = (acc << 5) | *v as u32;
        bits += 5;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
    }
    Ok(out)
}

fn hexenc<T: AsRef<[u8]>>(b: T) -> String { b.as_ref().iter().map(|x| format!("{:02x}", x)).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    const ME: &str = "terp16xmf57fr64n9ajux0up3n6ypytct5zggj5d05r";
    const ROOT: &str = "7e77c434000000000000000000000000000000000000000000000000000000c2";
    fn cfg() -> Cfg { Cfg { raw: json!({"chain_id": "morocco-1", "wasm_sha256": "00", "label": "silo-cap-root"}) } }

    #[test]
    fn instantiate_admin_defaults_to_deployer() {
        let p = build_plan(&cfg(), &Step::Instantiate { code_id: 7, root: ROOT.into(), deployer: ME.into(), admin: None, metadata: None }).unwrap();
        let v = &p.msgs[0].1;
        assert_eq!(v["admin"], ME);
        assert_eq!(v["msg"]["admin"], ME);
        assert_eq!(v["msg"]["merkle_root"], ROOT);
        assert_eq!(p.signer, ME);
        let m = MsgInstantiateContract::decode(p.msgs[0].0.value.as_slice()).unwrap();
        assert_eq!(m.code_id, 7);
        let inner: Value = serde_json::from_slice(&m.msg).unwrap();
        assert_eq!(inner["merkle_root"], ROOT);
    }

    #[test]
    fn instantiate_explicit_admin_e_g_foundation_dao() {
        let p = build_plan(&cfg(), &Step::Instantiate { code_id: 7, root: ROOT.into(), deployer: ME.into(), admin: Some(FOUNDATION_DAO.into()), metadata: Some("m".into()) }).unwrap();
        assert_eq!(p.msgs[0].1["admin"], FOUNDATION_DAO);
        assert_eq!(p.msgs[0].1["msg"]["metadata"], "m");
    }

    #[test]
    fn bad_root_and_address_refused() {
        assert!(build_plan(&cfg(), &Step::Instantiate { code_id: 7, root: "XYZ".into(), deployer: ME.into(), admin: None, metadata: None }).is_err());
        assert!(build_plan(&cfg(), &Step::UpdateRoot { contract: "terp1bad".into(), root: ROOT.into(), admin: ME.into() }).is_err());
    }

    #[test]
    fn update_root_message_shape() {
        let p = build_plan(&cfg(), &Step::UpdateRoot { contract: FOUNDATION_DAO.into(), root: ROOT.into(), admin: ME.into() }).unwrap();
        assert_eq!(p.msgs[0].1["msg"]["update_root"]["root"], ROOT);
        assert_eq!(p.msgs[0].0.type_url, T_EXEC);
    }

    #[test]
    fn store_refuses_unpinned_wasm() {
        let d = std::env::temp_dir().join(format!("scr-{}", std::process::id()));
        std::fs::write(&d, b"\0asm\x01\0\0\0").unwrap();
        let r = build_plan(&cfg(), &Step::Store { wasm: d.clone(), deployer: ME.into() });
        assert!(r.is_err());
        let sum = hexenc(cw_orch_kms::wasm_checksum(&std::fs::read(&d).unwrap()).unwrap());
        let ok = build_plan(&Cfg { raw: json!({"chain_id": "morocco-1", "wasm_sha256": sum}) }, &Step::Store { wasm: d.clone(), deployer: ME.into() }).unwrap();
        assert_eq!(ok.msgs[0].1["instantiate_permission"]["addresses"], json!([ME]));
        let _ = std::fs::remove_file(d);
    }

    #[test]
    fn simulate_body_is_session_wrapped() {
        use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec;
        let w = session_wrap("session", vec![]);
        assert_eq!(MsgExec::decode(w[0].value.as_slice()).unwrap().grantee, "session");
    }
}
