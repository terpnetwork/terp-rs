//! FIAB bounty-escrow rollout on morocco-1, signed only through cw-orch-kms.
//!
//! Every subcommand defaults to PREVIEW: it builds the exact messages, prints
//! them with the signer (the cw-orch-kms granter), and writes
//! `<pack>/cw-orch-preview/<step>.json`. No network, no session, no signature.
//! `--execute` (plus FIAB_ESCROW_EXECUTE=1) opens a cw-orch-kms approval
//! session (cold key on the loopback page, ephemeral grantee), simulates, then
//! broadcasts as MsgExec under that grant and closes (revokes) the session.
//!
//! Build: cargo build -p terp-orch --features gov --bin fiab_bounty_escrow
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::Timestamp;
use cosmos_sdk_proto::cosmos::authz::v1beta1::{Grant, MsgExec, MsgGrant, MsgRevoke};
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
use cosmos_sdk_proto::cosmwasm::wasm::v1::{
    AcceptedMessageKeysFilter, AccessConfig, CombinedLimit, ContractExecutionAuthorization,
    ContractGrant, MsgExecuteContract, MsgStoreCode,
};
use cosmos_sdk_proto::prost::Message;
use cosmwasm_std::Binary;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[path = "fiab_bounty_escrow/curate.rs"]
mod curate;
/// Seed set: export-seed / upload-seed / create-bounties (see seed.rs).
#[path = "fiab_bounty_escrow/seed.rs"]
mod seed;

/// Verified offline on 2026-10-28 (python protobuf + terpd 6.2.1 tx decode).
const VERIFIED_GRANT_SHA256: &str =
    "ca2251ae032406401615a425fa5707ca193225074d1518a2b5f3a2862d2a8efb";
const VERIFIED_ESCROW: &str = "terp19hrlad50vp2dc07cmnd67wtvphcggj3fjaakp2aqg5m7nrj04xpqq2fnxa";
const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const T_STORE: &str = "/cosmwasm.wasm.v1.MsgStoreCode";
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
const T_GRANT: &str = "/cosmos.authz.v1beta1.MsgGrant";
const T_REVOKE: &str = "/cosmos.authz.v1beta1.MsgRevoke";
const T_CEA: &str = "/cosmwasm.wasm.v1.ContractExecutionAuthorization";
const T_COMBINED: &str = "/cosmwasm.wasm.v1.CombinedLimit";
const T_KEYS: &str = "/cosmwasm.wasm.v1.AcceptedMessageKeysFilter";
const ACCESS_ANY_OF_ADDRESSES: i32 = 4;

#[derive(Parser)]
#[command(about = "FIAB bounty escrow rollout via cw-orch-kms (preview by default)")]
struct Cli {
    /// Rollout pack dir (holds config.json).
    #[arg(long, env = "FIAB_PACK", default_value = ".")]
    pack: PathBuf,
    /// Sign and broadcast through cw-orch-kms. Also needs FIAB_ESCROW_EXECUTE=1.
    #[arg(long)]
    execute: bool,
    /// Cold granter address. Empty with --execute: the Zigner signature picks it
    /// (cw-orch-granter challenge) and it must equal the step's signer.
    #[arg(long, default_value = "")]
    granter: String,
    /// gRPC endpoint for --execute (required then; no default). Previews never connect.
    #[arg(long, env = "GRPC_URL")]
    grpc_url: Option<String>,
    #[arg(long, default_value_t = 0u32)]
    account_index: u32,
    #[arg(long, default_value_t = 30 * 60)]
    ttl_secs: u64,
    /// Override the escrow address (only if the instantiated one differs).
    #[arg(long)]
    escrow: Option<String>,
    #[command(subcommand)]
    step: Step,
}

#[derive(Subcommand)]
enum Step {
    /// 00: MsgStoreCode, instantiate permission AnyOfAddresses[FIAB core].
    Store {
        #[arg(long)]
        wasm: PathBuf,
        /// Uploader (granter) address. Required: no default uploader.
        #[arg(long)]
        uploader: String,
    },
    /// 01: propose update_proposal_modules (instantiate2, salt from config).
    ProposeInstantiate {
        #[arg(long)]
        code_id: u64,
    },
    /// 02: propose the stargate MsgGrant (bytes pinned to the verified sha256).
    ProposeGrant,
    /// 02b: propose the stargate MsgRevoke.
    ProposeRevoke,
    /// 04: submit a lifecycle proposal JSON from gen_04_lifecycle.py.
    ProposeFile {
        #[arg(long)]
        file: PathBuf,
    },
    /// Vote on a FIAB proposal.
    Vote {
        #[arg(long)]
        proposal_id: u64,
        #[arg(long, value_parser = ["yes", "no", "abstain"])]
        vote: String,
    },
    /// Execute a passed FIAB proposal.
    Execute {
        #[arg(long)]
        proposal_id: u64,
    },
    /// 03: per-bounty create_bounty as FIAB core, through the 02 grant (nested MsgExec).
    CreateBounty {
        /// out/<id>.inner.json from lib/gen_03_create_bounty.py
        #[arg(long)]
        inner: PathBuf,
    },
    /// Seed: one canonical terp-bounty-details/v1 doc per (catalog item, denom) + manifest.json.
    ExportSeed {
        /// catalog bounties.json (round-2 build).
        #[arg(long)]
        catalog: PathBuf,
        /// Catalog horizon that is the 2-month window.
        #[arg(long, default_value = "Now")]
        horizon: String,
        /// Bounty deadline (UTC date); expires_at = this date 00:00:00Z. Pinned in the hashed doc.
        #[arg(long, default_value = "2027-01-01")]
        deadline: String,
        #[arg(long, default_value = "https://s3.terp.network/static")]
        public_base: String,
        /// Single payout denom (only `uterp`), or `split` for the catalog's TERP+THIOL window split.
        #[arg(long, default_value = "uterp")]
        payout_denom: String,
        /// USD per TERP for the single-denom conversion (round-2 catalog baseline, Prop 61).
        #[arg(long, default_value = "0.05172")]
        terp_usd: String,
    },
    /// Seed: upload docs + manifest to static/terp.network/bounties/seed/<pv>/ via an existing mc alias.
    /// Upload needs --execute, BOUNTY_UPLOAD_EXECUTE=1 and BOUNTY_MC_ALIAS.
    UploadSeed {
        #[arg(long)]
        params_version: String,
        /// In preview, also `mc stat` each key (needs BOUNTY_MC_ALIAS).
        #[arg(long)]
        check_remote: bool,
    },
    /// Seed: batched create_bounty via nested authz. Preview writes every batch;
    /// --execute signs exactly one batch (--batch N) per phone approval.
    CreateBounties {
        #[arg(long)]
        params_version: String,
        #[arg(long, default_value_t = 10)]
        batch_size: usize,
        /// 1-based batch to sign with --execute.
        #[arg(long)]
        batch: Option<usize>,
        /// Grant calls / funds already consumed (read with terpd q authz grants).
        #[arg(long, default_value_t = 0)]
        spent_calls: u64,
        #[arg(long, default_value_t = 0)]
        spent_uterp: u128,
        #[arg(long, default_value_t = 0)]
        spent_uthiol: u128,
    },
    /// Curate bounties as FIAB core through the live grant: approve | reject | cancel.
    /// Reads each bounty's status from chain (LCD, read-only) first; preview by default.
    /// Read-only: open the same cw-orch gRPC channel --execute uses (GetNodeInfo + chain-id check). Signs nothing.
    GrpcCheck,
    Curate {
        /// approve | reject | cancel
        action: String,
        /// Bounty ids, comma separated.
        #[arg(long, value_delimiter = ',')]
        ids: Vec<u64>,
        /// Every bounty currently Pending (chain query, or the assumed seed with --assume-fresh-seed).
        #[arg(long)]
        all_pending: bool,
        /// Only allow bounties whose bounty_ref is in this seed manifest (params_version).
        #[arg(long)]
        manifest: Option<String>,
        /// PREVIEW ONLY: skip the chain read and assume a fresh escrow holding this seed as ids 1..N, all Pending.
        #[arg(long)]
        assume_fresh_seed: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long, default_value_t = 20)]
        batch_size: usize,
        /// 1-based batch to sign with --execute.
        #[arg(long)]
        batch: Option<usize>,
        /// Grant calls already consumed (terpd q authz grants).
        #[arg(long, default_value_t = 0)]
        spent_calls: u64,
    },
}

struct Plan {
    step: String,
    signer: String,
    note: String,
    /// What the cw-orch-kms session is allowed to do.
    session: Value,
    msgs: Vec<(Any, Value)>,
    kind: SessionKind,
}

enum SessionKind {
    Store(Vec<u8>),
    Execute { contract: String, key: String },
    NestedExec,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let cfg: Value = serde_json::from_slice(
        &std::fs::read(cli.pack.join("config.json")).context("read <pack>/config.json")?,
    )?;
    let s = |k: &str| -> Result<String> {
        cfg[k]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("config.{k} missing"))
    };
    let core = s("fiab_core")?;
    let module = s("fiab_proposal_module")?;
    let member = s("fiab_sole_member")?;
    let predicted = predicted_escrow(&cfg)?;
    if predicted != VERIFIED_ESCROW {
        bail!("predicted escrow {predicted} != verified {VERIFIED_ESCROW}");
    }
    let escrow = cli.escrow.clone().unwrap_or(predicted.clone());

    match &cli.step {
        Step::ExportSeed {
            catalog,
            horizon,
            deadline,
            public_base,
            payout_denom,
            terp_usd,
        } => {
            let payout = if payout_denom == "split" {
                seed::Payout::Split
            } else {
                seed::Payout::Single {
                    denom: payout_denom.clone(),
                    price_usd: terp_usd.clone(),
                }
            };
            return seed::export_seed(&cli.pack, catalog, horizon, deadline, public_base, &payout);
        }
        Step::UploadSeed {
            params_version,
            check_remote,
        } => {
            return seed::upload_seed(&cli.pack, params_version, cli.execute, *check_remote);
        }
        Step::CreateBounties {
            params_version,
            batch_size,
            batch,
            spent_calls,
            spent_uterp,
            spent_uthiol,
        } => {
            let spent = std::collections::BTreeMap::from([
                ("uterp".to_string(), *spent_uterp),
                ("uthiol".to_string(), *spent_uthiol),
            ]);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64;
            let plans = seed::create_bounties_plans(
                &cfg,
                &core,
                &member,
                &escrow,
                &cli.pack,
                params_version,
                *batch_size,
                *spent_calls,
                &spent,
                now,
            )?;
            for p in &plans {
                print_and_write(&cli.pack, p)?;
            }
            println!("{} batch(es)", plans.len());
            if !cli.execute {
                println!(
                    "mode: PREVIEW (nothing signed). Sign one batch with --batch N --execute and FIAB_ESCROW_EXECUTE=1."
                );
                return Ok(());
            }
            let n = batch
                .ok_or_else(|| anyhow!("--execute needs --batch N (one batch per approval)"))?;
            cw_orch_kms::progress::set_batch(n as u32, plans.len() as u32);
            let plan = plans
                .into_iter()
                .nth(n.wrapping_sub(1))
                .ok_or_else(|| anyhow!("no batch {n}"))?;
            return execute(&cli, &cfg, plan);
        }
        Step::GrpcCheck => {
            let url = cli
                .grpc_url
                .clone()
                .ok_or_else(|| anyhow!("grpc-check needs --grpc-url or GRPC_URL"))?;
            let chain_id = cfg["chain_id"].as_str().unwrap().to_string();
            let rt = cw_orch::tokio::runtime::Runtime::new()?;
            match rt.block_on(cw_orch::daemon::GrpcChannel::connect(&[url], &chain_id)) {
                Ok(_) => println!("grpc-check OK: cw-orch channel up, node network == {chain_id}"),
                Err(e) => bail!("grpc-check FAIL: {e} ({e:?})"),
            }
            return Ok(());
        }
        Step::Curate {
            action,
            ids,
            all_pending,
            manifest,
            assume_fresh_seed,
            reason,
            batch_size,
            batch,
            spent_calls,
        } => {
            let action = curate::Action::parse(action)?;
            if ids.is_empty() && !*all_pending {
                bail!("curate needs --ids or --all-pending");
            }
            let (states, source) = match assume_fresh_seed {
                Some(pv) => {
                    if cli.execute {
                        bail!(
                            "--assume-fresh-seed is preview-only; --execute reads status from chain"
                        );
                    }
                    (
                        curate::assumed_fresh(&cli.pack, pv, ids, *all_pending)?,
                        format!(
                            "ASSUMED fresh escrow, seed {pv} created in manifest order as ids 1..N, all pending (not read from chain)"
                        ),
                    )
                }
                None => {
                    let lcd = cfg["lcd"]
                        .as_str()
                        .ok_or_else(|| anyhow!("config.lcd missing"))?;
                    (
                        curate::chain_states(lcd, &escrow, ids, *all_pending)?,
                        format!("chain: LCD smart query on {escrow}"),
                    )
                }
            };
            let refs: Option<Vec<String>> = match manifest.as_ref().or(assume_fresh_seed.as_ref()) {
                Some(pv) => Some(
                    seed::load_manifest(&cli.pack, pv)?.0["bounties"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|e| e["id"].as_str().map(str::to_string))
                        .collect(),
                ),
                None => None,
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64;
            curate::validate(action, &states, refs.as_deref(), now)?;
            let plans = curate::plans(
                &cfg,
                &core,
                &member,
                &escrow,
                action,
                reason,
                &states,
                *batch_size,
                *spent_calls,
                &source,
            )?;
            for p in &plans {
                print_and_write(&cli.pack, p)?;
            }
            println!(
                "curate {}: {} bounties in {} batch(es); status source: {source}",
                action.key(),
                states.len(),
                plans.len()
            );
            if !cli.execute {
                println!(
                    "mode: PREVIEW (nothing signed). Sign one batch with --execute ... --batch N and FIAB_ESCROW_EXECUTE=1, GRPC_URL set."
                );
                return Ok(());
            }
            let n = batch
                .ok_or_else(|| anyhow!("--execute needs --batch N (one batch per approval)"))?;
            cw_orch_kms::progress::set_batch(n as u32, plans.len() as u32);
            let plan = plans
                .into_iter()
                .nth(n.wrapping_sub(1))
                .ok_or_else(|| anyhow!("no batch {n}"))?;
            return execute(&cli, &cfg, plan);
        }
        _ => {}
    }

    let plan = match &cli.step {
        Step::Store { wasm, uploader } => {
            let bytes = std::fs::read(wasm).with_context(|| format!("read {}", wasm.display()))?;
            let sum = hexenc(cw_orch_kms::wasm_checksum(&bytes).map_err(|e| anyhow!(e))?);
            if sum != s("wasm_sha256")? {
                bail!("wasm sha256 {sum} != config {}", s("wasm_sha256")?);
            }
            let msg = MsgStoreCode {
                sender: uploader.clone(),
                wasm_byte_code: bytes.clone(),
                instantiate_permission: Some(AccessConfig {
                    permission: ACCESS_ANY_OF_ADDRESSES,
                    addresses: vec![core.clone()],
                }),
            };
            let view = json!({"@type": T_STORE, "sender": uploader, "wasm_byte_code":
                format!("<{} bytes, sha256 {sum}>", bytes.len()),
                "instantiate_permission": {"permission": "AnyOfAddresses", "addresses": [core]}});
            Plan {
                step: "00-store".into(),
                signer: uploader.clone(),
                note: "uploader pays gas; only FIAB core may instantiate the code".into(),
                session: json!({"stores": [sum], "type_urls": [T_STORE]}),
                msgs: vec![(any(T_STORE, &msg), view)],
                kind: SessionKind::Store(bytes),
            }
        }
        Step::ProposeInstantiate { code_id } => {
            let p = instantiate_proposal(&cfg, *code_id, &predicted)?;
            propose_plan("01-propose-instantiate", &module, &member, p)
        }
        Step::ProposeGrant => {
            let (bytes, _) = grant_bytes(&cfg, &escrow)?;
            let sum = sha256_hex(&bytes)?;
            if cli.escrow.is_none() && sum != VERIFIED_GRANT_SHA256 {
                bail!("MsgGrant sha256 {sum} != verified {VERIFIED_GRANT_SHA256}");
            }
            let p = grant_proposal(&cfg, &escrow, &bytes)?;
            let mut plan = propose_plan("02-propose-grant", &module, &member, p);
            plan.note = format!("stargate MsgGrant value {} bytes sha256 {sum}", bytes.len());
            plan
        }
        Step::ProposeRevoke => {
            let p = revoke_proposal(&cfg)?;
            propose_plan("02b-propose-revoke", &module, &member, p)
        }
        Step::ProposeFile { file } => {
            let mut p: Value = serde_json::from_slice(&std::fs::read(file)?)?;
            if p.get("_TEMPLATE").is_some() {
                bail!(
                    "{} is a template; fill it with gen_04_lifecycle.py first",
                    file.display()
                );
            }
            if let Some(o) = p.as_object_mut() {
                o.retain(|k, _| !k.starts_with('_'));
            }
            if p.get("propose").is_none() {
                bail!("{} has no top-level propose", file.display());
            }
            let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("04");
            propose_plan(&format!("04-{name}"), &module, &member, p)
        }
        Step::Vote { proposal_id, vote } => exec_plan(
            &format!("vote-{proposal_id}"),
            &module,
            &member,
            "vote",
            json!({"vote": {"proposal_id": proposal_id, "vote": vote}}),
        ),
        Step::Execute { proposal_id } => exec_plan(
            &format!("execute-{proposal_id}"),
            &module,
            &member,
            "execute",
            json!({"execute": {"proposal_id": proposal_id}}),
        ),
        Step::CreateBounty { inner } => create_bounty_plan(&cfg, &core, &member, &escrow, inner)?,
        Step::ExportSeed { .. }
        | Step::UploadSeed { .. }
        | Step::CreateBounties { .. }
        | Step::Curate { .. }
        | Step::GrpcCheck => unreachable!(),
    };

    print_and_write(&cli.pack, &plan)?;
    if !cli.execute {
        println!(
            "mode: PREVIEW (nothing signed). Re-run with --execute and FIAB_ESCROW_EXECUTE=1 to sign via cw-orch-kms."
        );
        return Ok(());
    }
    execute(&cli, &cfg, plan)
}

fn propose_plan(step: &str, module: &str, member: &str, p: Value) -> Plan {
    let mut plan = exec_plan(step, module, member, "propose", p);
    plan.note = "FIAB member submits to the FIAB proposal module".into();
    plan
}

fn exec_plan(step: &str, module: &str, member: &str, key: &str, msg: Value) -> Plan {
    let m = MsgExecuteContract {
        sender: member.into(),
        contract: module.into(),
        msg: serde_json::to_vec(&msg).expect("json"),
        funds: vec![],
    };
    Plan {
        step: step.into(),
        signer: member.into(),
        note: format!("FIAB member calls {key} on the FIAB proposal module"),
        session: json!({"executes": [{"address": module, "keys": [key], "calls": 1, "funds": []}],
                        "type_urls": [T_EXEC]}),
        msgs: vec![(
            any(T_EXEC, &m),
            json!({"@type": T_EXEC, "sender": member,
                    "contract": module, "msg": msg, "funds": []}),
        )],
        kind: SessionKind::Execute {
            contract: module.into(),
            key: key.into(),
        },
    }
}

fn create_bounty_plan(
    cfg: &Value,
    core: &str,
    member: &str,
    escrow: &str,
    path: &Path,
) -> Result<Plan> {
    let tx: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let msgs = tx["body"]["messages"]
        .as_array()
        .ok_or_else(|| anyhow!("no body.messages"))?;
    if msgs.len() != 1 {
        bail!("want exactly one inner message");
    }
    let m = &msgs[0];
    if m["@type"] != T_EXEC || m["sender"] != core || m["contract"] != escrow {
        bail!("inner must be MsgExecuteContract sender=FIAB core contract={escrow}");
    }
    let cb = &m["msg"]["create_bounty"];
    if cb.is_null() || m["msg"].as_object().map(|o| o.len()) != Some(1) {
        bail!("inner msg must be exactly create_bounty");
    }
    if m["funds"] != json!([cb["reward"]]) {
        bail!("funds must equal [reward]");
    }
    if cb["details_hash"]
        .as_str()
        .map(|h| h.chars().all(|c| c == '0'))
        == Some(true)
        && std::env::var("ALLOW_EXAMPLE").ok().as_deref() != Some("1")
    {
        bail!("details_hash is all zero (example); refusing");
    }
    let reward = &cb["reward"];
    let denom = reward["denom"].as_str().unwrap_or_default().to_string();
    let amount = reward["amount"].as_str().unwrap_or_default().to_string();
    let inner = MsgExecuteContract {
        sender: core.into(),
        contract: escrow.into(),
        msg: serde_json::to_vec(&m["msg"])?,
        funds: vec![Coin {
            denom: denom.clone(),
            amount: amount.clone(),
        }],
    };
    let exec = MsgExec {
        grantee: member.into(),
        msgs: vec![any(T_EXEC, &inner)],
    };
    let _ = cfg;
    Ok(Plan {
        step: format!("03-create-bounty-{}", cb["bounty_ref"].as_str().unwrap_or("x")),
        signer: member.into(),
        note: "nested: MsgExec(session key){MsgExec(FIAB member){MsgExecuteContract(FIAB core)}}; spends the 02 grant".into(),
        session: json!({"nested_exec": true, "type_urls": [T_AUTHZ_EXEC]}),
        msgs: vec![(any(T_AUTHZ_EXEC, &exec), json!({"@type": T_AUTHZ_EXEC, "grantee": member,
            "msgs": [m]}))],
        kind: SessionKind::NestedExec,
    })
}

fn instantiate_proposal(cfg: &Value, code_id: u64, predicted: &str) -> Result<Value> {
    if cfg["register_as"] != "proposal_module" {
        bail!("config.register_as must be proposal_module (confirmed)");
    }
    let core = cfg["fiab_core"].as_str().unwrap();
    let m = &cfg["instantiate_msg"];
    let salt = Binary::from(cfg["salt_utf8"].as_str().unwrap().as_bytes()).to_base64();
    let inner = json!({"update_proposal_modules": {"to_add": [{
        "code_id": code_id, "msg": b64json(m)?, "admin": {"core_module": {}},
        "funds": [], "label": cfg["label"], "salt": salt}], "to_disable": []}});
    let desc = format!(
        "Instantiates terp-bounty-escrow (code {code_id}, sha256 {}) registered as a FIAB proposal module (DAO DAO update_proposal_modules, admin = FIAB core). Predicted address (instantiate2, salt '{}'): {predicted}. FIAB core is owner, approver and DAO; only FIAB (allowlist) can create bounties. No funds move in this proposal. Per-bounty funding happens later via a capped authz grant (separate proposal).",
        cfg["wasm_sha256"].as_str().unwrap(),
        cfg["salt_utf8"].as_str().unwrap()
    );
    Ok(
        json!({"propose": {"title": "Add the Terp bounty escrow to FIAB-SUBDAO", "description": desc,
        "msgs": [{"wasm": {"execute": {"contract_addr": core, "msg": b64json(&inner)?, "funds": []}}}]}}),
    )
}

fn grant_bytes(cfg: &Value, escrow: &str) -> Result<(Vec<u8>, String)> {
    let g = &cfg["grant"];
    let coins = g["amounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| Coin {
            denom: c["denom"].as_str().unwrap().into(),
            amount: c["amount"].as_str().unwrap().into(),
        })
        .collect::<Vec<_>>();
    let mut sorted = coins.iter().map(|c| c.denom.clone()).collect::<Vec<_>>();
    sorted.sort();
    if sorted != coins.iter().map(|c| c.denom.clone()).collect::<Vec<_>>() {
        bail!("CombinedLimit amounts must be sorted by denom");
    }
    let keys = g["accepted_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap().to_string())
        .collect();
    let auth = ContractExecutionAuthorization {
        grants: vec![ContractGrant {
            contract: escrow.into(),
            limit: Some(any(
                T_COMBINED,
                &CombinedLimit {
                    calls_remaining: g["calls_remaining"].as_u64().unwrap(),
                    amounts: coins,
                },
            )),
            filter: Some(any(T_KEYS, &AcceptedMessageKeysFilter { keys })),
        }],
    };
    let exp = g["expiration_rfc3339"].as_str().unwrap();
    let msg = MsgGrant {
        granter: cfg["fiab_core"].as_str().unwrap().into(),
        grantee: g["grantee"].as_str().unwrap().into(),
        grant: Some(Grant {
            authorization: Some(any(T_CEA, &auth)),
            expiration: Some(Timestamp {
                seconds: rfc3339_utc(exp)?,
                nanos: 0,
            }),
        }),
    };
    Ok((msg.encode_to_vec(), exp.to_string()))
}

fn grant_proposal(cfg: &Value, escrow: &str, bytes: &[u8]) -> Result<Value> {
    let g = &cfg["grant"];
    let amounts = g["amounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{}{}",
                c["amount"].as_str().unwrap(),
                c["denom"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let grantee = g["grantee"].as_str().unwrap();
    let desc = format!(
        "Grants {grantee} (FIAB's sole member) authz to execute ONLY create_bounty on the FIAB bounty escrow {escrow} on FIAB's behalf: max {} calls and max {amounts} total funds, expiring {}. Bounties still start Pending and need a FIAB proposal to approve. Revocable at any time by a MsgRevoke proposal (02b).",
        g["calls_remaining"],
        g["expiration_rfc3339"].as_str().unwrap()
    );
    Ok(
        json!({"propose": {"title": "FIAB: capped authz for bounty creation on the escrow", "description": desc,
        "msgs": [{"stargate": {"type_url": T_GRANT, "value": Binary::from(bytes).to_base64()}}]}}),
    )
}

fn revoke_proposal(cfg: &Value) -> Result<Value> {
    let grantee = cfg["grant"]["grantee"].as_str().unwrap();
    let msg = MsgRevoke {
        granter: cfg["fiab_core"].as_str().unwrap().into(),
        grantee: grantee.into(),
        msg_type_url: T_EXEC.into(),
    };
    let desc = format!(
        "Revokes the authz grant from FIAB core to {grantee} for {T_EXEC}. Existing bounties and escrowed funds are unaffected; open bounties can still be cancelled/rejected by FIAB proposals (refunds go to FIAB)."
    );
    Ok(
        json!({"propose": {"title": "FIAB: revoke bounty-creation authz grant", "description": desc,
        "msgs": [{"stargate": {"type_url": T_REVOKE, "value": Binary::from(msg.encode_to_vec()).to_base64()}}]}}),
    )
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
        println!(
            "message {}:\n{}",
            a.type_url,
            serde_json::to_string_pretty(v)?
        );
    }
    println!("wrote {}", path.display());
    Ok(())
}

fn execute(cli: &Cli, cfg: &Value, plan: Plan) -> Result<()> {
    use cw_orch::daemon::{DaemonBuilder, TxSender};
    use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
    if std::env::var("FIAB_ESCROW_EXECUTE").ok().as_deref() != Some("1") {
        bail!("refusing to sign: --execute also needs FIAB_ESCROW_EXECUTE=1");
    }
    let chain_id = cfg["chain_id"].as_str().unwrap().to_string();
    let chain = ChainInfoOwned {
        chain_id: chain_id.clone(),
        gas_denom: "uthiol".into(),
        gas_price: 0.05,
        grpc_urls: vec![cli.grpc_url.clone().ok_or_else(|| {
            anyhow!("--execute needs --grpc-url or GRPC_URL (a morocco-1 gRPC endpoint)")
        })?],
        rpc_urls: vec![],
        lcd_url: cfg["lcd"].as_str().map(str::to_string),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: "terp".into(),
            coin_type: 118,
        },
        kind: ChainKind::Mainnet,
    };
    // SAFETY: set before cw-orch reads it; nothing else races on the env.
    unsafe { std::env::set_var("CW_ORCH_WALLET_BALANCE_ASSERTION", "false") };
    let granter = if cli.granter.is_empty() {
        println!("sign the cw-orch-granter challenge at http://127.0.0.1:8741");
        cw_orch_kms::capture_granter_for_chain_opts_sync(
            "127.0.0.1:8741".parse()?,
            cli.account_index,
            "fiab-bounty-escrow",
            &chain_id,
            cli.ttl_secs,
            Some(&plan.signer),
        )
        .map_err(|e| anyhow!(e))?
    } else {
        cw_orch_kms::set_approval_chain_id(&chain_id).map_err(|e| anyhow!(e))?;
        cli.granter.clone()
    };
    if granter != plan.signer {
        bail!(
            "granter {granter} is not this step's signer {}",
            plan.signer
        );
    }
    let mut builder = DaemonBuilder::new(chain);
    builder.load_network(false);
    let mut session = cw_orch_kms::ApprovalSession::open(&granter);
    session = match &plan.kind {
        SessionKind::Store(wasm) => session.allow_store(wasm).map_err(|e| anyhow!(e))?,
        SessionKind::Execute { contract, key } => session
            .allow_execute(cw_orch_kms::ScopedCall {
                address: contract.clone(),
                keys: vec![key.clone()],
                calls: 1,
                funds: vec![],
            })
            .map_err(|e| anyhow!(e))?,
        SessionKind::NestedExec => session.allow_nested_exec(),
    };
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cw_orch_kms::planned_spend_limit(4);
    let actions = vec![cw_orch_kms::SigningRequest::new("session-key").body(plan.step.clone())];
    let run = cw_orch_kms::enable_session(&builder, session, actions).map_err(|e| anyhow!(e))?;
    let daemon = run.daemon.clone();
    println!("session key (grantee): {}", run.grantee());
    let anys: Vec<Any> = plan.msgs.iter().map(|(a, _)| a.clone()).collect();
    let memo = format!("fiab-bounty-escrow {}", plan.step);
    let result = (|| -> Result<()> {
        // cw-orch wraps MsgExec(session) inside commit_tx_any but NOT inside simulate. Without this the
        // simulated tx's signer is the member (inner MsgExec grantee): "expected <member seq>, got 0".
        let sim = session_wrap(run.grantee(), anys.clone());
        use cw_orch_kms::progress::Status;
        run.step("simulate", Status::Running, None, None);
        let (gas, fee) = match daemon
            .rt_handle
            .block_on(daemon.sender().simulate(sim, Some(&memo)))
        {
            Ok(v) => v,
            Err(e) => {
                run.step("simulate", Status::Error, None, Some(&e.to_string()));
                return Err(e.into());
            }
        };
        println!("simulate OK gas={gas} fee={fee}");
        run.step(
            "simulate",
            Status::Confirmed,
            None,
            Some(&format!("gas {gas}, fee {fee}")),
        );
        run.step("broadcast", Status::Running, None, None);
        let resp = match daemon
            .rt_handle
            .block_on(daemon.sender().commit_tx_any(anys, Some(&memo)))
        {
            Ok(r) => r,
            Err(e) => {
                run.step("broadcast", Status::Error, None, Some(&e.to_string()));
                return Err(e.into());
            }
        };
        println!("BROADCAST txhash: {}", resp.txhash);
        if resp.code == 0 {
            run.step("broadcast", Status::Confirmed, Some(&resp.txhash), None);
        } else {
            run.step(
                "broadcast",
                Status::Error,
                Some(&resp.txhash),
                Some(&format!("code {}", resp.code)),
            );
        }
        Ok(())
    })();
    run.close().map_err(|e| anyhow!(e))?;
    println!("session grant closed");
    result
}

// ---- helpers ----
/// The outer MsgExec(session) that commit_tx_any adds when authz_granter is set; simulate needs it explicitly.
fn session_wrap(session_grantee: &str, msgs: Vec<Any>) -> Vec<Any> {
    use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec;
    vec![any(
        T_AUTHZ_EXEC,
        &MsgExec {
            grantee: session_grantee.into(),
            msgs,
        },
    )]
}

#[cfg(test)]
mod exec_tests {
    use super::*;
    use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec;
    #[test]
    fn simulate_body_is_session_wrapped() {
        let inner = any(
            T_AUTHZ_EXEC,
            &MsgExec {
                grantee: "member".into(),
                msgs: vec![],
            },
        );
        let w = session_wrap("session", vec![inner.clone()]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].type_url, T_AUTHZ_EXEC);
        let outer = MsgExec::decode(w[0].value.as_slice()).unwrap();
        assert_eq!(outer.grantee, "session"); // first signer = session key, not the member
        assert_eq!(outer.msgs, vec![inner]);
    }
}
fn any<M: Message>(type_url: &str, m: &M) -> Any {
    Any {
        type_url: type_url.into(),
        value: m.encode_to_vec(),
    }
}

fn b64json(v: &Value) -> Result<String> {
    Ok(Binary::from(serde_json::to_vec(v)?).to_base64())
}

fn sha256_hex(bytes: &[u8]) -> Result<String> {
    // wasm_checksum is plain SHA-256 unless the bytes start with the gzip magic.
    if bytes.starts_with(&[0x1f, 0x8b]) {
        bail!("unexpected gzip magic");
    }
    Ok(hexenc(
        cw_orch_kms::wasm_checksum(bytes).map_err(|e| anyhow!(e))?,
    ))
}

fn predicted_escrow(cfg: &Value) -> Result<String> {
    let checksum = hexdec(cfg["wasm_sha256"].as_str().unwrap())?;
    let creator = bech32_data(cfg["fiab_core"].as_str().unwrap())?;
    cw_orch_kms::instantiate2_bech32(
        "terp",
        &checksum,
        &creator,
        cfg["salt_utf8"].as_str().unwrap().as_bytes(),
    )
    .map_err(|e| anyhow!(e))
}

/// Minimal bech32 (BIP-173) decode to data bytes, checksum verified.
fn bech32_data(addr: &str) -> Result<Vec<u8>> {
    const CS: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let pos = addr
        .rfind('1')
        .ok_or_else(|| anyhow!("bech32: no separator"))?;
    let (hrp, data) = (&addr[..pos], &addr[pos + 1..]);
    let vals: Vec<u8> = data
        .chars()
        .map(|c| CS.find(c).map(|i| i as u8))
        .collect::<Option<_>>()
        .ok_or_else(|| anyhow!("bech32: bad char"))?;
    let mut chk: Vec<u8> = hrp.bytes().map(|b| b >> 5).collect();
    chk.push(0);
    chk.extend(hrp.bytes().map(|b| b & 31));
    chk.extend(&vals);
    let gens = [
        0x3b6a57b2u32,
        0x26508e6d,
        0x1ea119fa,
        0x3d4233dd,
        0x2a1462b3,
    ];
    let mut p = 1u32;
    for v in chk {
        let b = p >> 25;
        p = ((p & 0x1ffffff) << 5) ^ v as u32;
        for (i, g) in gens.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                p ^= g;
            }
        }
    }
    if p != 1 {
        bail!("bech32: bad checksum");
    }
    let (mut acc, mut bits, mut out) = (0u32, 0u32, vec![]);
    for v in &vals[..vals.len() - 6] {
        acc = (acc << 5) | *v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// "YYYY-MM-DDTHH:MM:SSZ" -> unix seconds (UTC only).
fn rfc3339_utc(s: &str) -> Result<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[19] != b'Z' {
        bail!("expiration must be YYYY-MM-DDTHH:MM:SSZ");
    }
    let n = |r: std::ops::Range<usize>| -> Result<i64> { Ok(s[r].parse()?) };
    let (y, m, d, hh, mm, ss) = (
        n(0..4)?,
        n(5..7)?,
        n(8..10)?,
        n(11..13)?,
        n(14..16)?,
        n(17..19)?,
    );
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Ok(days * 86400 + hh * 3600 + mm * 60 + ss)
}

fn hexenc<T: AsRef<[u8]>>(b: T) -> String {
    b.as_ref().iter().map(|x| format!("{:02x}", x)).collect()
}
fn hexdec(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("odd hex length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!(e)))
        .collect()
}
