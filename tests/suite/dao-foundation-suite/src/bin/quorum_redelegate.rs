//! Dry-run the foundation stake move that puts one operator strictly above
//! one third of bonded tokens.
//!
//! Phase 1 is create-validator, signed by the new operator, and only when
//! that operator is not already bonded. Phase 2 is the next block: one
//! transaction from the foundation wallet. It sends 36 million TERP to the
//! foundation DAO first, then redelegates. Delegate liquid uterp only if the
//! redelegation is still short. Gas on that transaction is an explicit limit
//! for every message, not `--gas auto`.
//!
//! This path writes `01-create-validator.json` and `02-next-block.json` and
//! does not broadcast.
//!
//! `--rebalance` writes every legal redelegation into
//! `02-next-block-unsigned.json`. That includes the terpz share once its
//! source lock has ended. Locked moves stay in `03-terpz-share-design.json`.
//! Dry run is the default. `--broadcast` signs the unsigned tx with
//! `TERPZ_FOUNDATION_MNEMONIC` through cw-orch and submits it on gRPC.
//! The operator key is not read, and `terpd` is not invoked.

use anyhow::{Context, Result, bail};
use clap::Parser;
use cw_orch::environment::{ChainKind, NetworkInfoOwned};
use cw_orch::prelude::{ChainInfoOwned, Daemon, DaemonBuilder, Stargate, TxHandler};
use dao_foundation_suite::ibc_ops::quorum::{
    ALXVOY_OPERATOR, ActiveRedelegation, BankSend, DAO_SEND_TERP, DEFAULT_SELF_BOND,
    FOUNDATION_DAO, FOUNDATION_DELEGATOR, FOUNDATION_MNEMONIC_ENV, FoundationBook, MICRO,
    OperatorRow, QuorumSnapshot, SERGO_OPERATOR, StakeSource, TERPZ_CONSENSUS_PUBKEY,
    TERPZ_DETAILS, TERPZ_MONIKER, TERPZ_OPERATOR, TERPZ_TARGET_BPS,
    assert_foundation_redelegations, create_validator_file, dotenv_assignments, dotenv_value,
    encode_msg_begin_redelegate, executable_rebalance, fee_for_gas, next_block_commands,
    next_block_gas, next_block_messages, plan_next_block, plan_share_rebalance,
    rebalance_broadcast_block, sources_after_locks, stake_move_commands, stake_move_messages,
    unsigned_tx,
};
use prost_types::Any;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "quorum_redelegate")]
struct Args {
    #[arg(long, env = "TERP_LCD", default_value = "http://192.168.10.101:1317")]
    lcd: String,
    #[arg(long, default_value = FOUNDATION_DELEGATOR)]
    delegator: String,
    /// Destination operator (`terpvaloper…`). Required unless `--snapshot` has one.
    #[arg(long)]
    dest: Option<String>,
    /// Skip the LCD and plan from this `QuorumSnapshot` JSON file.
    #[arg(long)]
    snapshot: Option<PathBuf>,
    #[arg(long, default_value = "out")]
    out_dir: PathBuf,
    /// Self-bond counted when the destination is not bonded yet. 1 TERP.
    #[arg(long, default_value_t = DEFAULT_SELF_BOND)]
    self_bond: u128,
    #[arg(long, default_value = "uterp")]
    denom: String,
    #[arg(long, env = "CHAIN_ID", default_value = "morocco-1")]
    chain_id: String,
    #[arg(long, default_value = "0.05uthiol")]
    gas_prices: String,
    #[arg(long, default_value = TERPZ_MONIKER)]
    moniker: String,
    #[arg(long, default_value = TERPZ_DETAILS)]
    details: String,
    /// Base64 ed25519 consensus pubkey of the mac node.
    #[arg(long, default_value = TERPZ_CONSENSUS_PUBKEY)]
    consensus_pubkey: String,
    /// Whole TERP sent to the foundation DAO in the same tx. Zero skips the send.
    /// Ignored by `--rebalance`.
    #[arg(long, default_value_t = DAO_SEND_TERP)]
    dao_send_terp: u128,
    #[arg(long, default_value = FOUNDATION_DAO)]
    dao: String,
    /// Empty SerGo into AlxVoy and bring the hub down to `--target-bps`.
    /// Ready moves, including an unlocked hub, are written as one unsigned tx.
    #[arg(long)]
    rebalance: bool,
    /// Sign the rebalance tx with `TERPZ_FOUNDATION_MNEMONIC` and broadcast it
    /// through cw-orch. Omit this to write the unsigned tx and send nothing.
    #[arg(long, default_value_t = false)]
    broadcast: bool,
    /// Dotenv file read only when `--broadcast` is set and the foundation
    /// mnemonic is not already in the environment. The phrase is never printed.
    #[arg(long, default_value = ".env")]
    env_file: PathBuf,
    /// gRPC endpoint cw-orch uses when `--broadcast` is set.
    #[arg(long, env = "GRPC_URL", default_value = "http://192.168.10.101:9090")]
    grpc_url: String,
    /// Kept so older commands still parse. Broadcast does not dial it.
    #[arg(long, default_value = "tcp://192.168.10.101:26657")]
    node: String,
    /// Share of the bonded pool terpz should hold, in basis points. 2200 is 22%.
    #[arg(long, default_value_t = TERPZ_TARGET_BPS)]
    target_bps: u32,
    /// Operator whose foundation stake is moved in full. Default SerGo.
    #[arg(long, default_value = SERGO_OPERATOR)]
    empty_src: String,
    /// Operator that receives `--empty-src`. Default AlxVoy.
    #[arg(long, default_value = ALXVOY_OPERATOR)]
    empty_dst: String,
    /// Operator whose voting power is brought down to `--target-bps`. Default terpz.
    #[arg(long, default_value = TERPZ_OPERATOR)]
    hub: String,
    /// Skip the LCD and plan from this `FoundationBook` JSON file.
    #[arg(long)]
    book: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.broadcast && !args.rebalance {
        bail!("--broadcast sends the rebalance tx only");
    }
    if args.rebalance {
        return run_rebalance(&args);
    }
    let (mut snapshot, dest_was_bonded) = load_snapshot(&args)?;
    if snapshot.delegator.is_empty() {
        snapshot.delegator = args.delegator.clone();
    }
    if let Some(dest) = args.dest.as_deref() {
        snapshot.dest_operator = dest.to_string();
    }
    if snapshot.dest_operator.is_empty() {
        bail!("pass --dest or a snapshot whose dest_operator is set");
    }
    if snapshot.denom.is_empty() {
        snapshot.denom = args.denom.clone();
    }

    let self_bond_counted = if dest_was_bonded {
        0
    } else {
        snapshot = snapshot.with_new_validator_self_bond(args.self_bond);
        args.self_bond
    };

    let bank_send = if args.dao_send_terp == 0 {
        None
    } else {
        let amount = args
            .dao_send_terp
            .checked_mul(MICRO)
            .context("dao send overflow")?;
        snapshot
            .reserve_bank_send(amount)
            .context("holding the DAO send out of liquid stake")?;
        Some(BankSend {
            to: args.dao.clone(),
            amount,
            denom: snapshot.denom.clone(),
        })
    };

    let plan = plan_next_block(&snapshot).with_context(|| {
        format!(
            "planning {} -> {}",
            snapshot.delegator, snapshot.dest_operator
        )
    })?;

    std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create {}", args.out_dir.display()))?;

    let create_path = args.out_dir.join("01-create-validator.json");
    let next_path = args.out_dir.join("02-next-block.json");

    if dest_was_bonded {
        std::fs::write(
            &create_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "skipped": true,
                "reason": "destination is already bonded",
                "dest_operator": snapshot.dest_operator,
            }))?,
        )?;
    } else {
        std::fs::write(
            &create_path,
            serde_json::to_vec_pretty(&create_validator_file(
                &args.moniker,
                &args.details,
                &args.consensus_pubkey,
                args.self_bond,
                &snapshot.denom,
            ))?,
        )?;
    }

    let messages = next_block_messages(
        &snapshot.delegator,
        &snapshot.dest_operator,
        &snapshot.denom,
        bank_send.as_ref(),
        &plan,
    );
    let commands = next_block_commands(
        &args.chain_id,
        &args.gas_prices,
        &snapshot.delegator,
        &snapshot.dest_operator,
        &snapshot.denom,
        bank_send.as_ref(),
        &plan,
    );
    let gas = next_block_gas(bank_send.is_some(), &plan);
    let (fee_amount, fee_denom) = fee_for_gas(gas, &args.gas_prices)
        .map_err(|err| anyhow::anyhow!("gas price {}: {err}", args.gas_prices))?;
    let unsigned = unsigned_tx(&messages, gas, fee_amount, &fee_denom);
    let unsigned_path = args.out_dir.join("02-next-block-unsigned.json");
    std::fs::write(&unsigned_path, serde_json::to_vec_pretty(&unsigned)?)?;
    let next = serde_json::json!({
        "delegator": snapshot.delegator,
        "dest_operator": snapshot.dest_operator,
        "denom": snapshot.denom,
        "chain_id": args.chain_id,
        "dest_bonded": snapshot.dest_bonded.to_string(),
        "total_bonded": snapshot.total_bonded.to_string(),
        "self_bond_counted": self_bond_counted.to_string(),
        "bank_send": bank_send,
        "redelegates": plan.redelegates,
        "delegate": plan.delegate.to_string(),
        "dest_after": plan.dest_after.to_string(),
        "total_after": plan.total_after.to_string(),
        "above_one_third": plan.above_one_third(),
        "gas_limit": gas.to_string(),
        "fee": {"denom": fee_denom, "amount": fee_amount.to_string()},
        "messages": messages,
        "unsigned_tx": unsigned_path.display().to_string(),
        "commands": commands,
        "note": "Dry run. Sign 02-next-block-unsigned.json once. The bank send is the first message and the redelegations follow, so they land in one block. gas_limit already covers every message; do not resign with --gas auto. Commands omit --yes and a node URL. Create-validator, when present, is the previous block and a different signer.",
    });
    std::fs::write(&next_path, serde_json::to_vec_pretty(&next)?)?;

    println!("{}", serde_json::to_string_pretty(&next)?);
    println!(
        "wrote {}, {}, and {}",
        create_path.display(),
        next_path.display(),
        unsigned_path.display()
    );
    if !dest_was_bonded && args.consensus_pubkey.is_empty() {
        eprintln!(
            "create-validator pubkey is empty; set --consensus-pubkey before that tx is signed"
        );
    }
    Ok(())
}

fn run_rebalance(args: &Args) -> Result<()> {
    // `--node` stays on the CLI so an older command still parses. Broadcast dials gRPC.
    let _ = &args.node;
    let book = load_book(args)?;
    let plan = plan_share_rebalance(
        &book,
        &args.hub,
        &args.empty_src,
        &args.empty_dst,
        args.target_bps,
    )
    .with_context(|| format!("rebalance {} to {} bps", args.hub, args.target_bps))?;
    std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create {}", args.out_dir.display()))?;

    let create_path = args.out_dir.join("01-create-validator.json");
    std::fs::write(
        &create_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "skipped": true,
            "reason": "rebalance does not create a validator",
        }))?,
    )?;

    let executable = executable_rebalance(&plan);
    let ready_msgs = stake_move_messages(&book.delegator, &book.denom, &executable);
    let ready_cmds = stake_move_commands(
        &args.chain_id,
        &args.gas_prices,
        &book.delegator,
        &book.denom,
        &executable,
    );
    let gas = if executable.is_empty() {
        0
    } else {
        next_block_gas(
            false,
            &dao_foundation_suite::ibc_ops::quorum::QuorumPlan {
                redelegates: executable
                    .iter()
                    .map(|mv| dao_foundation_suite::ibc_ops::quorum::Redelegate {
                        src: mv.src.clone(),
                        amount: mv.amount,
                    })
                    .collect(),
                delegate: 0,
                dest_after: 0,
                total_after: 0,
            },
        )
    };
    let unsigned_path = args.out_dir.join("02-next-block-unsigned.json");
    if let Some(reason) = rebalance_broadcast_block(&plan) {
        std::fs::write(
            &unsigned_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "skipped": true,
                "reason": reason,
            }))?,
        )?;
    } else {
        let (fee_amount, fee_denom) = fee_for_gas(gas, &args.gas_prices)
            .map_err(|err| anyhow::anyhow!("gas price {}: {err}", args.gas_prices))?;
        let unsigned = unsigned_tx(&ready_msgs, gas, fee_amount, &fee_denom);
        std::fs::write(&unsigned_path, serde_json::to_vec_pretty(&unsigned)?)?;
    }
    let share = |tokens: u128, total: u128| -> String {
        if total == 0 {
            return "0".into();
        }
        format!("{:.2}%", (tokens as f64) * 100.0 / (total as f64))
    };
    let next = serde_json::json!({
        "delegator": book.delegator,
        "hub": args.hub,
        "empty_src": args.empty_src,
        "empty_dst": args.empty_dst,
        "target_bps": plan.target_bps,
        "hub_before": plan.hub_before.to_string(),
        "hub_share_before": share(plan.hub_before, book.total_bonded),
        "hub_after_ready": plan.hub_after_ready.to_string(),
        "hub_share_after_ready": share(plan.hub_after_ready, plan.total_after_ready),
        "hub_after": plan.hub_after.to_string(),
        "hub_share_after": share(plan.hub_after, plan.total_after),
        "total_bonded": book.total_bonded.to_string(),
        "total_after_ready": plan.total_after_ready.to_string(),
        "total_after": plan.total_after.to_string(),
        "ready": executable,
        "gas_limit": gas.to_string(),
        "messages": ready_msgs,
        "unsigned_tx": unsigned_path.display().to_string(),
        "commands": ready_cmds,
        "deferred_until": plan.deferred_until,
        "note": if plan.deferred.is_empty() {
            format!(
                "Dry run. Every ready redelegation, including the hub share, is in 02-next-block-unsigned.json. Pass --broadcast to sign it with {FOUNDATION_MNEMONIC_ENV} and submit it through cw-orch to {}. No bank send. Commands omit --yes and a node URL.",
                args.grpc_url
            )
        } else {
            format!(
                "Dry run. Ready redelegations are in 02-next-block-unsigned.json. {} move(s) stay locked until {} and are listed in 03-terpz-share-design.json, not in this transaction. After the lock, run --rebalance again and pass --broadcast with {FOUNDATION_MNEMONIC_ENV}. No bank send. Commands omit --yes and a node URL.",
                plan.deferred.len(),
                plan.deferred_until
            )
        },
    });
    let next_path = args.out_dir.join("02-next-block.json");
    std::fs::write(&next_path, serde_json::to_vec_pretty(&next)?)?;
    let deferred_path = args.out_dir.join("03-terpz-share-design.json");
    let deferred = serde_json::json!({
        "executable": false,
        "deferred_until": plan.deferred_until.clone(),
        "moves": plan.deferred.clone(),
        "hub_after": plan.hub_after.to_string(),
        "total_after": plan.total_after.to_string(),
        "target_bps": plan.target_bps,
        "note": "Omitted from 02-next-block-unsigned.json until deferred_until. A later --rebalance puts these moves in that tx. --broadcast signs only the unsigned tx with the foundation key.",
    });
    std::fs::write(&deferred_path, serde_json::to_vec_pretty(&deferred)?)?;
    let stale_deferred = args.out_dir.join("03-deferred.json");
    if stale_deferred != deferred_path {
        let _ = std::fs::remove_file(&stale_deferred);
    }
    println!("{}", serde_json::to_string_pretty(&next)?);
    println!(
        "wrote {}, {}, {}, and {}",
        create_path.display(),
        next_path.display(),
        unsigned_path.display(),
        deferred_path.display()
    );
    if !args.broadcast {
        println!("dry run; nothing was signed or broadcast");
        if let Some(reason) = rebalance_broadcast_block(&plan) {
            println!("{reason}");
        } else {
            println!(
                "pass --broadcast to sign {} with {FOUNDATION_MNEMONIC_ENV} and submit it through cw-orch to {}",
                unsigned_path.display(),
                args.grpc_url
            );
        }
        if !plan.deferred.is_empty() {
            println!(
                "{} redelegation(s) stay locked until {}",
                plan.deferred.len(),
                plan.deferred_until
            );
        }
        return Ok(());
    }
    if let Some(reason) = rebalance_broadcast_block(&plan) {
        bail!("refusing to broadcast: {reason}");
    }
    broadcast_foundation_tx(args, &unsigned_path, &book.delegator)?;
    Ok(())
}

fn broadcast_foundation_tx(args: &Args, unsigned: &Path, delegator: &str) -> Result<()> {
    let tx: serde_json::Value = serde_json::from_slice(&fs::read(unsigned)?)?;
    if tx.get("skipped").and_then(|v| v.as_bool()) == Some(true) {
        bail!("refusing to broadcast: the unsigned tx was skipped");
    }
    assert_foundation_redelegations(&tx, delegator).map_err(|err| anyhow::anyhow!(err))?;
    if delegator != FOUNDATION_DELEGATOR {
        bail!("this plan signs {delegator}; the foundation key signs {FOUNDATION_DELEGATOR}");
    }
    let msgs = redelegate_anys(&tx)?;
    let mnemonic = load_foundation_mnemonic(&args.env_file)?;
    let daemon = connect(&chain_info(args), &mnemonic).map_err(|err| {
        let detail = redact(&error_chain(&err), &mnemonic);
        if detail.contains("Bad Phrase") {
            anyhow::anyhow!(
                "{FOUNDATION_MNEMONIC_ENV} is not a valid English mnemonic. Check the word list and the checksum. The phrase was not printed and nothing was signed."
            )
        } else {
            anyhow::anyhow!("build cw-orch daemon: {detail}")
        }
    })?;
    drop(mnemonic);
    let foundation = daemon.sender_addr().to_string();
    if foundation != delegator {
        bail!("foundation key is {foundation}; this plan signs {delegator}");
    }
    println!("foundation {foundation}");
    println!("cw-orch grpc {}", args.grpc_url);
    let resp = Stargate::commit_any(&daemon, msgs, None)
        .map_err(|err| anyhow::anyhow!("cw-orch broadcast failed: {err}"))?;
    println!(
        "rebalance tx {} code {} height {}",
        resp.txhash, resp.code, resp.height
    );
    if resp.code != 0 {
        bail!(
            "chain rejected the tx with code {}: {}",
            resp.code,
            resp.raw_log
        );
    }
    Ok(())
}

fn redelegate_anys(tx: &serde_json::Value) -> Result<Vec<Any>> {
    let messages = tx
        .pointer("/body/messages")
        .and_then(|v| v.as_array())
        .context("unsigned tx has no messages")?;
    let mut out = Vec::with_capacity(messages.len());
    for msg in messages {
        let kind = msg.get("@type").and_then(|v| v.as_str()).unwrap_or("");
        if !kind.ends_with("MsgBeginRedelegate") {
            bail!("unsigned tx contains {kind}");
        }
        let delegator = field(msg, "delegator_address")?;
        let src = field(msg, "validator_src_address")?;
        let dst = field(msg, "validator_dst_address")?;
        let denom = msg
            .pointer("/amount/denom")
            .and_then(|v| v.as_str())
            .context("redelegate amount denom")?;
        let amount: u128 = msg
            .pointer("/amount/amount")
            .and_then(|v| v.as_str())
            .context("redelegate amount")?
            .parse()
            .context("redelegate amount")?;
        out.push(Any {
            type_url: "/cosmos.staking.v1beta1.MsgBeginRedelegate".to_string(),
            value: encode_msg_begin_redelegate(delegator, src, dst, amount, denom),
        });
    }
    if out.is_empty() {
        bail!("unsigned tx has no redelegation");
    }
    Ok(out)
}

fn field<'a>(msg: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    msg.get(key)
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .with_context(|| format!("redelegate message is missing {key}"))
}

fn connect(chain: &ChainInfoOwned, mnemonic: &str) -> Result<Daemon> {
    let mut builder = DaemonBuilder::new(chain.clone());
    // A temp state file keeps this signer off the shared ~/.cw-orch/state.json lock.
    builder.mnemonic(mnemonic).load_network(false).is_test(true);
    builder.build().context("build cw-orch daemon")
}

fn chain_info(args: &Args) -> ChainInfoOwned {
    let gas_price = args
        .gas_prices
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .unwrap_or(0.05);
    ChainInfoOwned {
        chain_id: args.chain_id.clone(),
        gas_denom: "uthiol".into(),
        gas_price,
        grpc_urls: vec![args.grpc_url.clone()],
        rpc_urls: vec![],
        lcd_url: Some(args.lcd.clone()),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: "terp".into(),
            coin_type: 118,
        },
        kind: ChainKind::Mainnet,
    }
}

fn error_chain(err: &anyhow::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(inner) = source {
        let text = inner.to_string();
        if parts.last().is_none_or(|prev| prev != &text) {
            parts.push(text);
        }
        source = inner.source();
    }
    parts.join(": ")
}

fn redact(text: &str, secret: &str) -> String {
    if secret.len() > 8 {
        text.replace(secret, "[redacted]")
    } else {
        text.to_string()
    }
}

fn load_foundation_mnemonic(env_file: &Path) -> Result<String> {
    if let Ok(value) = std::env::var(FOUNDATION_MNEMONIC_ENV) {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return checked_phrase(&value);
        }
    }
    let text = fs::read_to_string(env_file).with_context(|| {
        format!(
            "read {} and set {FOUNDATION_MNEMONIC_ENV}",
            env_file.display()
        )
    })?;
    let pairs = dotenv_assignments(&text);
    let value = dotenv_value(&pairs, FOUNDATION_MNEMONIC_ENV)
        .ok_or_else(|| anyhow::anyhow!("{FOUNDATION_MNEMONIC_ENV} is empty in the env file"))?;
    checked_phrase(value)
}

fn checked_phrase(value: &str) -> Result<String> {
    let words = value.split_whitespace().count();
    if words != 12 && words != 24 {
        bail!("{FOUNDATION_MNEMONIC_ENV} must be 12 or 24 words");
    }
    Ok(value.to_string())
}

fn load_book(args: &Args) -> Result<FoundationBook> {
    if let Some(path) = &args.book {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        return serde_json::from_slice(&bytes)
            .with_context(|| format!("parse book {}", path.display()));
    }
    let lcd = args.lcd.trim_end_matches('/');
    let validators = lcd_pages(
        lcd,
        "/cosmos/staking/v1beta1/validators?pagination.limit=200",
        "validators",
    )?;
    let mut operators = Vec::new();
    for val in &validators {
        let operator = val
            .get("operator_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if operator.is_empty() {
            continue;
        }
        let jailed = val.get("jailed").and_then(|v| v.as_bool()).unwrap_or(false);
        let status = val.get("status").and_then(|v| v.as_str()).unwrap_or("");
        operators.push(OperatorRow {
            operator,
            moniker: val
                .pointer("/description/moniker")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            tokens: parse_u128(val.get("tokens").and_then(|v| v.as_str()).unwrap_or("0"))?,
            bonded: status == "BOND_STATUS_BONDED" && !jailed,
        });
    }
    let pool = lcd_get(&format!("{lcd}/cosmos/staking/v1beta1/pool"))?;
    let total_bonded = parse_u128(
        pool.pointer("/pool/bonded_tokens")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    let delegations = lcd_pages(
        lcd,
        &format!(
            "/cosmos/staking/v1beta1/delegations/{}?pagination.limit=200",
            args.delegator
        ),
        "delegation_responses",
    )?;
    let mut rows = Vec::new();
    for row in &delegations {
        let validator = row
            .pointer("/delegation/validator_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let amount = row
            .pointer("/balance/amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let row_denom = row
            .pointer("/balance/denom")
            .and_then(|v| v.as_str())
            .unwrap_or(&args.denom);
        if row_denom != args.denom || validator.is_empty() {
            continue;
        }
        rows.push(StakeSource {
            validator,
            redelegable: parse_u128(amount)?,
        });
    }
    let redelegations = lcd_pages(
        lcd,
        &format!(
            "/cosmos/staking/v1beta1/delegators/{}/redelegations?pagination.limit=200",
            args.delegator
        ),
        "redelegation_responses",
    )?;
    let mut entries = Vec::new();
    for row in &redelegations {
        let src = row
            .pointer("/redelegation/validator_src_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let dst = row
            .pointer("/redelegation/validator_dst_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let items = row
            .get("entries")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for item in items {
            entries.push(ActiveRedelegation {
                src: src.clone(),
                dst: dst.clone(),
                balance: parse_u128(item.get("balance").and_then(|v| v.as_str()).unwrap_or("0"))?,
                completion_time: item
                    .pointer("/redelegation_entry/completion_time")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Ok(FoundationBook {
        delegator: args.delegator.clone(),
        denom: args.denom.clone(),
        total_bonded,
        operators,
        delegations: rows,
        redelegations: entries,
    })
}

fn load_snapshot(args: &Args) -> Result<(QuorumSnapshot, bool)> {
    if let Some(path) = &args.snapshot {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let snapshot: QuorumSnapshot = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse snapshot {}", path.display()))?;
        let bonded = snapshot.dest_bonded > 0;
        return Ok((snapshot, bonded));
    }

    let lcd = args.lcd.trim_end_matches('/');
    let delegator = &args.delegator;
    let denom = &args.denom;
    let dest = args
        .dest
        .clone()
        .context("pass --dest when there is no --snapshot")?;

    let validators = lcd_pages(
        lcd,
        &format!(
            "/cosmos/staking/v1beta1/validators?status=BOND_STATUS_BONDED&pagination.limit=200"
        ),
        "validators",
    )?;
    let mut total_bonded: u128 = 0;
    let mut dest_bonded: u128 = 0;
    let mut dest_was_bonded = false;
    for val in &validators {
        let tokens = parse_u128(val.get("tokens").and_then(|v| v.as_str()).unwrap_or("0"))?;
        total_bonded = total_bonded
            .checked_add(tokens)
            .context("bonded token total overflow")?;
        if val.get("operator_address").and_then(|v| v.as_str()) == Some(dest.as_str()) {
            dest_bonded = tokens;
            dest_was_bonded = true;
        }
    }

    let delegations = lcd_pages(
        lcd,
        &format!("/cosmos/staking/v1beta1/delegations/{delegator}?pagination.limit=200"),
        "delegation_responses",
    )?;
    let mut sources = Vec::new();
    for row in &delegations {
        let validator = row
            .pointer("/delegation/validator_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let amount = row
            .pointer("/balance/amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let row_denom = row
            .pointer("/balance/denom")
            .and_then(|v| v.as_str())
            .unwrap_or(denom);
        if row_denom != denom {
            continue;
        }
        sources.push(StakeSource {
            validator,
            redelegable: parse_u128(amount)?,
        });
    }

    let redelegations = lcd_pages(
        lcd,
        &format!(
            "/cosmos/staking/v1beta1/delegators/{delegator}/redelegations?pagination.limit=200"
        ),
        "redelegation_responses",
    )?;
    let mut locks = Vec::new();
    for row in &redelegations {
        let src = row
            .pointer("/redelegation/validator_src_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let entries = row
            .get("entries")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for entry in entries {
            let balance = entry.get("balance").and_then(|v| v.as_str()).unwrap_or("0");
            locks.push((src.clone(), parse_u128(balance)?));
        }
    }
    sources = sources_after_locks(sources, locks);

    let balance_url =
        format!("{lcd}/cosmos/bank/v1beta1/balances/{delegator}/by_denom?denom={denom}");
    let balance = lcd_get(&balance_url)?;
    let liquid = balance
        .pointer("/balance/amount")
        .and_then(|v| v.as_str())
        .unwrap_or("0");

    Ok((
        QuorumSnapshot {
            delegator: delegator.clone(),
            dest_operator: dest,
            dest_bonded,
            total_bonded,
            sources,
            liquid: parse_u128(liquid)?,
            denom: denom.clone(),
        },
        dest_was_bonded,
    ))
}

fn lcd_pages(lcd: &str, path_and_query: &str, array_key: &str) -> Result<Vec<serde_json::Value>> {
    let mut out = Vec::new();
    let mut next: Option<String> = None;
    loop {
        let mut url = format!("{lcd}{path_and_query}");
        if let Some(key) = &next {
            url.push_str(&format!("&pagination.key={}", percent_encode(key)));
        }
        let page = lcd_get(&url)?;
        if let Some(rows) = page.get(array_key).and_then(|v| v.as_array()) {
            out.extend(rows.iter().cloned());
        }
        next = page
            .pointer("/pagination/next_key")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if next.is_none() {
            break;
        }
    }
    Ok(out)
}

fn lcd_get(url: &str) -> Result<serde_json::Value> {
    let output = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "30",
            "-H",
            "User-Agent: terp-quorum-redelegate",
            url,
        ])
        .output()
        .context("curl LCD")?;
    if !output.status.success() {
        bail!(
            "LCD {url} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).with_context(|| format!("LCD JSON from {url}"))
}

fn parse_u128(text: &str) -> Result<u128> {
    text.parse::<u128>()
        .with_context(|| format!("u128 amount {text}"))
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
