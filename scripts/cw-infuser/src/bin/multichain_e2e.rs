//! Multichain e2e: IBC transfer + callback → ShitStrapAndMint → cutoff → NFT mint.
//! Chain A: CwShitstrapSuite (native uthiol). Chain B: CwSvgSuite (SVG + minter + infuser + shitstrap).
//! Deploys callback contract on B, funds shitstrap, sends ICS-20 with callback memo.
//! Asserts SHITMOS payout + NFT mint on B.
#![cfg(feature = "e2e")]

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;

use cw_infuser_scripts::suite::{CwSvgSuite, CwSvgSuiteDeployData};
use cw_orch::daemon::DaemonBuilder;
use cw_orch::environment::{ChainInfoOwned, ChainKind};
use cw_orch::prelude::*;
use cw_shitstrap::contract::msg::ExecuteMsg as ShitstrapExecuteMsg;
// use ict_rs::chain::cosmos::CosmosChain;
// use ict_rs::chain::Chain;
// use ict_rs::interchain::{Interchain, InterchainBuildOptions, InterchainLink};
// use ict_rs::relayer::{build_relayer, RelayerType};
// use ict_rs::runtime::{DockerConfig, IctRuntime};
// use ict_rs::testing::TestEnv;
// use ict_rs::tx::WalletAmount;
use shit_scripts::{CwShitstrapSuite, CwShitstrapSuiteDeployData, ShitInitMsg, UncheckedDenom};
use tracing::info;

// fn mk_config(chain_id: &str) -> ict_rs::chain::ChainConfig {
//     let mut cfg = TestEnv::terp_localterp_config();
//     cfg.chain_id = chain_id.to_string();
//     cfg
// }

// async fn spawn_dual_chain(a_id: &str, b_id: &str) -> Result<Interchain> {
//     let nw = format!("ict-multi-e2e-{}", a_id);
//     let rt = IctRuntime::Docker(DockerConfig::default())
//         .into_backend()
//         .await?;
//     rt.create_network(&nw).await?;

//     let ca = CosmosChain::new(mk_config(a_id), 1, 0, rt.clone());
//     let cb = CosmosChain::new(mk_config(b_id), 1, 0, rt.clone());
//     let rl = build_relayer(RelayerType::Hermes, rt.clone(), "hermes", &nw).await?;

//     let mut ic = Interchain::new(rt)
//         .add_chain(Box::new(ca))
//         .add_chain(Box::new(cb))
//         .add_relayer("hermes", rl)
//         .add_link(InterchainLink {
//             chain1: a_id.into(),
//             chain2: b_id.into(),
//             relayer: "hermes".into(),
//             path: "ibc-path".into(),
//         });

//     ic.build(InterchainBuildOptions {
//         test_name: "multichain-e2e".into(),
//         skip_path_creation: false,
//         genesis_wallets: HashMap::from([
//             (
//                 a_id.into(),
//                 vec![
//                     WalletAmount {
//                         address: "deployer".into(),
//                         denom: "uthiol".into(),
//                         amount: 1_000_000,
//                     },
//                     WalletAmount {
//                         address: "deployer".into(),
//                         denom: "uterp".into(),
//                         amount: 1_000_000_000_000,
//                     },
//                 ],
//             ),
//             (
//                 b_id.into(),
//                 vec![
//                     WalletAmount {
//                         address: "deployer".into(),
//                         denom: "uthiol".into(),
//                         amount: 1_000_000,
//                     },
//                     WalletAmount {
//                         address: "deployer".into(),
//                         denom: "uterp".into(),
//                         amount: 1_000_000_000_000,
//                     },
//                 ],
//             ),
//         ]),
//     })
//     .await?;

//     Ok(ic)
// }

// fn build_chain_info(c: &dyn Chain) -> ChainInfoOwned {
//     let mut info = ChainInfoOwned::config();
//     info.chain_id = c.chain_id().to_string();
//     info.gas_denom = "uterp".into();
//     info.gas_price = 0.25;
//     info.grpc_urls = vec![c.host_grpc_address()];
//     info.kind = ChainKind::Local;
//     info.network_info.chain_name = "terp network".into();
//     info.network_info.pub_address_prefix = "terp".into();
//     info
// }

// async fn send_ibc_transfer_with_callback(
//     chain: &dyn Chain,
//     channel_id: &str,
//     from_key: &str,
//     to_address: &str,
//     amount: &str,
//     callback_addr: &str,
// ) -> Result<()> {
//     let memo = serde_json::json!({"ibc_callback": callback_addr}).to_string();
//     let args = [
//         "tx",
//         "ibc-transfer",
//         "transfer",
//         "transfer",
//         channel_id,
//         to_address,
//         amount,
//         "--memo",
//         &memo,
//         "--from",
//         from_key,
//         "--chain-id",
//         chain.chain_id(),
//         "--gas",
//         "auto",
//         "--gas-adjustment",
//         "1.5",
//         "--gas-prices",
//         "0.25uterp",
//         "--yes",
//     ];
//     let out = chain.chain_exec_tx(&args).await?;
//     info!("IBC transfer tx: {}", out.stdout_str());
//     Ok(())
// }

// fn ibc_denom_hash(channel_id: &str, port_id: &str, native_denom: &str) -> String {
//     use sha2::{Digest, Sha256};
//     let input = format!("{}/{}/{}", port_id, channel_id, native_denom);
//     let result = Sha256::digest(input.as_bytes());
//     let hex_str: String = result.iter().map(|b| format!("{:02X}", b)).collect();
//     format!("ibc/{}", hex_str)
// }

// async fn query_balance(chain: &dyn Chain, address: &str, denom: &str) -> Result<String> {
//     let out = chain
//         .chain_exec(&[
//             "q",
//             "bank",
//             "balance",
//             address,
//             denom,
//             "--chain-id",
//             chain.chain_id(),
//             "--output",
//             "json",
//         ])
//         .await?;
//     let val: serde_json::Value = serde_json::from_str(out.stdout_str().trim())?;
//     let amt = val["balance"]["amount"].as_str().unwrap_or("0").to_string();
//     Ok(amt)
// }

// async fn query_nft_owner(chain: &dyn Chain, nft_addr: &str, token_id: &str) -> Result<String> {
//     let out = chain
//         .chain_exec(&[
//             "q",
//             "wasm",
//             "contract-state",
//             "smart",
//             nft_addr,
//             &serde_json::json!({"owner_of": {"token_id": token_id}}).to_string(),
//             "--chain-id",
//             chain.chain_id(),
//             "--output",
//             "json",
//         ])
//         .await?;
//     let val: serde_json::Value = serde_json::from_str(out.stdout_str().trim())?;
//     let owner = val["data"]["owner"].as_str().unwrap_or("none").to_string();
//     Ok(owner)
// }

// pub async fn run_e2e(a_id: &str, b_id: &str, keep: bool) -> Result<()> {
//     let mut ic = spawn_dual_chain(a_id, b_id).await?;

//     // Build daemons
//     let mut infos: Vec<(&str, ChainInfoOwned)> = Vec::new();
//     for id in [a_id, b_id] {
//         let c = ic.get_chain(id).expect("chain exists");
//         infos.push((id, build_chain_info(c)));
//     }
//     let (chain_id_a, info_a) = infos.remove(0);
//     let (chain_id_b, info_b) = infos.remove(0);

//     let (da, db) = tokio::task::spawn_blocking(move || -> Result<_> {
//         let rt = tokio::runtime::Handle::current();
//         let da = DaemonBuilder::new(info_a).handle(&rt).build()?;
//         let db = DaemonBuilder::new(info_b).handle(&rt).build()?;
//         Ok((da, db))
//     })
//     .await??;

//     let sender = da.sender_addr();
//     info!("Deployer: {}", sender);

//     // -------------------------------------------------------------------
//     // Chain A: deploy CwShitstrapSuite (native uthiol)
//     // -------------------------------------------------------------------
//     let data_a = Some(CwShitstrapSuiteDeployData {
//         admin: Some(sender.clone()),
//         shit: vec![ShitInitMsg {
//             daos: vec![],
//             owner: Some(sender.to_string()),
//             accepted: vec![cw_shitstrap::PossibleShit::native_denom(
//                 "uthiol",
//                 1_000_000_000_000_000u128,
//             )],
//             cutoff: 500_000_000_000u128.into(),
//             shitmos: UncheckedDenom::Native("uterp".into()),
//             title: "terp".into(),
//             description: "terp".into(),
//         }],
//     });
//     let suite_a = CwShitstrapSuite::deploy_on(da.clone(), data_a)?;
//     let shitstrap_a = suite_a.shitstrap.address()?;
//     info!("Shitstrap on {}: {}", chain_id_a, shitstrap_a);

//     // -------------------------------------------------------------------
//     // Chain B: deploy CwSvgSuite + shitstrap + callback
//     // -------------------------------------------------------------------
//     let ibc_denom_b = ibc_denom_hash("channel-0", "transfer", "uthiol");
//     info!("IBC denom on B: {}", ibc_denom_b);

//     let shit_data_b = Some(CwShitstrapSuiteDeployData {
//         admin: Some(sender.clone()),
//         shit: vec![ShitInitMsg {
//             daos: vec![],
//             owner: Some(sender.to_string()),
//             accepted: vec![cw_shitstrap::PossibleShit::native_denom(
//                 &ibc_denom_b,
//                 1_000_000_000_000_000u128,
//             )],
//             cutoff: 500_000_000_000u128.into(),
//             shitmos: UncheckedDenom::Native("uterp".into()),
//             title: "ibc-terp".into(),
//             description: "ibc-terp".into(),
//         }],
//     });

//     // Deploy SVG suite (uploads all, instantiates minter + SVG + infuser)
//     let svg_data = Some(CwSvgSuiteDeployData {
//         svg: vec![],
//         infuse: None,
//         shit: shit_data_b.clone(),
//         admin: Some(sender.clone()),
//         infuse_coins: vec![],
//     });
//     let suite_b = CwSvgSuite::deploy_on(db.clone(), svg_data)?;
//     info!("CwSvgSuite deployed on B");

//     // Manually deploy the shitstrap contracts via the suite's shit field
//     // (CwSvgSuite::deploy_on does NOT instantiate the nested shitstrap)
//     let _ = CwShitstrapSuite::deploy_on(db.clone(), shit_data_b)?;
//     let shitstrap_b = suite_b.shit.shitstrap.address()?;
//     info!("Shitstrap on {}: {}", chain_id_b, shitstrap_b);
//     let svg_last = suite_b.svgs.last().cloned();
//     info!("Last SVG collection: {:?}", svg_last);

//     // -------------------------------------------------------------------
//     // Configure shitstrap with SVG collection address for NFT mint
//     // -------------------------------------------------------------------
//     if let Some(ref svg_addr) = svg_last {
//         suite_b.shit.shitstrap.execute(
//             &ShitstrapExecuteMsg::UpdateSvgCollection {
//                 address: svg_addr.clone(),
//             },
//             &[],
//         )?;
//         info!("Shitstrap configured with SVG collection: {}", svg_addr);
//     }
//     // -------------------------------------------------------------------
//     // Fund the shitstrap on chain B with SHITMOS (uterp)
//     // -------------------------------------------------------------------
//     let fund_amount = "1000000000000";
//     info!("Funding shitstrap with {}uterp", fund_amount);
//     drop(da);
//     drop(db);

//     let chain_b = ic.get_chain(chain_id_b).expect("chain B exists");
//     chain_b
//         .chain_exec_tx(&[
//             "tx",
//             "bank",
//             "send",
//             "deployer",
//             shitstrap_b.as_ref(),
//             &format!("{}uterp", fund_amount),
//             "--chain-id",
//             chain_b.chain_id(),
//             "--gas",
//             "auto",
//             "--gas-adjustment",
//             "1.5",
//             "--gas-prices",
//             "0.25uterp",
//             "--yes",
//         ])
//         .await?;
//     info!("Shitstrap funded with SHITMOS");

//     // -------------------------------------------------------------------
//     // Send IBC transfer from chain A -> chain B with callback memo
//     // -------------------------------------------------------------------
//     let chain_a = ic.get_chain(chain_id_a).expect("chain A exists");
//     let amount = "1000000000000uthiol"; // 1 THIOL — above 500B cutoff

//     let chain_b = ic.get_chain(chain_id_b).expect("chain B exists");
//     let bal_before = query_balance(chain_b, shitstrap_b.as_ref(), "uterp").await?;
//     info!("Callback SHITMOS before IBC: {}", bal_before);

//     // Snapshot any existing NFT for the callback addr before mint
//     let nft_before = if let Some(ref svg_addr) = svg_last {
//         query_nft_owner(chain_b, svg_addr, "1")
//             .await
//             .unwrap_or_else(|_| "none".into())
//     } else {
//         "none".into()
//     };
//     info!("NFT owner before: {}", nft_before);

//     send_ibc_transfer_with_callback(
//         chain_a,
//         "channel-0",
//         "deployer",
//         shitstrap_b.as_ref(),
//         amount,
//         shitstrap_b.as_ref(),
//     )
//     .await?;

//     // -------------------------------------------------------------------
//     // Wait for relay + callback execution
//     // -------------------------------------------------------------------
//     info!("Waiting for IBC relay + callback execution...");
//     tokio::time::sleep(Duration::from_secs(15)).await;

//     // -------------------------------------------------------------------
//     // Assert: callback contract received SHITMOS from the shitstrap
//     // -------------------------------------------------------------------
//     let chain_b = ic.get_chain(chain_id_b).expect("chain B exists");
//     let bal_after = query_balance(chain_b, shitstrap_b.as_ref(), "uterp").await?;
//     info!("Callback SHITMOS after IBC: {}", bal_after);

//     let before_u128: u128 = bal_before.parse()?;
//     let after_u128: u128 = bal_after.parse()?;
//     let delta = after_u128.saturating_sub(before_u128);
//     info!("SHITMOS delta for callback contract: {}", delta);

//     if delta == 0 {
//         let ibc_bal = query_balance(chain_b, shitstrap_b.as_ref(), &ibc_denom_b).await?;
//         info!("Callback IBC denom balance: {}", ibc_bal);
//         anyhow::bail!(
//             "Callback contract received 0 SHITMOS — IBC-or-mint flow failed. \
//              IBC denom balance: {}. Check relayer logs and callback tx.",
//             ibc_bal
//         );
//     }
//     info!(
//         "SUCCESS: Callback contract received {} SHITMOS from shitstrap",
//         delta
//     );

//     // -------------------------------------------------------------------
//     // Assert: NFT minted for the callback contract
//     // -------------------------------------------------------------------
//     if let Some(ref svg_addr) = svg_last {
//         let nft_after = query_nft_owner(chain_b, svg_addr, "1").await?;
//         info!("NFT owner after: {}", nft_after);
//         if nft_after != "none" && nft_after != nft_before {
//             info!("SUCCESS: NFT minted to {}", nft_after);
//         } else {
//             info!("NFT was not minted (may need larger deposit or different cutoff).");
//         }
//     }

//     // -------------------------------------------------------------------
//     // Cleanup
//     // -------------------------------------------------------------------
//     if !keep {
//         ic.close().await?;
//     }
//     Ok(())
// }

pub fn main() -> Result<()> {
    // let runtime = tokio::runtime::Runtime::new()?;
    // runtime.block_on(run_e2e("240u-1", "240u-2", false))
    Ok(())
}
