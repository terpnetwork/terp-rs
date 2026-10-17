//! Deploy `cw-private-dex` to Terp testnet and seed swap demo assets.
//!
//! Private settle pools use **virtual reserves** + 32-byte asset ids (not bank
//! liquidity). We still mint **tokenfactory** denoms so wallets / UI have real
//! coins to hold while demos map denoms → asset ids.
//!
//! ```text
//! export MNEMONIC='…'   # funded on 120u-1
//! # optional: CHAIN=local
//! # optional: MOCK_VERIFY=0  (default 1 = lab mock_verify)
//! cargo run -p zk-test-press --bin deploy_private_dex_testnet --features interface
//! ```
//!
//! Requires `cw_private_dex.wasm` under workspace artifacts (e.g. headstash/artifacts).

use std::env;

use cosmwasm_std::Binary;
use cw_orch::anyhow::{self, Context};
use cw_orch::daemon::networks::{TERP_TESTNET, TERP_TESTNET_LOCAL};
use cw_orch::prelude::*;
use osmosis_std::types::{
    cosmos::base::v1beta1::Coin,
    osmosis::tokenfactory::v1beta1::{MsgCreateDenom, MsgMint},
};
use prost::Message;
use prost_types::Any;
use sha2::{Digest, Sha256};
use zk_test_press::harness::swap_statement_cw::{
    lab_allowed_root, LAB_GAMMA, LAB_GAMMA_DEN, LAB_R_IN, LAB_R_OUT,
};
use zk_test_press::suites::private_dex::PrivateDexSuite;

/// Demo subdenoms (factory/{creator}/{sub}).
const DEMO_SUBDENOMS: &[&str] = &["pdexalpha", "pdexbeta", "pdexgamma"];

/// Mint amount per demo denom (base units).
const MINT_AMOUNT: &str = "1000000000000"; // 1e12

fn asset_id_from_denom(denom: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(denom.as_bytes()));
    out
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let mnemonic = env::var("MNEMONIC").context(
        "MNEMONIC env required (funded testnet wallet, never commit secrets)",
    )?;

    let chain_info = match env::var("CHAIN").as_deref() {
        Ok("local") | Ok("LOCAL") => TERP_TESTNET_LOCAL.clone(),
        _ => TERP_TESTNET.clone(),
    };

    let mock_verify = env::var("MOCK_VERIFY")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(true);

    eprintln!(
        "deploy_private_dex_testnet: chain_id={} mock_verify={mock_verify} grpc={:?}",
        chain_info.chain_id, chain_info.grpc_urls
    );

    let daemon = Daemon::builder(chain_info)
        .mnemonic(&mnemonic)
        .build()?;

    let sender = daemon.sender_addr().to_string();

    // ── 1) Tokenfactory: create + mint a few swap-demo denoms ───────────────
    let mut denoms = Vec::new();
    for sub in DEMO_SUBDENOMS {
        eprintln!("tokenfactory create_denom {sub}…");
        daemon.commit_any(
            vec![Any {
                type_url: MsgCreateDenom::TYPE_URL.to_string(),
                value: MsgCreateDenom {
                    sender: sender.clone(),
                    subdenom: sub.to_string(),
                }
                .encode_to_vec(),
            }],
            None,
        )?;
        let denom = format!("factory/{sender}/{sub}");
        eprintln!("tokenfactory mint {denom}…");
        daemon.commit_any(
            vec![Any {
                type_url: MsgMint::TYPE_URL.to_string(),
                value: MsgMint {
                    sender: sender.clone(),
                    amount: Some(Coin {
                        amount: MINT_AMOUNT.to_string(),
                        denom: denom.clone(),
                    }),
                    mint_to_address: sender.clone(),
                }
                .encode_to_vec(),
            }],
            None,
        )?;
        denoms.push(denom);
    }

    // ── 2) Deploy private-dex (lab mock_verify by default) ──────────────────
    let mut suite = PrivateDexSuite::new(daemon.clone());
    eprintln!("upload + instantiate cw-private-dex…");
    suite.upload_and_instantiate(
        mock_verify,
        Some(Binary::from(lab_allowed_root().to_vec())),
    )?;

    let dex_addr = suite.dex.addr_str()?;
    let code_id = suite.dex.code_id().ok();

    // ── 3) Virtual pools: pair first three denoms as asset ids ──────────────
    // Assets are 32-byte ids (sha256 denom); reserves are virtual (not bank-funded).
    let mut pools = Vec::new();
    let pairs = [(0usize, 1usize), (1, 2), (0, 2)];
    for (i, j) in pairs {
        let da = &denoms[i];
        let db = &denoms[j];
        let asset_a = Binary::from(asset_id_from_denom(da).to_vec());
        let asset_b = Binary::from(asset_id_from_denom(db).to_vec());
        eprintln!("CreatePool {da} ↔ {db}…");
        suite.create_pool(
            asset_a.clone(),
            asset_b.clone(),
            LAB_R_IN,
            LAB_R_OUT,
            LAB_GAMMA,
            LAB_GAMMA_DEN,
        )?;
        pools.push(serde_json::json!({
            "asset_a_denom": da,
            "asset_b_denom": db,
            "asset_a_id_hex": hex::encode(asset_id_from_denom(da)),
            "asset_b_id_hex": hex::encode(asset_id_from_denom(db)),
            "r_a": LAB_R_IN,
            "r_b": LAB_R_OUT,
            "gamma": LAB_GAMMA,
            "gamma_den": LAB_GAMMA_DEN,
        }));
    }

    let out = serde_json::json!({
        "demo": "private-dex",
        "mock_verify": mock_verify,
        "contract_code_id": code_id,
        "contract_address": dex_addr,
        "admin": sender,
        "tokenfactory_denoms": denoms.iter().map(|d| {
            serde_json::json!({
                "denom": d,
                "minted": MINT_AMOUNT,
                "asset_id_hex": hex::encode(asset_id_from_denom(d)),
            })
        }).collect::<Vec<_>>(),
        "pools_created": pools,
        "notes": [
            "Private-dex pools use virtual reserves + 32-byte asset ids (not bank LP).",
            "Tokenfactory denoms are wallet-visible demo coins mapped via sha256(denom) → asset_id.",
            "Settle path: mock_verify lab proofs until a swap circuit + zkid is registered.",
            "Not BTC/CashApp corridor — that remains a separate stack.",
        ],
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
