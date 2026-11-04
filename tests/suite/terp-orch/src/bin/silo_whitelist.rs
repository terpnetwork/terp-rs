//! Silo-auth capability whitelist (cw-infuser `whitelist-mtree`) on morocco-1,
//! signed only through cw-orch-kms. Same shape as `fiab_bounty_escrow`.
//!
//! Every step defaults to PREVIEW: it builds the exact messages, prints them with
//! the signer, and writes `<pack>/cw-orch-preview/<step>.json`. No network, no
//! session, no signature. `--execute` (plus SILO_WHITELIST_EXECUTE=1) opens a
//! cw-orch-kms approval session, simulates, then broadcasts as MsgExec under that
//! grant and closes (revokes) the session. `--execute --simulate-only` stops after
//! the simulation (the session grant itself is still signed and revoked).
//!
//! Ownership: the Foundation DAO (dao-dao-core) is the wasm admin AND the only
//! entry in the contract's ADMIN_LIST, so instantiate, root rotation and migrate all
//! go through Foundation DAO proposals (pre-propose-single, members only, no deposit).
//! Root rotation needs the `UpdateMerkleTree` execute that b9d162f lacks; see the
//! cw-infuser branch feat/whitelist-mtree-update-root-20261104 (a9d8c44).
//!
//! Build: cargo build -p terp-orch --features gov --bin silo_whitelist
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::cosmwasm::wasm::v1::{AccessConfig, MsgExecuteContract, MsgStoreCode};
use cosmos_sdk_proto::prost::Message;
use cosmwasm_std::Binary;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const T_STORE: &str = "/cosmwasm.wasm.v1.MsgStoreCode";
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
const ACCESS_ANY_OF_ADDRESSES: i32 = 4;
const EXECUTE_ENV: &str = "SILO_WHITELIST_EXECUTE";

#[derive(Parser)]
#[command(about = "Silo-auth capability whitelist rollout via cw-orch-kms (preview by default)")]
struct Cli {
    /// Rollout pack dir (holds config.json; see silo-whitelist/config.example.json).
    #[arg(long, env = "SILO_WL_PACK", default_value = ".")]
    pack: PathBuf,
    /// Sign and broadcast through cw-orch-kms. Also needs SILO_WHITELIST_EXECUTE=1.
    #[arg(long)]
    execute: bool,
    /// With --execute: simulate only, never broadcast the step's messages.
    #[arg(long)]
    simulate_only: bool,
    /// Cold granter address. Empty with --execute: the Zigner signature picks it
    /// and it must equal the step's signer.
    #[arg(long, default_value = "")]
    granter: String,
    /// gRPC endpoint for --execute (required then; no default). Previews never connect.
    #[arg(long, env = "GRPC_URL")]
    grpc_url: Option<String>,
    #[arg(long, default_value_t = 0u32)]
    account_index: u32,
    #[arg(long, default_value_t = 30 * 60)]
    ttl_secs: u64,
    #[command(subcommand)]
    step: Step,
}

#[derive(Subcommand)]
enum Step {
    /// 00: MsgStoreCode; instantiate permission AnyOfAddresses[Foundation DAO core].
    Store {
        #[arg(long)]
        wasm: PathBuf,
        /// Uploader (granter) address. Required: no default uploader.
        #[arg(long)]
        uploader: String,
    },
    /// 01: DAO proposal: instantiate (wasm admin = DAO, admins = [DAO]) with the initial root.
    ProposeInstantiate {
        #[arg(long)]
        code_id: u64,
        /// Initial root: 64 lowercase hex (silo-auth tree builder output).
        #[arg(long)]
        root: String,
        #[arg(long)]
        tree_uri: Option<String>,
        /// Proposing Foundation DAO member (cw-orch-kms granter).
        #[arg(long)]
        member: String,
    },
    /// 02: DAO proposal: rotate the root (UpdateMerkleTree; needs the patched code).
    ProposeUpdateRoot {
        #[arg(long)]
        contract: String,
        #[arg(long)]
        root: String,
        #[arg(long)]
        tree_uri: Option<String>,
        #[arg(long)]
        member: String,
    },
    /// 02b: DAO proposal: migrate to another code id (wasm admin = DAO).
    ProposeMigrate {
        #[arg(long)]
        contract: String,
        #[arg(long)]
        new_code_id: u64,
        #[arg(long)]
        member: String,
    },
    /// Vote on a Foundation DAO proposal (dao-proposal-single).
    Vote {
        #[arg(long)]
        proposal_id: u64,
        #[arg(long, value_parser = ["yes", "no", "abstain"])]
        vote: String,
        #[arg(long)]
        member: String,
    },
    /// Execute a passed Foundation DAO proposal.
    Execute {
        #[arg(long)]
        proposal_id: u64,
        #[arg(long)]
        member: String,
    },
}

struct Plan {
    step: String,
    signer: String,
    note: String,
    session: Value,
    msgs: Vec<(Any, Value)>,
    kind: SessionKind,
}

enum SessionKind {
    Store(Vec<u8>),
    Execute { contract: String, key: String },
}

struct Cfg {
    raw: Value,
    dao_core: String,
    pre_propose: String,
    proposal_module: String,
    members: Vec<String>,
}

fn load_cfg(pack: &Path) -> Result<Cfg> {
    let raw: Value = serde_json::from_slice(
        &std::fs::read(pack.join("config.json")).context("read <pack>/config.json")?,
    )?;
    let s = |k: &str| -> Result<String> {
        raw[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("config.{k} missing"))
    };
    if s("chain_id")? != "morocco-1" {
        bail!("config.chain_id must be morocco-1");
    }
    let members = raw["dao_members"].as_array().ok_or_else(|| anyhow!("config.dao_members missing"))?
        .iter().map(|m| m.as_str().map(str::to_string).ok_or_else(|| anyhow!("dao_members: not a string")))
        .collect::<Result<Vec<_>>>()?;
    for a in [s("dao_core")?, s("pre_propose_single")?, s("proposal_single")?].iter().chain(members.iter()) {
        bech32_data(a).with_context(|| format!("bad address {a}"))?;
        if !a.starts_with("terp1") { bail!("{a}: not a terp1 address"); }
    }
    Ok(Cfg { dao_core: s("dao_core")?, pre_propose: s("pre_propose_single")?, proposal_module: s("proposal_single")?, members, raw })
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
    let plan = build_plan(&cfg, &cli.step)?;
    print_and_write(&cli.pack, &plan)?;
    if !cli.execute {
        println!("mode: PREVIEW (nothing signed). Re-run with --execute and {EXECUTE_ENV}=1 to sign via cw-orch-kms.");
        return Ok(());
    }
    execute(&cli, &cfg, plan)
}

fn build_plan(cfg: &Cfg, step: &Step) -> Result<Plan> {
    let member_ok = |m: &str| -> Result<()> {
        if !cfg.members.iter().any(|x| x == m) {
            bail!("{m} is not in config.dao_members (Foundation DAO cw4 group)");
        }
        Ok(())
    };
    Ok(match step {
        Step::Store { wasm, uploader } => {
            let bytes = std::fs::read(wasm).with_context(|| format!("read {}", wasm.display()))?;
            let sum = hexenc(cw_orch_kms::wasm_checksum(&bytes).map_err(|e| anyhow!(e))?);
            let want = cfg.raw["wasm_sha256"].as_str().ok_or_else(|| anyhow!("config.wasm_sha256 missing"))?;
            if sum != want {
                bail!("wasm sha256 {sum} != config {want}");
            }
            let msg = MsgStoreCode {
                sender: uploader.clone(),
                wasm_byte_code: bytes.clone(),
                instantiate_permission: Some(AccessConfig {
                    permission: ACCESS_ANY_OF_ADDRESSES,
                    addresses: vec![cfg.dao_core.clone()],
                }),
            };
            let view = json!({"@type": T_STORE, "sender": uploader, "wasm_byte_code":
                format!("<{} bytes, sha256 {sum}>", bytes.len()),
                "instantiate_permission": {"permission": "AnyOfAddresses", "addresses": [cfg.dao_core]}});
            Plan {
                step: "00-store".into(),
                signer: uploader.clone(),
                note: "uploader pays gas; only the Foundation DAO core may instantiate the code".into(),
                session: json!({"stores": [sum], "type_urls": [T_STORE]}),
                msgs: vec![(any(T_STORE, &msg), view)],
                kind: SessionKind::Store(bytes),
            }
        }
        Step::ProposeInstantiate { code_id, root, tree_uri, member } => {
            member_ok(member)?;
            let p = instantiate_proposal(cfg, *code_id, root, tree_uri.as_deref())?;
            propose_plan("01-propose-instantiate", cfg, member, p)
        }
        Step::ProposeUpdateRoot { contract, root, tree_uri, member } => {
            member_ok(member)?;
            if cfg.raw["contract_supports_update_root"] != json!(true) {
                bail!("config.contract_supports_update_root is not true: cw-infuser b9d162f has no UpdateMerkleTree execute; store the patched code (a9d8c44) and migrate first");
            }
            let p = update_root_proposal(contract, root, tree_uri.as_deref())?;
            propose_plan("02-propose-update-root", cfg, member, p)
        }
        Step::ProposeMigrate { contract, new_code_id, member } => {
            member_ok(member)?;
            bech32_data(contract)?;
            let desc = format!("Migrates the silo-auth capability whitelist {contract} to code {new_code_id}. The Foundation DAO is the wasm admin. Storage (root, admins) is kept; whitelist-mtree migrate only bumps cw2.");
            let p = json!({"title": "Silo-auth whitelist: migrate", "description": desc,
                "msgs": [{"wasm": {"migrate": {"contract_addr": contract, "new_code_id": new_code_id, "msg": b64json(&json!({}))?}}}]});
            propose_plan("02b-propose-migrate", cfg, member, p)
        }
        Step::Vote { proposal_id, vote, member } => {
            member_ok(member)?;
            exec_plan(&format!("vote-{proposal_id}"), &cfg.proposal_module, member, "vote",
                json!({"vote": {"proposal_id": proposal_id, "vote": vote}}))
        }
        Step::Execute { proposal_id, member } => {
            member_ok(member)?;
            exec_plan(&format!("execute-{proposal_id}"), &cfg.proposal_module, member, "execute",
                json!({"execute": {"proposal_id": proposal_id}}))
        }
    })
}

fn check_root(root: &str) -> Result<()> {
    if root.len() != 64 || !root.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        bail!("root must be 64 lowercase hex chars (blake3 sorted-pair root)");
    }
    Ok(())
}

fn instantiate_msg(cfg: &Cfg, root: &str, tree_uri: Option<&str>) -> Result<Value> {
    check_root(root)?;
    Ok(json!({"merkle_root": root, "merkle_tree_uri": tree_uri, "admins": [cfg.dao_core], "admins_mutable": true}))
}

fn instantiate_proposal(cfg: &Cfg, code_id: u64, root: &str, tree_uri: Option<&str>) -> Result<Value> {
    let label = cfg.raw["label"].as_str().unwrap_or("silo-auth-capabilities");
    let m = instantiate_msg(cfg, root, tree_uri)?;
    let desc = format!(
        "Instantiates the silo-auth capability whitelist (cw-infuser whitelist-mtree, code {code_id}). Initial Merkle root {root}: Foundation-DAO admin leaves (silo-admin/v1) and upload-capability leaves in one tree. Wasm admin = Foundation DAO; ADMIN_LIST = [Foundation DAO] (mutable), so every later root rotation or migrate is a DAO proposal. No funds move."
    );
    Ok(json!({"title": "Silo-auth: instantiate the capability whitelist", "description": desc,
        "msgs": [{"wasm": {"instantiate": {"admin": cfg.dao_core, "code_id": code_id,
            "msg": b64json(&m)?, "funds": [], "label": label}}}]}))
}

fn update_root_proposal(contract: &str, root: &str, tree_uri: Option<&str>) -> Result<Value> {
    check_root(root)?;
    bech32_data(contract)?;
    let inner = json!({"update_merkle_tree": {"merkle_root": root, "merkle_tree_uri": tree_uri}});
    let desc = format!("Rotates the silo-auth capability root on {contract} to {root}. The adapter honours the old root for its configured block grace, then only the new one.");
    Ok(json!({"title": "Silo-auth: rotate capability root", "description": desc,
        "msgs": [{"wasm": {"execute": {"contract_addr": contract, "msg": b64json(&inner)?, "funds": []}}}]}))
}

fn propose_plan(step: &str, cfg: &Cfg, member: &str, proposal: Value) -> Plan {
    // dao-pre-propose-single: {"propose":{"msg":{"propose":{title,description,msgs}}}}
    let msg = json!({"propose": {"msg": {"propose": proposal}}});
    let mut plan = exec_plan(step, &cfg.pre_propose, member, "propose", msg);
    plan.note = "Foundation DAO member submits through dao-pre-propose-single (members only, no deposit)".into();
    plan
}

fn exec_plan(step: &str, contract: &str, member: &str, key: &str, msg: Value) -> Plan {
    let m = MsgExecuteContract {
        sender: member.into(),
        contract: contract.into(),
        msg: serde_json::to_vec(&msg).expect("json"),
        funds: vec![],
    };
    Plan {
        step: step.into(),
        signer: member.into(),
        note: format!("Foundation DAO member calls {key} on {contract}"),
        session: json!({"executes": [{"address": contract, "keys": [key], "calls": 1, "funds": []}],
                        "type_urls": [T_EXEC]}),
        msgs: vec![(any(T_EXEC, &m), json!({"@type": T_EXEC, "sender": member,
                    "contract": contract, "msg": msg, "funds": []}))],
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
    let chain_id = "morocco-1".to_string();
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
        cw_orch_kms::capture_granter_for_chain_opts_sync("127.0.0.1:8741".parse()?, cli.account_index, "silo-whitelist", &chain_id, cli.ttl_secs, Some(&plan.signer))
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
    let memo = format!("silo-whitelist {}", plan.step);
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

fn b64json(v: &Value) -> Result<String> {
    Ok(Binary::from(serde_json::to_vec(v)?).to_base64())
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
    const DAO: &str = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w";
    const M1: &str = "terp16xmf57fr64n9ajux0up3n6ypytct5zggj5d05r";
    const ROOT: &str = "7e77c434000000000000000000000000000000000000000000000000000000c2";

    fn cfg(update: bool) -> Cfg {
        Cfg {
            raw: json!({"chain_id": "morocco-1", "contract_supports_update_root": update}),
            dao_core: DAO.into(),
            pre_propose: "terp1armcr7lme0ac3l59dx6ne3et3pe9mfyv57d8pc9m5tyxtzm64dpq9nnwpp".into(),
            proposal_module: "terp1p5582k3sw008jkmw0f48ucszn660nazqr9x3ut6h5aqj4frq8f0shaey4g".into(),
            members: vec![M1.into()],
        }
    }

    #[test]
    fn instantiate_sets_dao_as_admin_and_root() {
        let p = build_plan(&cfg(false), &Step::ProposeInstantiate { code_id: 7, root: ROOT.into(), tree_uri: None, member: M1.into() }).unwrap();
        let v = &p.msgs[0].1["msg"]["propose"]["msg"]["propose"]["msgs"][0]["wasm"]["instantiate"];
        assert_eq!(v["admin"], DAO);
        let inner: Value = serde_json::from_slice(&Binary::from_base64(v["msg"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(inner["admins"], json!([DAO]));
        assert_eq!(inner["merkle_root"], ROOT);
        assert_eq!(p.signer, M1);
    }

    #[test]
    fn non_member_and_bad_root_refused() {
        let c = cfg(true);
        assert!(build_plan(&c, &Step::ProposeInstantiate { code_id: 7, root: ROOT.into(), tree_uri: None, member: DAO.into() }).is_err());
        assert!(build_plan(&c, &Step::ProposeInstantiate { code_id: 7, root: "XYZ".into(), tree_uri: None, member: M1.into() }).is_err());
    }

    #[test]
    fn update_root_gated_on_patched_code() {
        let s = Step::ProposeUpdateRoot { contract: DAO.into(), root: ROOT.into(), tree_uri: None, member: M1.into() };
        assert!(build_plan(&cfg(false), &s).is_err());
        let p = build_plan(&cfg(true), &s).unwrap();
        let e = &p.msgs[0].1["msg"]["propose"]["msg"]["propose"]["msgs"][0]["wasm"]["execute"];
        let inner: Value = serde_json::from_slice(&Binary::from_base64(e["msg"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(inner["update_merkle_tree"]["merkle_root"], ROOT);
    }

    #[test]
    fn simulate_body_is_session_wrapped() {
        use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec;
        let inner = any(T_AUTHZ_EXEC, &MsgExec { grantee: "member".into(), msgs: vec![] });
        let w = session_wrap("session", vec![inner.clone()]);
        let outer = MsgExec::decode(w[0].value.as_slice()).unwrap();
        assert_eq!(outer.grantee, "session");
    }
}
