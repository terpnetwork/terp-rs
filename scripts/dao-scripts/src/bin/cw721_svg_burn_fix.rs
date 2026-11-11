//! cw721-svg burn-then-mint fix on morocco-1, through cw-orch-kms.
//!
//! The permissionless fractal (cw721-svg code 98) numbered each new token by the
//! live token count. Burning token 12 dropped the count to 17, so every mint
//! retried the taken id 17. The fixed cw721-svg keeps an id counter that only goes
//! up. Three steps, run in order, each its own cold-key-approved session:
//!
//!   store           MsgStoreCode of the fixed wasm (session scoped to its checksum)
//!   migrate         MsgMigrateContract collection -> --code-id, {"next_token_id": N}
//!   update-code-id  svg-minter `update_code_id { code_id }` (cw-ownable owner only)
//!
//! The new code id is only known once `store` lands, so `migrate` and
//! `update-code-id` take it as `--code-id`. Both refuse unless that code's on-chain
//! checksum equals sha256(--wasm).
//!
//! Dry-run by default: read-only checks against the chain (wasm admin / factory
//! owner = --granter, current code, checksum), the planned message printed and
//! written to `--out/plan-<step>.json`. Nothing is signed.
//! `--broadcast` also needs `TERP_SCRIPT_BROADCAST=1` and `--chain-id morocco-1`.
//!
//! ```sh
//! cargo run -p dao-scripts --bin cw721_svg_burn_fix -- store --wasm <fixed.wasm>
//! cargo run -p dao-scripts --bin cw721_svg_burn_fix -- migrate --wasm <fixed.wasm> --code-id <N>
//! cargo run -p dao-scripts --bin cw721_svg_burn_fix -- update-code-id --wasm <fixed.wasm> --code-id <N>
//! ```
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use cw_orch::daemon::{Daemon, DaemonBuilder, TxSender};
use cw_orch::environment::WasmQuerier;
use cw_orch::prelude::*;
use cw_orch_kms::{
    ApprovalSession, ScopedCall, SigningRequest, enable_session, planned_spend_limit,
};
use dao_scripts::kms_gate;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// permissionless-fractal, cw721-svg code 98, created by the svg-minter factory.
const FRACTAL: &str = "terp15anc99l7lvk2v2pzjusnmylr3lw3xnpxg89xv8uh0fj6pwrumrksqwzs6x";
/// svg-minter factory (code 99). `update_code_id` is owner-only.
const SVG_MINTER: &str = "terp1wg5z9rcwpw5kvrl4g78ees2l9ejptrhrx4t2k56nq6t27uly87zssujm80";
/// Fractal wasm admin and svg-minter owner.
const ADMIN: &str = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052";
const OLD_CODE: u64 = 98;

#[derive(Parser, Debug)]
#[command(name = "cw721_svg_burn_fix")]
struct Cli {
    /// store | migrate | update-code-id
    step: String,
    /// Fixed cw721-svg wasm (optimized, raw). Its sha256 pins the store grant and
    /// is checked against --code-id for the later steps.
    #[arg(long)]
    wasm: PathBuf,
    /// Code id the store tx returned (migrate, update-code-id).
    #[arg(long)]
    code_id: Option<u64>,
    #[arg(long, default_value_t = 18)]
    next_token_id: u64,
    #[arg(long, default_value = FRACTAL)]
    collection: String,
    #[arg(long, default_value = SVG_MINTER)]
    factory: String,
    /// Signing granter (cold key). Must be the collection admin / factory owner.
    #[arg(long, default_value = ADMIN)]
    granter: String,
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long)]
    grpc: Option<String>,
    #[arg(long)]
    rpc: Option<String>,
    #[arg(long)]
    lcd: Option<String>,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 600)]
    ttl_secs: u64,
    #[arg(long, default_value = "out/cw721-svg-burn-fix")]
    out: String,
    #[arg(long)]
    broadcast: bool,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

enum Plan {
    Store(Vec<u8>),
    Migrate { code_id: u64, msg: Value },
    Execute { msg: Value },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut info = kms_gate::morocco_info();
    if let Some(g) = &cli.grpc {
        info.grpc_urls = vec![g.clone()];
    }
    let info = kms_gate::with_http_fallback(info, cli.rpc.as_deref(), cli.lcd.as_deref());
    let wasm = std::fs::read(&cli.wasm).with_context(|| format!("read {:?}", cli.wasm))?;
    if wasm.starts_with(&[0x1f, 0x8b]) {
        bail!("--wasm must be raw wasm: MsgStoreCode under MsgExec cannot take gzip on terpd");
    }
    if !wasm.starts_with(b"\0asm") {
        bail!("--wasm is not a wasm module");
    }
    let sum = hex(&Sha256::digest(&wasm));
    println!("wasm {:?} {} bytes sha256 {sum}", cli.wasm, wasm.len());

    let q = kms_gate::query_only(&info)?;
    let wq = q.wasm_querier();
    let granter = cli.granter.clone();

    let need_code = |step: &str| -> Result<u64> {
        let id = cli
            .code_id
            .ok_or_else(|| anyhow!("{step} needs --code-id (from the store tx)"))?;
        let have = hex(wq
            .code_id_hash(id)
            .map_err(|e| anyhow!("code {id}: {e}"))?
            .as_slice());
        if have != sum {
            bail!("code {id} checksum {have} != sha256(--wasm) {sum}; refusing");
        }
        println!("  ok code {id} checksum matches --wasm");
        Ok(id)
    };

    let (plan, note, msg_any) = match cli.step.as_str() {
        "store" => {
            let any = kms_gate::any(
                "/cosmwasm.wasm.v1.MsgStoreCode",
                &cosmos_sdk_proto::cosmwasm::wasm::v1::MsgStoreCode {
                    sender: granter.clone(),
                    wasm_byte_code: wasm.clone(),
                    // Explicit, never nil (x/wasm CodeGrant.Accept derefs it).
                    instantiate_permission: Some(
                        cosmos_sdk_proto::cosmwasm::wasm::v1::AccessConfig {
                            permission:
                                cosmos_sdk_proto::cosmwasm::wasm::v1::AccessType::Everybody
                                    as i32,
                            addresses: vec![],
                        },
                    ),
                },
            );
            let note = json!({"msg": "/cosmwasm.wasm.v1.MsgStoreCode", "sender": granter,
                "wasm_sha256": sum, "wasm_bytes": wasm.len(), "instantiate_permission": "Everybody"});
            (Plan::Store(wasm.clone()), note, any)
        }
        "migrate" => {
            let code_id = need_code("migrate")?;
            let addr = Addr::unchecked(&cli.collection);
            let ci = wq
                .contract_info(&addr)
                .map_err(|e| anyhow!("contract_info {addr}: {e}"))?;
            println!("  collection {addr} code {} admin {:?}", ci.code_id, ci.admin);
            if ci.admin.as_ref().map(|a| a.as_str()) != Some(granter.as_str()) {
                bail!("{addr} wasm admin is {:?}, not --granter {granter}", ci.admin);
            }
            if ci.code_id != OLD_CODE && ci.code_id != code_id {
                bail!("{addr} is on code {}, expected {OLD_CODE}", ci.code_id);
            }
            let n: Value = wq
                .smart_query(&addr, &json!({"num_tokens": {}}))
                .map_err(|e| anyhow!("num_tokens: {e}"))?;
            println!("  num_tokens {}", n["count"]);
            let exists = |id: u64| -> bool {
                wq.smart_query::<_, Value>(&addr, &json!({"owner_of": {"token_id": id.to_string()}}))
                    .is_ok()
            };
            if cli.next_token_id == 0 || !exists(cli.next_token_id - 1) {
                println!(
                    "  warn: token {} does not exist; check --next-token-id",
                    cli.next_token_id.saturating_sub(1)
                );
            }
            if exists(cli.next_token_id) {
                bail!("token {} already exists; --next-token-id too low", cli.next_token_id);
            }
            let msg = json!({"next_token_id": cli.next_token_id});
            let any = kms_gate::any(
                "/cosmwasm.wasm.v1.MsgMigrateContract",
                &cosmos_sdk_proto::cosmwasm::wasm::v1::MsgMigrateContract {
                    sender: granter.clone(),
                    contract: cli.collection.clone(),
                    code_id,
                    msg: serde_json::to_vec(&msg)?,
                },
            );
            let note = json!({"msg": "/cosmwasm.wasm.v1.MsgMigrateContract", "sender": granter,
                "contract": cli.collection, "code_id": code_id, "migrate_msg": msg});
            (Plan::Migrate { code_id, msg }, note, any)
        }
        "update-code-id" => {
            let code_id = need_code("update-code-id")?;
            let addr = Addr::unchecked(&cli.factory);
            let own: Value = wq
                .smart_query(&addr, &json!({"ownership": {}}))
                .map_err(|e| anyhow!("ownership: {e}"))?;
            println!("  factory {addr} owner {}", own["owner"]);
            if own["owner"].as_str() != Some(granter.as_str()) {
                bail!("{addr} owner is {}, not --granter {granter}", own["owner"]);
            }
            let cur: Value = wq
                .smart_query(&addr, &json!({"code_id": {}}))
                .map_err(|e| anyhow!("code_id: {e}"))?;
            println!("  factory svg_code_id now {cur}");
            let msg = json!({"update_code_id": {"code_id": code_id}});
            let any = kms_gate::execute_any(&granter, &cli.factory, &msg)?;
            let note = json!({"msg": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": granter,
                "contract": cli.factory, "execute_msg": msg});
            (Plan::Execute { msg }, note, any)
        }
        s => bail!("unknown step {s:?}: store | migrate | update-code-id"),
    };

    let plan_path = format!("{}/plan-{}.json", cli.out, cli.step);
    kms_gate::write_plan(
        &plan_path,
        &json!({"chain_id": info.chain_id, "granter": granter, "step": cli.step,
            "broadcast": cli.broadcast, "message": note}),
    )?;
    if !cli.broadcast {
        println!(
            "dry-run: nothing signed. Re-run with --broadcast --chain-id morocco-1 and TERP_SCRIPT_BROADCAST=1."
        );
        return Ok(());
    }

    // The read-only daemon holds the cw-orch state-file lock. Release it before
    // enable_session builds the session daemon on the same state file, as
    // dao_account does with drop(d). Otherwise: "State file ... already locked".
    drop(wq);
    drop(q);
    kms_gate::check_broadcast_preconditions(
        &kms_gate::BroadcastRequest {
            info: info.clone(),
            expected_chain_id: cli.chain_id.clone(),
            granter: granter.clone(),
            purpose: format!("cw721-svg-burn-fix:{}", cli.step),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: kms_gate::Scope::Sdk(vec![]),
            summary: note.to_string(),
        },
        std::env::var(kms_gate::BROADCAST_ENV).ok().as_deref(),
    )?;
    cw_orch_kms::ensure_ephemeral_mnemonic(info.kind.clone()).map_err(|e| anyhow!(e))?;
    cw_orch_kms::set_approval_chain(kms_gate::kms_profile(&info)).map_err(|e| anyhow!(e))?;
    cw_orch_kms::set_approval_chain_id(&info.chain_id).map_err(|e| anyhow!(e))?;

    let mut session = ApprovalSession::open(&granter);
    session = match &plan {
        Plan::Store(bytes) => session.allow_store(bytes).map_err(|e| anyhow!(e))?,
        // calls 2: x/wasm deletes a MaxCallsLimit grant at 0 and the close revoke would fail.
        Plan::Migrate { .. } => session
            .allow_migrate(ScopedCall {
                address: cli.collection.clone(),
                keys: vec![],
                calls: 2,
                funds: vec![],
            })
            .map_err(|e| anyhow!(e))?,
        Plan::Execute { .. } => session
            .allow_execute(ScopedCall {
                address: cli.factory.clone(),
                keys: vec!["update_code_id".into()],
                calls: 2,
                funds: vec![],
            })
            .map_err(|e| anyhow!(e))?,
    };
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = planned_spend_limit(2);
    let mut builder: DaemonBuilder = Daemon::builder(info.clone());
    builder.load_network(false);
    let run = enable_session(
        &builder,
        session,
        vec![SigningRequest::new("session-key").body(note.to_string())],
    )
    .map_err(|e| anyhow!(e))?;
    let memo = format!("cw721-svg burn fix: {}", cli.step);
    let result = run
        .daemon
        .rt_handle
        .block_on(run.daemon.sender().commit_tx_any(vec![msg_any], Some(memo.as_str())))
        .context("broadcast through cw-orch Daemon");
    let closed = run.close().map_err(|e| anyhow!(e));
    let resp = result?;
    closed?;
    println!("tx {} code {}", resp.txhash, resp.code);
    match &plan {
        Plan::Store(_) => {
            let id = resp.uploaded_code_id().map_err(|e| anyhow!("{e}"))?;
            println!(
                "stored code id {id}. next: migrate --code-id {id}, then update-code-id --code-id {id}"
            );
        }
        Plan::Migrate { code_id, msg } => {
            println!("migrated {} to code {code_id} with {msg}", cli.collection)
        }
        Plan::Execute { msg } => println!("factory {} executed {msg}", cli.factory),
    }
    Ok(())
}
