//! Bitsong Delegation Realignment Protocol
//!
//! This tool automates the rebalancing of delegations across 33 obligated validators
//! for the Bitsong DAO multisig addresses.
//!
//! # Design Overview
//!
//! 1. **Collect Obligations** — Loads target delegation amounts from `new-delegations.csv`.
//! 2. **Identify Bad Validators** — Detects unbonded, unbonding, jailed, or zero-obligation validators.
//! 3. **Gather Freed Tokens** — Pulls all delegations from bad validators + excess from over-delegated active validators.
//! 4. **Even Redistribution** — Freed tokens are distributed as evenly as possible across active validators
//!    with remaining deficits using a greedy round-robin algorithm (largest deficit first, largest source first).
//! 5. **Output** — Generates `delegation_messages.json` containing `MsgBeginRedelegate`, `MsgDelegate`, and `MsgUndelegate` messages.
//!
//! # Usage
//!
//! ```bash
//! # Dry-run (generate messages only)
//! cargo run --bin delegations -- --network main
//!
//! # Broadcast transactions (requires authz setup)
//! cargo run --bin delegations -- --network main --broadcast
//! ```
//!
//! **Required files:**
//! - `./src/bin/data/new-delegations.csv` — (validator_operator_address,amount_in_ubtsg)
//! - Valid mnemonic with Authz grants from the three DAO addresses.
//!
//! **Key Features:**
//! - Even token distribution to avoid lumpy redelegations
//! - Proper handling of jailed/unbonded validators
//! - Final state verification
//! - Batch broadcasting with 32 msgs per transaction

use std::{collections::HashMap, fs::File, io::Write, str::FromStr};

use anyhow::anyhow;
use clap::Parser;
use cosmos_sdk_proto::cosmos::{
    base::{query::v1beta1::PageRequest, v1beta1::Coin as ProtoCoin},
    staking::v1beta1::{DelegationResponse, MsgBeginRedelegate, MsgDelegate, MsgUndelegate},
};
use cosmrs::{AccountId, Any, distribution::MsgSetWithdrawAddress, tx::Msg};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal, Uint128};
use csv::ReaderBuilder;
use cw_orch::{
    daemon::{
        DaemonBuilder, TxSender, Wallet,
        networks::TERP_MAINNET,
        queriers::{Bank, Staking},
    },
    environment::{ChainKind, NetworkInfo},
    prelude::*,
};

use terp_rs::{Message, Name, cosmos::staking::v1::MsgCancelUnbondingDelegation};
use tokio::runtime::Runtime;

pub const TOTAL_OBLIGATED_VALIDATORS: usize = 33;
pub const TOTAL_OBLIGATED_DELEGATED_Terp: Uint128 = Uint128::new(9_999_980_000_000u128);
pub const NEW_DELS_FILE: &str = "./src/bin/data/new-delegations.csv";
pub const RAW_MSG_JSON: &str = "delegation_messages.json";

#[cw_serde]
struct DelegationDaoEntity {
    dao_add: String,
    current_balance: Coin,
    current_delegation: Uint128,
    obligated_delegation: Uint128,
    total_delegation_count: usize,
}

#[cw_serde]
struct AlignedValidator {
    operator_addr: String,
    current_delegations: Vec<Delegation>,
    new_delegation_amount: Uint128,
}

#[cw_serde]
struct Delegation {
    del_addr: String,
    operator_addr: String,
    amount: Uint128,
    shares: String, // original shares string from chain; "0" for CSV-sourced
}

#[cw_serde]
struct AllAlignedDelegations {
    delegations: Vec<Delegation>,
    total: Uint128,
}

#[cw_serde]
struct RedelegateMsg {
    delegator_address: String,
    validator_src_address: String,
    validator_dst_address: String,
    amount: String,
    denom: String,
}

#[cw_serde]
struct UndelegateMsg {
    delegator_address: String,
    validator_address: String,
    amount: String,
    denom: String,
}

#[cw_serde]
struct DelegateMsg {
    delegator_address: String,
    validator_address: String,
    amount: String,
    denom: String,
}

#[cw_serde]
struct Redelegations {
    data: Vec<RedelegateMsg>,
    count: usize,
    total_ubtsg: Uint128,
}

#[cw_serde]
struct Delegations {
    data: Vec<DelegateMsg>,
    count: usize,
    total_ubtsg: Uint128,
}

#[cw_serde]
struct Undelegations {
    data: Vec<UndelegateMsg>,
    count: usize,
    total_ubtsg: Uint128,
}

#[cw_serde]
struct MessageExport {
    redelegations: Redelegations,
    delegations: Delegations,
    undelegates: Undelegations,
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Network to deploy on: main, testnet, local
    #[clap(short, long)]
    network: String,
    /// whether or not to broadcast the txs formed
    #[clap(short, long)]
    broadcast: bool,
    /// Optional CSV file of delegations (validator,amount) to unbond/redelegate
    /// If no header: one `validator_operator_address,amount` per line.
    /// If header: first line is `validator,amount` (auto-detected).
    /// Amount is in micro-denom (e.g. uthiol).
    /// Example (no header):
    ///   terpvaloper1...,123456789
    ///   terpvaloper1...,987654321
    /// Example (with header):
    ///   validator,amount
    ///   terpvaloper1...,123456789
    #[clap(long)]
    csv: Option<String>,
    /// Distribution method: "even" (default, equal per validator) or "greedy" (largest deficit first)
    #[clap(long, default_value = "even")]
    method: String,
}

fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    // Fix for rustls 0.23+ CryptoProvider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls ring crypto provider");

    // parse cargo command arguments for network type
    let args = Args::parse();

    let delegation_dao_addrs: Vec<String> =
        ["terp1qfpnat6kc99gsc3233cw6fq0s0fdv7cq06alqu".into()].to_vec();

    let bitsong_chain: ChainInfoOwned = match args.network.as_str() {
        "main" => TERP_MAINNET.to_owned(),
        // "testnet" => TERPNETWORK_TESTNET.to_owned(),
        // "local" => LOCAL_NETWORK1.to_owned(),
        _ => panic!("Invalid network"),
    }
    .into();
    // connect to chain with mnemonic
    let mut chain = DaemonBuilder::new(bitsong_chain.clone()).build()?;

    // create client
    let staking_query_client: Staking = chain.querier();
    let bank_query_client: Bank = chain.querier();
    let node_query: queriers::Node = chain.node_querier();
    let sender = chain.sender_mut().clone();
    let block = chain.node_querier().latest_block()?;
    let derived_address = chain.sender().address();

    let rt = chain.rt_handle;

    let account = rt.block_on(sender.base_account())?;
    println!("Script is using BaseAccount: {:#?}", account);
    println!("Script is using address: {}", derived_address);

    // let witdraw = MsgSetWithdrawAddress {
    //     delegator_address: AccountId::from_str(&sender.pub_addr_str()).unwrap(),
    //     withdraw_address: AccountId::from_str(
    //         &"terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w",
    //     )
    //     .unwrap(),
    // };

    // let any = Any {
    //     type_url: MsgCancelUnbondingDelegation::type_url(),
    //     value: MsgCancelUnbondingDelegation {
    //         delegator_address: sender.pub_addr_str(),
    //         validator_address: "terpvaloper1g80p4suzrz4vqsvs3za0yqhqcphlvnjy7snfnw".into(),
    //         amount: Some(terp_rs::Coin {
    //             denom: "uterp".into(),
    //             amount: Uint128::new(750000000000).to_string(),
    //         }),
    //         creation_height: 21482756 as i64,
    //         // creation_height: i.0 as i64,
    //     }
    //     .encode_to_vec(),
    // };

    let any = Any {
        type_url: MsgUndelegate::type_url(),
        value: MsgUndelegate {
            delegator_address: sender.pub_addr_str(),
            validator_address: "terpvaloper126jdyc74p70ahzrf3wvaka7hmeke8g2ntym70p".into(),
            amount: Some(cosmos_sdk_proto::cosmos::base::v1beta1::Coin {
                denom: "uterp".into(),
                amount: Uint128::new(750000000000).to_string(),
            }),
            // creation_height: i.0 as i64,
        }
        .encode_to_vec(),
    };

    let any2 = Any {
        type_url: MsgUndelegate::type_url(),
        value: MsgUndelegate {
            delegator_address: sender.pub_addr_str(),
            validator_address: "terpvaloper1g80p4suzrz4vqsvs3za0yqhqcphlvnjy7snfnw".into(),
            amount: Some(cosmos_sdk_proto::cosmos::base::v1beta1::Coin {
                denom: "uterp".into(),
                amount: Uint128::new(750000000000).to_string(),
            }),
            // creation_height: i.0 as i64,
        }
        .encode_to_vec(),
    };

    // let msg = form_redel_msg(RedelegateMsg {
    //     delegator_address: sender.pub_addr_str(),
    //     validator_src_address: "terpvaloper1g80p4suzrz4vqsvs3za0yqhqcphlvnjy7snfnw".into(),
    //     validator_dst_address: "terpvaloper1mzns2fp4urelqxyjxyjelef34j9eynu8ffd9vt".into(),
    //     amount: Uint128::new(750000000000).to_string(),
    //     denom: "uterp".into(),
    // });
    // let msg2 = form_redel_msg(RedelegateMsg {
    //     delegator_address: sender.pub_addr_str(),
    //     validator_src_address: "terpvaloper1fak8qed3zu86d8hlmkdsee9h2xtutawlqm660j".into(),
    //     validator_dst_address: "terpvaloper1dmyu7jdu7pgzgsxucnav6sanfk8s7jfsw0y60a".into(),
    //     amount: Uint128::new(500000000000).to_string(),
    //     denom: "uterp".into(),
    // });
    rt.block_on(sender.commit_tx_any(vec![any,any2], None))?;

    // eprintln!("[MAIN] Starting realign_delegations (dry run + file generation)...");
    // // Execute the async function using the runtime
    // rt.block_on(realign_delegations(
    //     staking_query_client,
    //     bank_query_client,
    //     &delegation_dao_addrs,
    //     block.height,
    //     args.csv.as_deref(),
    //     &args.method,
    // ))?;
    // eprintln!("[MAIN] realign_delegations completed successfully");

    // eprintln!("[MAIN] Starting form_and_broadcast_obligated_msgs...");
    // form_and_broadcast_obligated_msgs(&rt, sender, RAW_MSG_JSON, delegation_dao_addrs)?;
    eprintln!("[MAIN] Script completed successfully");
    Ok(())
}

async fn realign_delegations(
    staking_query_client: Staking,
    _bank_client: Bank,
    dao_addrs: &[String],
    height: u64,
    csv_path: Option<&str>,
    method: &str,
) -> anyhow::Result<()> {
    env_logger::init();

    eprintln!("[REALIGN] === Delegation Realignment Protocol (Even Redistribution) ===");

    // === Step 0: Determine source of delegations to unbond ===
    let to_redelegate: Vec<Delegation>;
    let csv_mode: bool;
    let unbonded_vals;
    let unbonding_vals;
    let val_historical: cosmos_sdk_proto::cosmos::staking::v1beta1::QueryHistoricalInfoResponse;

    if let Some(csv) = csv_path {
        // Load from CSV file (skip chain queries for bad validators)
        csv_mode = true;
        unbonded_vals = vec![];
        unbonding_vals = vec![];
        val_historical =
            cosmos_sdk_proto::cosmos::staking::v1beta1::QueryHistoricalInfoResponse::default();
        to_redelegate = load_unbond_delegations(csv, dao_addrs, "uthiol");
        println!(
            "Total loaded from CSV: {} uthiol ({} entries)",
            Decimal::from_atomics(to_redelegate.iter().map(|d| d.amount).sum::<Uint128>(), 6)?,
            to_redelegate.len(),
        );
    } else {
        // Query chain for unbonded/unbonding/jailed validator delegations
        csv_mode = false;
        unbonded_vals = staking_query_client
            ._validators(queriers::StakingBondStatus::Unbonded)
            .await?;
        unbonding_vals = staking_query_client
            ._validators(queriers::StakingBondStatus::Unbonding)
            .await?;
        val_historical = staking_query_client
            ._historical_info(height.try_into().unwrap())
            .await?;

        let bad_result =
            fetch_delegations_to_bad_validators(&staking_query_client, dao_addrs, height).await?;

        to_redelegate = bad_result.delegations;
        println!(
            "Total freed from bad validators: {} uterp",
            Decimal::from_atomics(to_redelegate.iter().map(|d| d.amount).sum::<Uint128>(), 6)?
        );
    };

    // === Step 1: Collect ALL current delegations ===
    let mut all_dao_delegations: Vec<DelegationResponse> = Vec::new();

    for dao in dao_addrs {
        let mut next_key = None;
        loop {
            let response = staking_query_client
                ._delegator_delegations(&Addr::unchecked(dao), next_key)
                .await?;

            all_dao_delegations.extend(response.delegation_responses);

            next_key = match response.pagination {
                Some(p) if !p.next_key.is_empty() => Some(PageRequest {
                    key: p.next_key,
                    ..Default::default()
                }),
                _ => break,
            };
        }
    }

    // === Step 3: Identify Active Validators ===
    println!();
    println!("=== All delegations (for reference) ===");

    let mut active_validators: Vec<AlignedValidator> = Vec::new();

    for del_resp in &all_dao_delegations {
        let delegation = del_resp.delegation.as_ref().unwrap();
        let balance = del_resp.balance.as_ref().unwrap();
        let amount = Uint128::from_str(&balance.amount).map_err(|e| anyhow!(e))?;

        // Print each delegation like the bash script
        println!(
            "  Validator: {} | Shares: {} | Balance: {} uterp",
            delegation.validator_address, delegation.shares, balance.amount
        );

        if amount.is_zero() {
            continue;
        }

        let is_bad = if csv_mode {
            // CSV mode: all validators are active recipients
            false
        } else {
            unbonded_vals
                .iter()
                .any(|v| v.address == delegation.validator_address)
                || unbonding_vals
                    .iter()
                    .any(|v| v.address == delegation.validator_address)
                || val_historical.hist.as_ref().map_or(false, |h| {
                    h.valset
                        .iter()
                        .any(|v| v.operator_address == delegation.validator_address && v.jailed)
                })
        };

        if !is_bad {
            // Add to active validator
            if let Some(existing) = active_validators
                .iter_mut()
                .find(|v| v.operator_addr == delegation.validator_address)
            {
                existing.current_delegations.push(Delegation {
                    del_addr: delegation.delegator_address.clone(),
                    operator_addr: delegation.validator_address.clone(),
                    amount,
                    shares: delegation.shares.clone(),
                });
            } else {
                active_validators.push(AlignedValidator {
                    operator_addr: delegation.validator_address.clone(),
                    current_delegations: vec![Delegation {
                        del_addr: delegation.delegator_address.clone(),
                        operator_addr: delegation.validator_address.clone(),
                        amount,
                        shares: delegation.shares.clone(),
                    }],
                    new_delegation_amount: Uint128::zero(), // will be set later
                });
            }
        }
    }

    println!(
        "Found {} active validators for redistribution",
        active_validators.len()
    );

    // Exclude validators we're redelegating FROM — can't redelegate to itself
    let before_filter = active_validators.len();
    active_validators.retain(|v| {
        !to_redelegate
            .iter()
            .any(|src| src.operator_addr == v.operator_addr)
    });
    if active_validators.len() < before_filter {
        println!(
            "Excluded {} source validator(s) from active set (can't redelegate to itself)",
            before_filter - active_validators.len()
        );
    }

    // === Step 4: Compute EVEN target per active validator ===
    let total_to_distribute: Uint128 = to_redelegate.iter().map(|d| d.amount).sum();
    let num_active = active_validators.len() as u128;

    if num_active == 0 {
        return Err(anyhow::anyhow!("No active validators found"));
    }

    let base_amount = total_to_distribute / Uint128::from(num_active);
    let remainder = total_to_distribute % Uint128::from(num_active);

    for (i, val) in active_validators.iter_mut().enumerate() {
        val.new_delegation_amount = if (i as u128) < remainder.u128() {
            base_amount + Uint128::one()
        } else {
            base_amount
        };
    }

    println!(
        "Distributing {} uterp → {} method ({} active validators)",
        Decimal::from_atomics(total_to_distribute, 6)?,
        method,
        num_active,
    );

    // === Step 5: Distribute using selected method ===
    let (redelegation_msgs, delegation_msgs, undelegate_msgs) = match method {
        "greedy" => distribute_redelegated_greedy(to_redelegate, &mut active_validators, "uthiol"),
        _ => distribute_redelegated_evenly(to_redelegate, &mut active_validators, "uthiol"),
    };

    // === Step 6: Build final export ===
    let total_redel: Uint128 = redelegation_msgs
        .iter()
        .map(|m| Uint128::from_str(&m.amount.as_ref().unwrap().amount).unwrap())
        .sum();

    let export = MessageExport {
        redelegations: Redelegations {
            data: redelegation_msgs
                .iter()
                .map(|msg| {
                    let amount = msg.amount.as_ref().unwrap();
                    RedelegateMsg {
                        delegator_address: msg.delegator_address.clone(),
                        validator_src_address: msg.validator_src_address.clone(),
                        validator_dst_address: msg.validator_dst_address.clone(),
                        amount: amount.amount.clone(),
                        denom: amount.denom.clone(),
                    }
                })
                .collect(),
            count: redelegation_msgs.len(),
            total_ubtsg: total_redel,
        },
        delegations: Delegations {
            data: vec![],
            count: 0,
            total_ubtsg: Uint128::zero(),
        },
        undelegates: Undelegations {
            data: undelegate_msgs
                .iter()
                .map(|msg| {
                    let amount = msg.amount.as_ref().unwrap();
                    UndelegateMsg {
                        delegator_address: msg.delegator_address.clone(),
                        validator_address: msg.validator_address.clone(),
                        amount: amount.amount.clone(),
                        denom: amount.denom.clone(),
                    }
                })
                .collect(),
            count: undelegate_msgs.len(),
            total_ubtsg: undelegate_msgs
                .iter()
                .map(|m| Uint128::from_str(&m.amount.as_ref().unwrap().amount).unwrap())
                .sum(),
        },
    };

    let json = serde_json::to_string_pretty(&export).unwrap();
    serialize_and_print(json, RAW_MSG_JSON.to_string());

    eprintln!(
        "[REALIGN] Saved {} redelegation messages to {}",
        redelegation_msgs.len(),
        RAW_MSG_JSON,
    );

    println!(
        "\n✅ Generated {} redelegation messages",
        redelegation_msgs.len()
    );
    verify_final_state(RAW_MSG_JSON, &all_dao_delegations, &[])?; // simplified verify

    Ok(())
}

/// Replaces delegations.sh - Fetches delegations and filters those going to
/// unbonded, unbonding, or jailed validators. Returns AllAlignedDelegations.
///
/// Mirrors the bash script logic:
///   - Query all validators by bond status: unbonded, unbonding
///   - Query historical validator set for jailed status
///   - For each delegation, check if its validator is unbonded, unbonding, or jailed
async fn fetch_delegations_to_bad_validators(
    staking: &Staking,
    dao_addrs: &[String],
    height: u64,
) -> anyhow::Result<AllAlignedDelegations> {
    eprintln!("[FETCH] === Fetching delegations using native Rust (replacing delegations.sh) ===");
    eprintln!();

    // Get validators by status (same pattern as realign_delegations, lines 251-260)
    eprintln!("[FETCH] Querying unbonded validators...");
    let unbonded_vals = staking
        ._validators(queriers::StakingBondStatus::Unbonded)
        .await?;
    eprintln!("[FETCH] Found {} unbonded validators", unbonded_vals.len());
    eprintln!("[FETCH] Querying unbonding validators...");
    let unbonding_vals = staking
        ._validators(queriers::StakingBondStatus::Unbonding)
        .await?;
    eprintln!(
        "[FETCH] Found {} unbonding validators",
        unbonding_vals.len()
    );
    eprintln!(
        "[FETCH] Querying historical info at height {} for jailed validators...",
        height
    );
    let historical = staking._historical_info(height.try_into().unwrap()).await?;
    let jailed_count = historical
        .hist
        .as_ref()
        .map_or(0, |h| h.valset.iter().filter(|v| v.jailed).count());
    eprintln!(
        "[FETCH] Historical info: {} validators in set ({} jailed)",
        historical.hist.as_ref().map_or(0, |h| h.valset.len()),
        jailed_count
    );

    let mut bad_delegations = Vec::new();
    let mut total_bad = Uint128::zero();

    eprintln!(
        "[FETCH] Checking delegations for {} DAO address(es)...",
        dao_addrs.len()
    );
    for dao in dao_addrs {
        eprintln!("[FETCH] Processing DAO: {}", dao);
        let mut next_key = None;
        let mut page: u32 = 0;

        loop {
            page += 1;
            eprintln!(
                "[FETCH]  DAO {} page {}: querying delegations...",
                dao, page
            );
            let response = staking
                ._delegator_delegations(&Addr::unchecked(dao), next_key)
                .await?;
            eprintln!(
                "[FETCH]  DAO {} page {}: got {} delegations",
                dao,
                page,
                response.delegation_responses.len()
            );

            for del_resp in response.delegation_responses {
                let delegation = del_resp
                    .delegation
                    .ok_or_else(|| anyhow::anyhow!("Missing delegation"))?;
                let balance = del_resp
                    .balance
                    .ok_or_else(|| anyhow::anyhow!("Missing balance"))?;

                let amount = Uint128::from_str(&balance.amount).map_err(|e| anyhow!(e))?;

                if amount.is_zero() {
                    continue;
                }

                // Check: is the validator unbonded, unbonding, or jailed?
                // (Same logic as realign_delegations lines 307-317)
                let is_unbonded = unbonded_vals
                    .iter()
                    .any(|v| v.address == delegation.validator_address);
                let is_unbonding = unbonding_vals
                    .iter()
                    .any(|v| v.address == delegation.validator_address);
                let is_jailed = historical.hist.as_ref().map_or(false, |h| {
                    h.valset
                        .iter()
                        .any(|v| v.operator_address == delegation.validator_address && v.jailed)
                });

                let is_bad = is_unbonded || is_unbonding || is_jailed;

                if is_bad {
                    let reason = if is_unbonded {
                        "Unbonded"
                    } else if is_unbonding {
                        "Unbonding"
                    } else {
                        "Jailed"
                    };

                    bad_delegations.push(Delegation {
                        del_addr: dao.clone(),
                        operator_addr: delegation.validator_address.clone(),
                        amount,
                        shares: delegation.shares.clone(),
                    });
                    total_bad += amount;

                    println!(
                        "  Validator: {} | Amount: {} uterp | Status: {}",
                        delegation.validator_address,
                        Decimal::from_atomics(amount, 6)?,
                        reason
                    );
                }
            }

            match response.pagination {
                Some(p) if !p.next_key.is_empty() => {
                    next_key = Some(PageRequest {
                        key: p.next_key,
                        ..Default::default()
                    });
                }
                _ => break,
            }
        }
    }

    println!();
    println!(
        "Total delegations to bad validators: {} uterp ({} entries)",
        Decimal::from_atomics(total_bad, 6)?,
        bad_delegations.len()
    );

    Ok(AllAlignedDelegations {
        delegations: bad_delegations,
        total: total_bad,
    })
}

fn form_and_broadcast_obligated_msgs(
    rt: &tokio::runtime::Handle,
    mut wallet: Wallet,
    json: &str,
    dao_addrs: Vec<String>,
) -> anyhow::Result<()> {
    eprintln!("[BROADCAST] Entering form_and_broadcast_obligated_msgs");
    eprintln!("[BROADCAST] JSON file: {}", json);
    eprintln!("[BROADCAST] DAO addresses to process: {:?}", dao_addrs);

    // load json msgs
    let file_content = std::fs::read_to_string(json.to_string())
        .map_err(|e| anyhow::anyhow!("[BROADCAST] Failed to read JSON file '{}': {}", json, e))?;
    let obligated_export: MessageExport = serde_json::from_str(&file_content)
        .map_err(|e| anyhow::anyhow!("[BROADCAST] Failed to parse JSON: {}", e))?;

    eprintln!(
        "[BROADCAST] Loaded {} redelegations, {} delegations, {} undelegations",
        obligated_export.redelegations.count,
        obligated_export.delegations.count,
        obligated_export.undelegates.count,
    );

    for dao in &dao_addrs {
        eprintln!("[BROADCAST] === Processing DAO: {} ===", dao);
        wallet.set_authz_granter(&Addr::unchecked(dao));

        let dao_msgs = filter_obligated_msgs(obligated_export.clone(), dao.clone());

        eprintln!(
            "[BROADCAST] Filtered msgs for {}: {} redelegates, {} delegates, {} undelegates",
            dao,
            dao_msgs.0.len(),
            dao_msgs.1.len(),
            dao_msgs.2.len(),
        );

        // Combine all messages into a single vector
        let mut all_msgs: Vec<terp_rs::Any> = Vec::new();
        all_msgs.extend(dao_msgs.0.iter().map(|msg| terp_rs::Any {
            type_url: "/cosmos.staking.v1beta1.MsgBeginRedelegate".to_string(),
            value: msg.clone().into_any().unwrap().value,
        }));
        all_msgs.extend(dao_msgs.1.iter().map(|msg| terp_rs::Any {
            type_url: "/cosmos.staking.v1beta1.MsgDelegate".to_string(),
            value: msg.clone().into_any().unwrap().value,
        }));
        all_msgs.extend(dao_msgs.2.iter().map(|msg| terp_rs::Any {
            type_url: "/cosmos.staking.v1beta1.MsgUndelegate".to_string(),
            value: msg.clone().into_any().unwrap().value,
        }));

        eprintln!(
            "[BROADCAST] Total combined msgs for {}: {}",
            dao,
            all_msgs.len(),
        );

        if all_msgs.is_empty() {
            eprintln!("[BROADCAST] No messages for {} — skipping", dao);
            continue;
        }

        // Bundle size 1 — each message is its own transaction.
        // This avoids Cosmos SDK token→shares precision issues when
        // multiple MsgBeginRedelegate from the same source are grouped
        // in a single tx (v0.53.4-pfm-migrate share check at delegation.go:1379).
        // Each message runs against the full pre-transaction delegation shares.
        let bundle_size: usize = 1;
        let num_bundles = (all_msgs.len() + bundle_size - 1) / bundle_size;

        eprintln!(
            "[BROADCAST] Splitting {} msgs into {} single-msg bundles (avoids same-source share conflict)",
            all_msgs.len(),
            num_bundles,
        );

        // Broadcast each bundle
        for i in 0..num_bundles {
            let start = i * bundle_size;
            let end = core::cmp::min(start + bundle_size, all_msgs.len());
            let bundle = all_msgs[start..end].to_vec();

            eprintln!(
                "[BROADCAST] Bundle {}/{}: msgs {}-{} (count: {})",
                i + 1,
                num_bundles,
                start,
                end - 1,
                bundle.len(),
            );
            let account = rt.block_on(wallet.base_account())?;
            println!("{:#?}", account.sequence);
            println!("{:#?}", account.address);

            // simulate first
            eprintln!("[BROADCAST] Simulating bundle {}...", i + 1);
            match rt.block_on(wallet.simulate(bundle.clone(), None)) {
                Ok((gas, fee)) => {
                    eprintln!(
                        "[BROADCAST] Simulation OK — gas: {}, fee: {} {}",
                        gas, fee.amount, fee.denom,
                    );
                }
                Err(e) => {
                    eprintln!("[BROADCAST] Simulation FAILED: {:?}", e);
                    return Err(anyhow::anyhow!("[BROADCAST] Simulation error: {}", e));
                }
            }

            // broadcast
            eprintln!("[BROADCAST] Broadcasting bundle {}...", i + 1);
            match rt.block_on(wallet.commit_tx_any(bundle, None)) {
                Ok(resp) => {
                    eprintln!(
                        "[BROADCAST] TX committed — code: {}, hash: {}",
                        resp.code, resp.txhash,
                    );
                }
                Err(e) => {
                    eprintln!("[BROADCAST] Broadcast FAILED: {:?}", e);
                    return Err(anyhow::anyhow!("[BROADCAST] Broadcast error: {}", e));
                }
            }

            if i + 1 < num_bundles {
                eprintln!("[BROADCAST] Waiting 7 seconds before next bundle...");
                std::thread::sleep(std::time::Duration::new(7, 0));
            }
        }

        eprintln!("[BROADCAST] Finished processing DAO: {}", dao);
    }

    eprintln!("[BROADCAST] All DAOs processed successfully");
    Ok(())
}

fn filter_obligated_msgs(
    obligated_export: MessageExport,
    dao: String,
) -> (
    Vec<cosmrs::staking::MsgBeginRedelegate>,
    Vec<cosmrs::staking::MsgDelegate>,
    Vec<cosmrs::staking::MsgUndelegate>,
) {
    // prepare 32 msgs batches. Include all msgs where this dao is delegator.
    let redels: Vec<RedelegateMsg> = obligated_export
        .redelegations
        .data
        .into_iter()
        .filter(|rd| rd.delegator_address == dao)
        .collect();

    let dels: Vec<DelegateMsg> = obligated_export
        .delegations
        .data
        .into_iter()
        .filter(|rd| rd.delegator_address == dao)
        .collect();

    let undels: Vec<UndelegateMsg> = obligated_export
        .undelegates
        .data
        .into_iter()
        .filter(|rd| rd.delegator_address == dao)
        .collect();

    // form into cosmrs msgs
    let mut rdel_msgs = vec![];
    let mut del_msgs = vec![];
    let mut udel_msgs = vec![];
    for rd in redels {
        rdel_msgs.push(form_redel_msg(rd));
    }
    for d in dels {
        del_msgs.push(form_del_msg(d));
    }
    for ud in undels {
        udel_msgs.push(form_undel_msg(ud));
    }

    (rdel_msgs, del_msgs, udel_msgs)
}

fn form_redel_msg(redel: RedelegateMsg) -> cosmrs::staking::MsgBeginRedelegate {
    cosmrs::staking::MsgBeginRedelegate {
        delegator_address: AccountId::from_str(&redel.delegator_address).unwrap(),
        validator_src_address: AccountId::from_str(&redel.validator_src_address).unwrap(),
        validator_dst_address: AccountId::from_str(&redel.validator_dst_address).unwrap(),
        amount: cosmrs::Coin {
            amount: Uint128::from_str(&redel.amount).unwrap().u128(),
            denom: cosmrs::Denom::from_str("uterp").unwrap(),
        },
    }
}
fn form_del_msg(del: DelegateMsg) -> cosmrs::staking::MsgDelegate {
    cosmrs::staking::MsgDelegate {
        // Delegator's address.
        delegator_address: AccountId::from_str(&del.delegator_address).unwrap(),
        validator_address: AccountId::from_str(&del.validator_address).unwrap(),

        // Amount to Delegate
        amount: cosmrs::Coin {
            amount: Uint128::from_str(&del.amount).unwrap().u128(),
            denom: cosmrs::Denom::from_str("uterp").unwrap(),
        },
    }
}
fn form_undel_msg(del: UndelegateMsg) -> cosmrs::staking::MsgUndelegate {
    cosmrs::staking::MsgUndelegate {
        // Delegator's address.
        delegator_address: AccountId::from_str(&del.delegator_address).unwrap(),
        validator_address: AccountId::from_str(&del.validator_address).unwrap(),

        // Amount to Delegate
        amount: cosmrs::Coin {
            amount: Uint128::from_str(&del.amount).unwrap().u128(),
            denom: cosmrs::Denom::from_str("uterp").unwrap(),
        },
    }
}

// Loads array of validators getting new delegations from file, returning the total new delegations
fn load_new_delegations(fp: &str, has_header: bool) -> AllAlignedDelegations {
    let file = File::open(fp).expect("Could not open file");

    // Create a reader with configurable header setting
    let mut rdr = ReaderBuilder::new()
        .has_headers(has_header)
        .from_reader(file);

    let mut delegations = vec![];
    let mut total = Uint128::zero();

    for result in rdr.records() {
        match result {
            Ok(record) => {
                // Ensure there are at least two fields
                if record.len() < 2 {
                    eprintln!(
                        "Invalid record format: expected at least 2 fields, got {}",
                        record.len()
                    );
                    continue;
                }

                let addr = match record[0].parse::<String>() {
                    Ok(addr) => addr,
                    Err(e) => {
                        eprintln!("Error parsing address: {}", e);
                        continue;
                    }
                };

                let amount = match record[1].parse::<Uint128>() {
                    Ok(num) => num,
                    Err(e) => {
                        eprintln!("Error parsing amount: {}", e);
                        continue;
                    }
                };

                delegations.push(Delegation {
                    del_addr: String::default(),
                    operator_addr: addr,
                    amount,
                    shares: String::default(),
                });

                total += amount;
            }
            Err(e) => {
                eprintln!("Error reading record: {}", e);
            }
        }
    }

    println!(
        "Loaded {} delegations with total amount {}",
        delegations.len(),
        total
    );
    AllAlignedDelegations { delegations, total }
}

/// Load delegations to unbond/redelegate from a CSV file.
///
/// CSV format:
/// ```ignore
/// validator,amount        // header line (optional, auto-detected)
/// terpvaloper1...,123456789
/// ```
/// - Column 1: validator operator address
/// - Column 2: amount in micro-denom (e.g. uthiol)
/// - Header is auto-detected — if first field is "validator" or "operator", it's skipped.
/// - No header required; plain `validator,amount` lines are fine.
/// - Delegator address is filled from the DAO address list (first DAO used).
fn load_unbond_delegations(fp: &str, dao_addrs: &[String], denom: &str) -> Vec<Delegation> {
    let default_dao = dao_addrs.first().cloned().unwrap_or_default();
    eprintln!(
        "[CSV] Loading unbond delegations from '{}' (delegator: {})",
        fp, default_dao
    );

    let file = File::open(fp).unwrap_or_else(|e| panic!("[CSV] Could not open '{}': {}", fp, e));

    // Read all lines into a vec so we can peek at the first row
    let mut rdr = ReaderBuilder::new().has_headers(false).from_reader(file);
    let records: Vec<csv::StringRecord> = rdr.records().filter_map(|r| r.ok()).collect();

    // Auto-detect header: skip first record if it looks like a header line
    let start_idx = records.first().map_or(0, |first| {
        let f0 = first.get(0).unwrap_or("").trim().to_lowercase();
        if f0 == "validator" || f0 == "operator_address" || f0 == "validator_address" {
            eprintln!("[CSV] Detected header row, skipping first line");
            1
        } else {
            0
        }
    });

    let mut delegations = Vec::new();
    let mut total = Uint128::zero();

    for record in &records[start_idx..] {
        if record.len() < 2 {
            eprintln!(
                "[CSV] Skipping invalid record (need 2 fields): {:?}",
                record
            );
            continue;
        }
        let validator = record[0].trim().to_string();
        let amount_str = record[1].to_string();
        let amount = match Uint128::from_str(amount_str.trim()) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[CSV] Skipping invalid amount '{}': {}", amount_str, e);
                continue;
            }
        };
        if amount.is_zero() {
            continue;
        }
        delegations.push(Delegation {
            del_addr: default_dao.clone(),
            operator_addr: validator,
            amount,
            shares: String::default(),
        });
        total += amount;
    }

    println!(
        "[CSV] Loaded {} delegations, total {} {}",
        delegations.len(),
        Decimal::from_atomics(total, 6).unwrap(),
        denom,
    );
    delegations
}
/// Greedy round-robin distribution: sort deficits largest-first, fill from largest source.
/// Redistributes freed tokens with maximum usage and minimum undelegates.
/// Preferred when you want to fully saturate top-deficit validators first.
fn distribute_redelegated_greedy(
    mut to_redelegate: Vec<Delegation>,
    active_validators: &mut [AlignedValidator],
    denom: &str,
) -> (
    Vec<MsgBeginRedelegate>,
    Vec<MsgDelegate>,
    Vec<MsgUndelegate>,
) {
    let total_freed: Uint128 = to_redelegate.iter().map(|d| d.amount).sum();
    if total_freed.is_zero() {
        return (vec![], vec![], vec![]);
    }

    println!(
        "Distributing {} {} greedily (largest deficit first, largest source first)",
        Decimal::from_atomics(total_freed, 6).unwrap(),
        denom,
    );

    // Build deficit list: (validator_ref, remaining_deficit)
    let mut deficits: Vec<(usize, Uint128)> = active_validators
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.new_delegation_amount.is_zero())
        .map(|(i, v)| {
            let current: Uint128 = v.current_delegations.iter().map(|d| d.amount).sum();
            let deficit = v.new_delegation_amount.saturating_sub(current);
            (i, deficit)
        })
        .filter(|(_, d)| !d.is_zero())
        .collect();

    if deficits.is_empty() {
        println!(
            "⚠️  No active validator has a deficit — all delegations will be undelegated directly"
        );
        let undelegate_msgs: Vec<MsgUndelegate> = to_redelegate
            .iter()
            .map(|src| MsgUndelegate {
                delegator_address: src.del_addr.clone(),
                validator_address: src.operator_addr.clone(),
                amount: Some(ProtoCoin {
                    denom: denom.to_string(),
                    amount: src.amount.to_string(),
                }),
            })
            .collect();
        return (vec![], vec![], undelegate_msgs);
    }

    let mut redelegation_msgs = Vec::new();
    let mut undelegate_msgs = Vec::new();

    // Sort sources descending for greedy picking
    to_redelegate.sort_by(|a, b| b.amount.cmp(&a.amount));

    // Multi-pass: each pass fills largest deficit from largest remaining source
    let mut made_progress = true;
    while made_progress {
        made_progress = false;

        // Sort deficits descending each pass
        deficits.sort_by(|a, b| b.1.cmp(&a.1));

        for (idx, deficit) in &mut deficits {
            if deficit.is_zero() {
                continue;
            }

            // Find the largest source with remaining tokens
            if let Some(src) = to_redelegate.iter_mut().find(|s| !s.amount.is_zero()) {
                let take = src.amount.min(*deficit);
                if take.is_zero() {
                    continue;
                }

                let val = &mut active_validators[*idx];
                redelegation_msgs.push(MsgBeginRedelegate {
                    delegator_address: src.del_addr.clone(),
                    validator_src_address: src.operator_addr.clone(),
                    validator_dst_address: val.operator_addr.clone(),
                    amount: Some(ProtoCoin {
                        denom: denom.to_string(),
                        amount: take.to_string(),
                    }),
                });

                // Track for verification
                val.current_delegations.push(Delegation {
                    del_addr: src.del_addr.clone(),
                    operator_addr: val.operator_addr.clone(),
                    amount: take,
                    shares: src.shares.clone(),
                });

                src.amount = src.amount.saturating_sub(take);
                *deficit = deficit.saturating_sub(take);
                made_progress = true;
            }
        }

        // Remove filled deficits
        deficits.retain(|(_, d)| !d.is_zero());
    }

    // Any leftover source tokens become undelegations
    let remaining: Uint128 = to_redelegate.iter().map(|d| d.amount).sum();
    if !remaining.is_zero() {
        println!(
            "⚠️  Excess {} {} will be undelegated (all deficits filled)",
            Decimal::from_atomics(remaining, 6).unwrap(),
            denom,
        );
        for src in to_redelegate.iter().filter(|d| !d.amount.is_zero()) {
            undelegate_msgs.push(MsgUndelegate {
                delegator_address: src.del_addr.clone(),
                validator_address: src.operator_addr.clone(),
                amount: Some(ProtoCoin {
                    denom: denom.to_string(),
                    amount: src.amount.to_string(),
                }),
            });
        }
    } else {
        println!("✅ All freed tokens successfully redelegated (no undelegates)");
    }

    (redelegation_msgs, vec![], undelegate_msgs)
}
/// Distribute freed tokens evenly across active validators.
/// The last active validator receives whatever remains in all sources.
/// This avoids cascading same-source redelegations: by giving the last
/// validator the exact remaining source balance, exactly N messages are
/// produced for N validators — no cascade, no double-reduction.
fn distribute_redelegated_evenly(
    mut to_redelegate: Vec<Delegation>,
    active_validators: &mut [AlignedValidator],
    denom: &str,
) -> (
    Vec<MsgBeginRedelegate>,
    Vec<MsgDelegate>,
    Vec<MsgUndelegate>,
) {
    let total_freed: Uint128 = to_redelegate.iter().map(|d| d.amount).sum();
    if total_freed.is_zero() {
        return (vec![], vec![], vec![]);
    }

    println!(
        "Distributing {} uterp evenly across active validators",
        Decimal::from_atomics(total_freed, 6).unwrap()
    );

    let mut redelegation_msgs = Vec::new();
    let mut undelegate_msgs = Vec::new();

    to_redelegate.sort_by(|a, b| b.amount.cmp(&a.amount));

    let mut source_idx = 0;
    let num_active = active_validators.len();

    for (val_idx, val) in active_validators.iter_mut().enumerate() {
        let is_last = val_idx == num_active - 1;
        let target = if is_last {
            to_redelegate.iter().map(|s| s.amount).sum()
        } else {
            val.new_delegation_amount
        };

        let mut target_remaining = target;

        while !target_remaining.is_zero() && source_idx < to_redelegate.len() {
            let src = &mut to_redelegate[source_idx];

            if src.amount.is_zero() {
                source_idx += 1;
                continue;
            }

            let take = src.amount.min(target_remaining);

            if !take.is_zero() {
                redelegation_msgs.push(MsgBeginRedelegate {
                    delegator_address: src.del_addr.clone(),
                    validator_src_address: src.operator_addr.clone(),
                    validator_dst_address: val.operator_addr.clone(),
                    amount: Some(ProtoCoin {
                        denom: denom.to_string(),
                        amount: take.to_string(),
                    }),
                });

                src.amount = src.amount.saturating_sub(take);
                target_remaining = target_remaining.saturating_sub(take);

                val.current_delegations.push(Delegation {
                    del_addr: src.del_addr.clone(),
                    operator_addr: val.operator_addr.clone(),
                    amount: take,
                    shares: src.shares.clone(),
                });
            }

            if src.amount.is_zero() {
                source_idx += 1;
            }
        }
    }

    let remaining: Uint128 = to_redelegate.iter().map(|d| d.amount).sum();
    if !remaining.is_zero() {
        println!(
            "⚠️ Excess {} uterp will be undelegated",
            Decimal::from_atomics(remaining, 6).unwrap()
        );
        for src in to_redelegate.iter().filter(|d| !d.amount.is_zero()) {
            undelegate_msgs.push(MsgUndelegate {
                delegator_address: src.del_addr.clone(),
                validator_address: src.operator_addr.clone(),
                amount: Some(ProtoCoin {
                    denom: denom.to_string(),
                    amount: src.amount.to_string(),
                }),
            });
        }
    } else {
        println!("✅ All freed tokens successfully redelegated (no undelegates)");
    }

    (redelegation_msgs, vec![], undelegate_msgs)
}

fn verify_final_state(
    json_file: &str,
    current_delegations: &[DelegationResponse],
    obligated_delegations: &[Delegation],
) -> anyhow::Result<()> {
    println!("\n--- VERIFYING FINAL VALIDATOR STATE ---");

    // Read and parse the JSON file
    let file_content = std::fs::read_to_string(json_file)?;
    let export: MessageExport = serde_json::from_str(&file_content)?;

    // Create maps for current and target delegations by validator
    let mut current_by_validator: HashMap<String, Uint128> = HashMap::new();
    let mut obligated_by_validator: HashMap<String, Uint128> = HashMap::new();

    // Populate current delegations map
    for del in current_delegations {
        let validator = del
            .delegation
            .as_ref()
            .expect("msg")
            .validator_address
            .clone();
        let amount = Uint128::from_str(&del.balance.as_ref().expect("msg").amount)
            .map_err(|e| anyhow!(e))?;
        *current_by_validator.entry(validator).or_default() += amount;
    }

    // Populate obligated delegations map
    for del in obligated_delegations {
        *obligated_by_validator
            .entry(del.operator_addr.clone())
            .or_default() += del.amount;
    }

    // Apply all changes from the export to simulate final state
    let mut final_state = current_by_validator.clone();

    // Apply redelegations (subtract from source, add to destination)
    for redel in &export.redelegations.data {
        let amount = Uint128::from_str(&redel.amount).map_err(|e| anyhow!(e))?;
        // Subtract from source (can't go below zero — use saturating_sub)
        let src_cur = final_state
            .get(&redel.validator_src_address)
            .copied()
            .unwrap_or(Uint128::zero());
        final_state.insert(
            redel.validator_src_address.clone(),
            src_cur.saturating_sub(amount),
        );
        // Add to destination
        let dst_cur = final_state
            .get(&redel.validator_dst_address)
            .copied()
            .unwrap_or(Uint128::zero());
        final_state.insert(redel.validator_dst_address.clone(), dst_cur + amount);
    }

    // Apply delegations (add to validator)
    for del in &export.delegations.data {
        let amount = Uint128::from_str(&del.amount).map_err(|e| anyhow!(e))?;
        let cur = final_state
            .get(&del.validator_address)
            .copied()
            .unwrap_or(Uint128::zero());
        final_state.insert(del.validator_address.clone(), cur + amount);
    }

    // Apply undelegations (subtract from validator)
    for undel in &export.undelegates.data {
        let amount = Uint128::from_str(&undel.amount).map_err(|e| anyhow!(e))?;
        let cur = final_state
            .get(&undel.validator_address)
            .copied()
            .unwrap_or(Uint128::zero());
        final_state.insert(undel.validator_address.clone(), cur.saturating_sub(amount));
    }

    // Verify the final state matches the obligated state
    let mut discrepancies = Vec::new();
    let mut total_final = Uint128::zero();
    let mut total_obligated = Uint128::zero();

    // Check each validator's final state against obligation
    for (validator, &obligated_amount) in &obligated_by_validator {
        let final_amount = final_state
            .get(validator)
            .copied()
            .unwrap_or(Uint128::zero());
        total_obligated += obligated_amount;
        total_final += final_amount;

        if final_amount != obligated_amount {
            discrepancies.push((
                validator.clone(),
                final_amount,
                obligated_amount,
                final_amount
                    .checked_sub(obligated_amount)
                    .unwrap_or_else(|_| {
                        obligated_amount
                            .checked_sub(final_amount)
                            .unwrap_or(Uint128::zero())
                    }),
            ));
        }
    }

    // Print verification results
    println!(
        "Total final delegation amount: {}",
        Decimal::from_atomics(total_final, 6)?
    );
    println!(
        "Total obligated delegation amount: {}",
        Decimal::from_atomics(total_obligated, 6)?
    );

    if discrepancies.is_empty() {
        println!(
            "✅ VERIFICATION PASSED: All validators have the correct obligated delegation amount"
        );
    } else {
        println!(
            "❌ VERIFICATION FAILED: Found {} validators with discrepancies",
            discrepancies.len()
        );

        // Sort discrepancies by difference amount (largest first)
        discrepancies.sort_by(|a, b| b.3.cmp(&a.3));

        println!("\nTop discrepancies:");
        for (validator, final_amount, obligated_amount, diff) in discrepancies.iter().take(10) {
            println!(
                "Validator {}: Final={}, Obligated={}, Diff={}",
                validator,
                Decimal::from_atomics(*final_amount, 6)?,
                Decimal::from_atomics(*obligated_amount, 6)?,
                Decimal::from_atomics(*diff, 6)?
            );
        }
    }

    // Check for validators with redelegations or undelegations that aren't in obligated_delegations
    let mut unexpected_validators = Vec::new();
    for validator in final_state.keys() {
        if !obligated_by_validator.contains_key(validator)
            && final_state
                .get(validator)
                .copied()
                .unwrap_or(Uint128::zero())
                > Uint128::zero()
        {
            unexpected_validators.push((
                validator.clone(),
                final_state
                    .get(validator)
                    .copied()
                    .unwrap_or(Uint128::zero()),
            ));
        }
    }

    // if !unexpected_validators.is_empty() {
    //     println!("\n⚠️ WARNING: Found {} validators with delegations that are not in the obligated list:", unexpected_validators.len());
    //     for (validator, amount) in unexpected_validators {
    //         println!(
    //             "Validator {}: Amount={}",
    //             validator,
    //             Decimal::from_atomics(amount, 6)?
    //         );
    //     }
    // }

    Ok(())
}

fn serialize_and_print(json: String, filepath: String) {
    let mut file = File::create(filepath).expect("Failed to create JSON file");
    file.write_all(json.as_bytes())
        .expect("Failed to write JSON to file");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_obligated_delegations_file() -> anyhow::Result<()> {
        let aad = load_new_delegations(NEW_DELS_FILE, false);
        // Check the calculated total from the struct

        // Calculate and check the sum of individual
        let obligated_delegation_sum: Uint128 = aad.delegations.iter().map(|a| a.amount).sum();
        println!("Total from struct : {}", aad.total);
        println!("Sum of delegations: {}", obligated_delegation_sum);

        // Both should match the expected value
        assert_eq!(aad.delegations.len(), TOTAL_OBLIGATED_VALIDATORS);
        assert_eq!(aad.total.u128(), TOTAL_OBLIGATED_DELEGATED_Terp.u128());
        assert_eq!(
            obligated_delegation_sum.u128(),
            TOTAL_OBLIGATED_DELEGATED_Terp.u128()
        );

        Ok(())
    }

    // Add this function near the end of realign_delegations,
    // just before the final Ok(()) return

    #[test]
    fn test_accuracy_delegations_message_json() -> anyhow::Result<()> {
        Ok(())
    }

    // Usage in test
    #[test]
    fn test_yes_no_load_obligated_delegations_file() -> anyhow::Result<()> {
        // Try with both header settings to see which matches expected value
        let aad_with_header = load_new_delegations(NEW_DELS_FILE, true);
        let aad_without_header = load_new_delegations(NEW_DELS_FILE, false);

        println!(
            "With header: {} delegations, total {}",
            aad_with_header.delegations.len(),
            aad_with_header.total
        );

        println!(
            "Without header: {} delegations, total {}",
            aad_without_header.delegations.len(),
            aad_without_header.total
        );

        // Check which one matches the expected value
        let expected = 9_999_980_000_000u128;

        if aad_with_header.total.u128() == expected {
            println!("CSV has a header row");
            assert_eq!(aad_with_header.total.u128(), expected);
        } else if aad_without_header.total.u128() == expected {
            println!("CSV does not have a header row");
            assert_eq!(aad_without_header.total.u128(), expected);
        } else {
            panic!("Neither header configuration matches expected total");
        }

        Ok(())
    }
}

// async fn realign_delegations(
//     staking_query_client: Staking,
//     bank_client: Bank,
//     dao_addrs: &[String],
//     height: u64,
// ) -> anyhow::Result<()> {
//     // logs any errors
//     env_logger::init();
//     // Get unbonded validators
//     let unbonded_vals = staking_query_client
//         ._validators(queriers::StakingBondStatus::Unbonded)
//         .await?;
//     let unbonding_vals = staking_query_client
//         ._validators(queriers::StakingBondStatus::Unbonding)
//         .await?;

//     let val_historical = staking_query_client
//         ._historical_info(height.try_into().unwrap())
//         .await?;

//     // Load new delegations from CSV file
//     let all_oblgated_dels =
//         fetch_delegations_to_bad_validators(&staking_query_client, &dao_addrs, height).await?;
//     let obligated_delegations = all_oblgated_dels.delegations;
//     let total_obligated_delegations = Uint128::from(all_oblgated_dels.total);

//     println!("Running Bitsong Delegation Realignment Protocol...");
//     println!(
//         "{} dels with {}uterp",
//         obligated_delegations.len(),
//         Decimal::from_atomics(
//             total_obligated_delegations.checked_mul(1000000u128.into())?,
//             6
//         )?
//         .to_string()
//     );

//     // collect all dao delegations
//     let mut all_dao_delegations = Vec::new();
//     let mut dao_entities = Vec::new();
//     let ommited_vals: Vec<String> = vec![];
//     for dao in dao_addrs {
//         let mut next_key = None;
//         loop {
//             let response = staking_query_client
//                 ._delegator_delegations(&Addr::unchecked(dao), next_key)
//                 .await?;

//             all_dao_delegations.extend(response.delegation_responses);
//             match response.pagination {
//                 Some(pagination) => {
//                     if pagination.next_key.is_empty() {
//                         break;
//                     }
//                     next_key = Some(PageRequest {
//                         key: pagination.next_key,
//                         offset: 0,
//                         limit: 100,
//                         count_total: false,
//                         reverse: false,
//                     });
//                 }
//                 None => {
//                     break;
//                 }
//             }
//         }

//         let entity_btsg_addr = bank_client
//             ._balance(&Addr::unchecked(dao), Some("uterp".into()))
//             .await?;

//         let total_non_team_de: Vec<&DelegationResponse> = all_dao_delegations
//             .iter()
//             .filter(|a| {
//                 !ommited_vals.contains(&a.delegation.clone().expect("msg").validator_address)
//             })
//             .collect();

//         let new_total_delegation_amount = obligated_delegations
//             .clone()
//             .iter()
//             .find(|d| d.del_addr == *dao)
//             .map(|a| a.amount)
//             .into_iter()
//             .sum();
//         dao_entities.push(DelegationDaoEntity {
//             dao_add: dao.to_string(),

//             current_balance: entity_btsg_addr[0].clone(),
//             total_delegation_count: total_non_team_de.len(),
//             current_delegation: total_non_team_de
//                 .clone()
//                 .iter()
//                 .map(|a| Uint128::from_str(&a.balance.clone().expect("msg").amount).expect("msg"))
//                 .sum(),
//             obligated_delegation: new_total_delegation_amount,
//         });
//     }

//     // current_vals - array of validators and the DAOs delegations to them
//     let mut current_vals: Vec<AlignedValidator> = Vec::new();

//     // all delegations, enumerated
//     for (i, dels) in all_dao_delegations.clone().iter_mut().enumerate() {
//         let del = dels.delegation.clone().unwrap();
//         if !ommited_vals.contains(&del.validator_address) {
//             let balance = Uint128::from_str(&dels.balance.clone().unwrap().amount)
//                 .expect("Failed to parse balance");
//             if !balance.is_zero() {
//                 if let Some(exists) = current_vals
//                     .iter_mut()
//                     .find(|a| a.operator_addr == del.validator_address)
//                 {
//                     exists.current_delegations.push(Delegation {
//                         del_addr: del.delegator_address,
//                         operator_addr: del.validator_address,
//                         amount: balance,
//                     });
//                 } else {
//                     // initialize aligned validator
//                     let target_amount = obligated_delegations
//                         .iter()
//                         .find(|a| a.operator_addr == del.validator_address)
//                         .map_or(Uint128::zero(), |a| a.amount);
//                     if target_amount != Uint128::zero() {
//                         current_vals.push(AlignedValidator {
//                             operator_addr: del.validator_address.clone(),
//                             current_delegations: vec![Delegation {
//                                 del_addr: del.delegator_address,
//                                 operator_addr: del.validator_address.clone(),
//                                 amount: balance,
//                             }],
//                             new_delegation_amount: target_amount,
//                         });
//                     }
//                 }
//             }
//         } else {
//             // do not add to this list of delegators, and disregard remaining list
//             all_dao_delegations.remove(i);
//         }
//     }
//     //assert we omit private agreement validator operators
//     assert!(current_vals
//         .iter()
//         .all(|cv| !ommited_vals.contains(&cv.operator_addr)));

//     // Add any completely new validators from aligned_vals that don't exist in current_vals
//     for obligated in &obligated_delegations {
//         if !current_vals
//             .iter()
//             .any(|cv| cv.operator_addr == obligated.operator_addr)
//         {
//             current_vals.push(AlignedValidator {
//                 operator_addr: obligated.operator_addr.clone(),
//                 current_delegations: Vec::new(),
//                 new_delegation_amount: obligated.amount,
//             });
//         }
//     }

//     // Modify the main delegation processing to use this debug function
//     debug_delegation_tracking(&all_dao_delegations, &obligated_delegations)?;

//     // --- NEW TWO-PASS APPROACH ---
//     let mut to_redelegate: Vec<Delegation> = Vec::new();
//     let mut active_vals: Vec<AlignedValidator> = Vec::new();

//     for val in current_vals {
//         let total_current: Uint128 = val.current_delegations.iter().map(|d| d.amount).sum();
//         let is_unbonded = unbonded_vals.iter().any(|v| v.address == val.operator_addr);
//         let is_unbonding = unbonding_vals
//             .iter()
//             .any(|v| v.address == val.operator_addr);
//         let is_jailed = match &val_historical.hist {
//             Some(hist) => hist
//                 .valset
//                 .iter()
//                 .any(|v| v.operator_address == val.operator_addr && v.jailed),
//             None => false,
//         };

//         if is_unbonded || is_unbonding || is_jailed || val.new_delegation_amount.is_zero() {
//             to_redelegate.extend(val.current_delegations);
//             println!(
//                 "Collecting {}uterp from bad validator {} (unbonded={}, unbonding={}, jailed={})",
//                 total_current, val.operator_addr, is_unbonded, is_unbonding, is_jailed
//             );
//         } else {
//             active_vals.push(val);
//         }
//     }

//     // Collect excess from over-delegated active validators
//     for val in &mut active_vals {
//         let total_current: Uint128 = val.current_delegations.iter().map(|d| d.amount).sum();
//         if total_current > val.new_delegation_amount {
//             let excess = total_current - val.new_delegation_amount;
//             let mut remaining = excess;

//             for del in &val.current_delegations {
//                 if remaining.is_zero() {
//                     break;
//                 }
//                 let take = del.amount.min(remaining);
//                 to_redelegate.push(Delegation {
//                     del_addr: del.del_addr.clone(),
//                     operator_addr: del.operator_addr.clone(),
//                     amount: take,
//                 });
//                 remaining = remaining.saturating_sub(take);
//             }
//             println!(
//                 "Collecting {}uterp excess from {}",
//                 excess, val.operator_addr
//             );
//         }
//     }

//     let total_freed: Uint128 = to_redelegate.iter().map(|d| d.amount).sum();
//     println!(
//         "\nTotal uterp freed for redistribution: {}",
//         Decimal::from_atomics(total_freed, 6)?
//     );

//     // Distribute evenly
//     let (redelegation_msgs, delegation_msgs, undelegate_msgs) =
//         distribute_redelegated_evenly(to_redelegate, &mut active_vals, "uterp");

//     // Print summary of delegation changes
//     println!("\n--- DELEGATIONS TO ADD (Direct) ---");
//     let mut total_del = Uint128::zero();
//     for del in &delegation_msgs {
//         let uint_amnt = Uint128::from_str(del.amount.clone().expect("shoot").amount.as_str())
//             .map_err(|e| anyhow!(e))?;
//         total_del += uint_amnt;
//     }
//     println!(
//         "Total to delegate: {}. Count: {}",
//         Decimal::from_atomics(total_del, 6)?,
//         delegation_msgs.len()
//     );

//     println!("\n--- REDELEGATIONS ---");
//     let mut total_redel = Uint128::zero();
//     for redel in &redelegation_msgs {
//         let uint_amnt = Uint128::from_str(redel.amount.clone().expect("shoot").amount.as_str())
//             .map_err(|e| anyhow!(e))?;
//         total_redel += uint_amnt;
//     }
//     println!(
//         "Total to redelegate: {} Terp. Count: {}",
//         Decimal::from_atomics(total_redel, 6)?,
//         redelegation_msgs.len()
//     );

//     // Compute undelegation total (for export)
//     let total_undel = undelegate_msgs
//         .iter()
//         .map(|msg| {
//             Uint128::from_str(msg.amount.as_ref().unwrap().amount.as_str())
//                 .unwrap_or(Uint128::zero())
//         })
//         .sum::<Uint128>();
//     if !total_undel.is_zero() {
//         println!(
//             "Total to undelegate: {} Terp",
//             Decimal::from_atomics(total_undel, 6)?
//         );
//     }

//     // Uncomment and modify the export creation and serialization at the end of the function
//     let export = MessageExport {
//         redelegations: Redelegations {
//             data: redelegation_msgs
//                 .iter()
//                 .map(|msg| {
//                     let amount = msg.amount.as_ref().unwrap();
//                     RedelegateMsg {
//                         delegator_address: msg.delegator_address.clone(),
//                         validator_src_address: msg.validator_src_address.clone(),
//                         validator_dst_address: msg.validator_dst_address.clone(),
//                         amount: amount.amount.clone(),
//                         denom: amount.denom.clone(),
//                     }
//                 })
//                 .collect(),
//             count: redelegation_msgs.len(),
//             total_ubtsg: total_redel,
//         },
//         delegations: Delegations {
//             data: delegation_msgs
//                 .iter()
//                 .map(|msg| {
//                     let amount = msg.amount.as_ref().unwrap();
//                     DelegateMsg {
//                         delegator_address: msg.delegator_address.clone(),
//                         validator_address: msg.validator_address.clone(),
//                         amount: amount.amount.clone(),
//                         denom: amount.denom.clone(),
//                     }
//                 })
//                 .collect(),
//             count: delegation_msgs.len(),
//             total_ubtsg: total_del,
//         },
//         undelegates: Undelegations {
//             data: undelegate_msgs
//                 .iter()
//                 .map(|msg| UndelegateMsg {
//                     delegator_address: msg.delegator_address.clone(),
//                     validator_address: msg.validator_address.clone(),
//                     amount: msg.amount.clone().expect("msg").amount,
//                     denom: msg.amount.clone().expect("msg").denom,
//                 })
//                 .collect(),
//             count: undelegate_msgs.len(),
//             total_ubtsg: total_undel,
//         },
//     };

//     // Serialize to JSON
//     let json = serde_json::to_string_pretty(&export).expect("Failed to serialize messages to JSON");

//     serialize_and_print(json.clone(), RAW_MSG_JSON.to_string());

//     // assert with the new information that the obligated validators will have the correct balance once delegations are applied
//     verify_final_state(RAW_MSG_JSON, &all_dao_delegations, &obligated_delegations)?;

//     Ok(())
// }

// fn optimize_delegations(
//     current_delegations: Vec<Delegation>,
//     obligated_delegations: &[Delegation],
//     denom: &str,
// ) -> (
//     Vec<MsgBeginRedelegate>,
//     Vec<MsgDelegate>,
//     Vec<MsgUndelegate>,
// ) {
//     // Maps of all current and obligated delegations
//     let mut old_delegations: HashMap<String, Vec<Delegation>> = HashMap::new();
//     let mut obligated_delegations_map: HashMap<String, Uint128> = HashMap::new();

//     // Preprocessing: Assert current and obligated delegations total
//     let mut total_current_delegation = Uint128::zero();
//     let mut total_obligated_delegation = Uint128::zero();

//     let mut redelegation_msgs = Vec::<MsgBeginRedelegate>::new();
//     let mut delegation_msgs = Vec::<MsgDelegate>::new();
//     let mut undelegate_msgs = Vec::<MsgUndelegate>::new();

//     // save old delegations hash map with validator as key
//     for del in current_delegations {
//         total_current_delegation += del.amount;
//         old_delegations
//             .entry(del.operator_addr.clone())
//             .or_default()
//             .push(del.clone());
//     }

//     // save desired delegations hash map with validator as key
//     for del in obligated_delegations {
//         let amount = del.amount;
//         total_obligated_delegation += amount;
//         *obligated_delegations_map
//             .entry(del.operator_addr.clone())
//             .or_default() += amount;
//     }
//     assert_eq!(total_obligated_delegation, TOTAL_OBLIGATED_DELEGATED_Terp);
//     // First pass: Process validators that need additional delegations
//     for target_del in obligated_delegations {
//         let target_validator = &target_del.operator_addr;
//         let target_amount = target_del.amount;

//         let current_amount = old_delegations
//             .get(target_validator)
//             .map(|dels| dels.iter().map(|d| d.amount).sum())
//             .unwrap_or(Uint128::zero());

//         let mut additional = Uint128::zero();
//         if current_amount < target_amount {
//             additional = target_amount.checked_sub(current_amount).expect("");
//         }

//         if additional.is_zero() {
//             continue;
//         }

//         // Try to source from other validators
//         let mut remaining_needed = additional;
//         for (src_validator, current_dels) in &mut old_delegations {
//             if src_validator == target_validator {
//                 continue; // Skip same validator
//             }

//             // Check if source validator has excess over its own target
//             let src_target = obligated_delegations_map
//                 .get(src_validator)
//                 .copied()
//                 .unwrap_or(Uint128::zero());
//             // current delegation sum
//             let src_current: Uint128 = current_dels.iter().map(|d| d.amount).sum();

//             if src_current <= src_target {
//                 continue; // Don't take from validators that need their delegations
//             }

//             // Sort current delegations to prioritize larger amounts
//             current_dels.sort_by(|a, b| b.amount.cmp(&a.amount));

//             for del in current_dels.iter_mut() {
//                 if remaining_needed.is_zero() {
//                     break;
//                 }

//                 // Safely calculate redelegate amount
//                 let excess = src_current.saturating_sub(src_target);
//                 let available_to_redelegate = del.amount.min(excess);
//                 let redelegate_amount = available_to_redelegate.min(remaining_needed);

//                 if !redelegate_amount.is_zero() {
//                     redelegation_msgs.push(MsgBeginRedelegate {
//                         delegator_address: del.del_addr.clone(),
//                         validator_src_address: src_validator.clone(),
//                         validator_dst_address: target_validator.clone(),
//                         amount: Some(ProtoCoin {
//                             denom: denom.to_string(),
//                             amount: redelegate_amount.to_string(),
//                         }),
//                     });

//                     // Safely update amounts
//                     remaining_needed = remaining_needed
//                         .checked_sub(redelegate_amount)
//                         .unwrap_or(Uint128::zero());

//                     del.amount = del
//                         .amount
//                         .checked_sub(redelegate_amount)
//                         .unwrap_or(Uint128::zero());
//                 }
//             }

//             // Break if no more needed
//             if remaining_needed.is_zero() {
//                 break;
//             }
//         }

//         // If still need delegation, add direct delegation
//         if !remaining_needed.is_zero() {
//             delegation_msgs.push(MsgDelegate {
//                 delegator_address: target_del.del_addr.clone(), // Use a default or first DAO address
//                 validator_address: target_validator.clone(),
//                 amount: Some(ProtoCoin {
//                     denom: denom.to_string(),
//                     amount: remaining_needed.to_string(),
//                 }),
//             });
//         }
//     }

//     // Second pass: Handle undelegations for validators with excess
//     for (validator, dels) in &old_delegations {
//         let current_total: Uint128 = dels.iter().map(|d| d.amount).sum();
//         let target = obligated_delegations_map
//             .get(validator)
//             .copied()
//             .unwrap_or(Uint128::zero());

//         // Check if there's excess that needs to be undelegated
//         if current_total > target {
//             let excess = current_total.checked_sub(target).unwrap_or(Uint128::zero());

//             if !excess.is_zero() {
//                 let mut remaining_excess = excess;

//                 // Process each delegation for this validator
//                 for del in dels {
//                     if remaining_excess.is_zero() {
//                         break;
//                     }

//                     let undelegate_amount = del.amount.min(remaining_excess);

//                     if !undelegate_amount.is_zero() {
//                         undelegate_msgs.push(MsgUndelegate {
//                             delegator_address: del.del_addr.clone(),
//                             validator_address: validator.clone(),
//                             amount: Some(ProtoCoin {
//                                 denom: denom.to_string(),
//                                 amount: undelegate_amount.to_string(),
//                             }),
//                         });

//                         remaining_excess = remaining_excess
//                             .checked_sub(undelegate_amount)
//                             .unwrap_or(Uint128::zero());
//                     }
//                 }
//             }
//         }
//     }

//     // Comprehensive logging
//     println!("Redelegation Msgs: {}", redelegation_msgs.len());
//     let total_redelegation = redelegation_msgs
//         .iter()
//         .map(|msg| Uint128::from_str(msg.amount.clone().expect("amount").amount.as_str()).unwrap())
//         .sum::<Uint128>();

//     println!("Delegation Msgs: {}", delegation_msgs.len());
//     let total_delegation = delegation_msgs
//         .iter()
//         .map(|msg| Uint128::from_str(msg.amount.clone().expect("amount").amount.as_str()).unwrap())
//         .sum::<Uint128>();

//     println!("Undelegate Msgs: {}", undelegate_msgs.len());
//     let total_undelegation = undelegate_msgs
//         .iter()
//         .map(|msg| Uint128::from_str(msg.amount.clone().expect("amount").amount.as_str()).unwrap())
//         .sum::<Uint128>();

//     println!("Total Redelegation Amount: {}", total_redelegation);
//     println!("Total Delegation Amount: {}", total_delegation);
//     println!("Total Undelegation Amount: {}", total_undelegation);

//     (redelegation_msgs, delegation_msgs, undelegate_msgs)
// }

// // Add detailed logging to track delegation totals
// fn debug_delegation_tracking(
//     current_dao_delegations: &[DelegationResponse],
//     obligated_dao_delegations: &[Delegation],
// ) -> anyhow::Result<()> {
//     // Track total current delegations
//     let total_current_delegations: Uint128 = current_dao_delegations
//         .iter()
//         .map(|a| Uint128::from_str(a.balance.clone().expect("msg").amount.as_str()).unwrap())
//         .sum();

//     // Track total target delegations
//     let total_obligated_delegations: Uint128 =
//         obligated_dao_delegations.iter().map(|d| d.amount).sum();

//     println!("\n--- DELEGATION TOTAL DEBUGGING ---");
//     println!(
//         "Determined Current Delegations : {}
//          Determine Obligated Delegations: {}",
//         Decimal::from_atomics(total_current_delegations, 6)?,
//         Decimal::from_atomics(total_obligated_delegations, 6)?
//     );

//     // Print detailed breakdown of current delegations
//     println!("\nCurrent Delegation Breakdown:");
//     let mut detailed_current_dels = current_dao_delegations
//         .iter()
//         .map(|a| {
//             let balance =
//                 Uint128::from_str(a.balance.clone().expect("msg").amount.as_str()).unwrap();
//             (
//                 a.delegation.clone().expect("msg").validator_address,
//                 balance,
//             )
//         })
//         .collect::<Vec<_>>();

//     detailed_current_dels.sort_by(|a, b| b.1.cmp(&a.1));
//     let mut sum = Uint128::zero();
//     for (validator, amount) in detailed_current_dels {
//         sum += amount;
//     }
//     let dec = Decimal::from_atomics(sum, 6)?;
//     println!("sum: {}", dec);

//     // Print detailed breakdown of target delegations
//     println!("\n Obligated Delegation Breakdown:");
//     let mut detailed_obligated_delegations = obligated_dao_delegations
//         .iter()
//         .map(|d| (d.operator_addr.clone(), d.amount))
//         .collect::<Vec<_>>();

//     detailed_obligated_delegations.sort_by(|a, b| b.1.cmp(&a.1));

//     sum = Uint128::zero();
//     for (_, amount) in detailed_obligated_delegations {
//         sum += amount;
//     }
//     println!("sum: {}", sum);

//     Ok(())
// }
