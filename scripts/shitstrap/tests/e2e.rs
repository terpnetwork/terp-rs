use anyhow::anyhow;

use cosmwasm_std::{Addr, AnyMsg, CosmosMsg, Timestamp, Uint128, Uint256, WasmMsg, to_json_binary};
use cw_orch::daemon::{Daemon, DaemonBuilder};
use cw_orch::prelude::*;
use cw_shit_denom::{AssetUnchecked, PossibleShit, UncheckedDenom};
use cw_shitstrap::contract::interface::CwShitstrap;
use cw_shitstrap::contract::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, ShitstrapMemoAdr08};

use terp_rs::Message;
use terp_rs::ibc::Any;
use terp_rs::ibc::applications::transfer::v1::MsgTransfer;

// ── Constants ──
/// Compute IBC denom hash: SHA256("transfer/{channel}/{denom}") hex-encoded
pub fn ibc_denom_hash(channel_id: &str, port_id: &str, native_denom: &str) -> String {
    use sha2::{Digest, Sha256};
    let input = format!("{}/{}/{}", port_id, channel_id, native_denom);
    let result = Sha256::digest(input.as_bytes());
    let hex_str: String = result.iter().map(|b| format!("{:02X}", b)).collect();
    format!("ibc/{}", hex_str)
}

const DAB: u128 = 1_000_000_000_000_000u128;
const CUTOFF: u128 = 500_000_000_000u128;
const SHITMOS_FUND: u128 = 1_000_000_000_000u128;
const IBC_TIMEOUT_SECONDS: u64 = 600;

// ── Helpers ──

fn default_ibc_instantiate(admin: String, ibc_denom: &str) -> InstantiateMsg {
    InstantiateMsg {
        owner: Some(admin.clone()),
        accepted: vec![PossibleShit::native_denom(ibc_denom, DAB)],
        cutoff: Uint128::from(CUTOFF),
        shitmos: UncheckedDenom::Native("uterp".into()),
        title: "ibc-strap".into(),
        description: "ibc shitstrap".into(),
        daos: vec![],
    }
}

/// Build an IBC transfer message with callback memo using cosmrs
fn build_ibc_transfer_msg<T: Into<Uint256>>(
    source_port: &str,
    source_channel: &str,
    sender: &str,
    receiver: &str,
    amount: T,
    denom: &str,
    memo: &str,
    timeout_timestamp_seconds: u64,
) -> Result<Any, Box<dyn std::error::Error>> {
    let msg = MsgTransfer {
        source_port: source_port.parse()?,
        source_channel: source_channel.parse()?,
        token: Some(terp_rs::cosmos::base::v1beta1::Coin {
            denom: denom.parse()?,
            amount: amount.into().to_string(),
        }),
        sender: sender.parse()?,
        receiver: receiver.parse()?,
        timeout_height: None,
        timeout_timestamp: Timestamp::from_seconds(timeout_timestamp_seconds).nanos(),
        memo: memo.to_string(),
        encoding: String::default(),
        use_aliasing: false,
    };
    Ok(Any::from_msg(&msg)?)
}

/// Build the ShitstrapMemoAdr08 for IBC callback
fn build_callback_memo<T: Into<Uint256> + Clone>(
    callback_addr: &str,
    ibc_denom: &str,
    deposit_amount: T,
) -> Result<String, anyhow::Error> {
    let memo = ShitstrapMemoAdr08 {
        ibc_callback: callback_addr.to_string(),
        a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: callback_addr.to_string(),
            msg: to_json_binary(&ExecuteMsg::ShitStrap {
                recp: None,
                shit: AssetUnchecked::from_native(ibc_denom, deposit_amount.clone()),
                dao: None,
            })
            .map_err(|e| anyhow!(e))?,
            funds: vec![Coin {
                denom: ibc_denom.to_string(),
                amount: deposit_amount.into(),
            }],
        })],
    };
    Ok(serde_json::to_string(&memo)?)
}

// ── Interchain Integration Tests ──

mod interchain {
    use super::*;
    use cw_orch::daemon::{
        TxSender,
        networks::{LOCAL_JUNO, OSMOSIS_1, TERP_LOCALNET, TERP_MAINNET},
        queriers::Bank,
    };
    use cw_orch_interchain::prelude::*;
    use cw_shitstrap::contract::msg::QueryMsgFns as _;

    // /// Test: IBC callback triggers shitstrap cutoff
    // // #[tokio::test]
    // #[test]
    // async fn test_ibc_callback_shitstrap_flow() -> Result<(), Box<dyn std::error::Error>> {
    //     let mut interchain =
    //         DaemonInterchain::new(vec![TERP_LOCALNET, LOCAL_JUNO], &ChannelCreationValidator)?;

    //     let terp = interchain.get_chain("terp")?;
    //     let osmo = interchain.get_chain("localosmosis")?;

    //     let sender_a = terp.sender_addr();
    //     let sender_b = osmo.sender_addr();

    //     // ── Deploy shitstrap on chain A (native uthiol) ──
    //     let strap_a = CwShitstrap::new(terp.clone());
    //     strap_a.upload()?;
    //     strap_a.instantiate(
    //         &InstantiateMsg {
    //             owner: Some(sender_a.to_string()),
    //             accepted: vec![PossibleShit::native_denom("uthiol", DAB)],
    //             cutoff: Uint128::from(CUTOFF),
    //             shitmos: UncheckedDenom::Native("uterp".into()),
    //             title: "terp-native".into(),
    //             description: "terp native strap".into(),
    //             daos: vec![],
    //         },
    //         None,
    //         &[],
    //     )?;

    //     // ── Deploy shitstrap on chain B (IBC denom of uthiol) ──
    //     let ibc_denom = ibc_denom_hash("channel-0", "transfer", "uthiol");
    //     let strap_b = CwShitstrap::new(osmo.clone());
    //     strap_b.upload()?;
    //     strap_b.instantiate(
    //         &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //         None,
    //         &[],
    //     )?;

    //     let strap_b_addr = strap_b.address()?;

    //     // ── Fund strap B with SHITMOS using Daemon bank_send ──
    //     let wallet_b = osmo.sender();
    //     osmo.rt_handle.block_on(wallet_b.bank_send(
    //         &Addr::unchecked(strap_b_addr.clone()),
    //         &[Coin {
    //             denom: "uterp".to_string(),
    //             amount: Uint256::from(SHITMOS_FUND),
    //         }],
    //     ))?;

    //     // ── Verify funding via Daemon querier ──
    //     let bank: Bank = osmo.querier();
    //     let binding = bank.balance(&strap_b_addr, Some("uterp".into()))?;
    //     let b: &Coin = binding.first().expect("must have balance");
    //     assert!(
    //         b.amount >= Uint256::from(SHITMOS_FUND),
    //         "Shitstrap not funded"
    //     );

    //     // ── Build callback memo ──
    //     let deposit_amount = Uint256::from(CUTOFF + 1); // exceed cutoff
    //     let memo_str = build_callback_memo(&strap_b_addr.as_str(), &ibc_denom, deposit_amount)?;

    //     // ── Send IBC transfer A -> B using Daemon commit_tx ──
    //     let wallet_a = terp.sender();
    //     let timeout = std::time::SystemTime::now()
    //         .duration_since(std::time::UNIX_EPOCH)?
    //         .as_secs()
    //         + IBC_TIMEOUT_SECONDS;

    //     let ibc_msg = build_ibc_transfer_msg(
    //         "transfer",
    //         "channel-0",
    //         &sender_a.to_string(),
    //         &strap_b_addr.as_str(),
    //         deposit_amount,
    //         "uthiol",
    //         &memo_str,
    //         timeout,
    //     )?;

    //     terp.rt_handle
    //         .block_on(wallet_a.commit_tx_any(vec![ibc_msg], None))?;

    //     // ── Wait for relay + callback execution ──
    //     osmo.next_block()?;
    //     tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    //     // ── Assert: strap B is now full of shit ──
    //     let full: bool = strap_b.full_of_shit()?;
    //     assert!(full, "Shitstrap should be full of shit after IBC callback");

    //     // ── Assert: SHITMOS were paid out ──
    //     let has_shit: Uint256 = strap_b.has_shit()?.into();
    //     assert!(has_shit >= Uint256::from(CUTOFF));

    //     Ok(())
    // }

    //     /// Test: Callback rejects actions spending more than transferred
    //     #[tokio::test]
    //     async fn test_ibc_callback_overspend_rejected() -> Result<(), Box<dyn std::error::Error>> {
    //         let mut interchain =
    //             DaemonInterchain::new(vec![TERP_MAINNET, OSMOSIS_1], &ChannelCreationValidator)?;

    //         let terp = interchain.get_chain("terp")?;
    //         let osmo = interchain.get_chain("localosmosis")?;

    //         let rt = tokio::runtime::Handle::current();
    //         let state = DaemonState::new_empty();

    //         let daemon_a = DaemonBuilder::new(TERP_MAINNET)
    //             .deployment_id("overspend-a")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let daemon_b = DaemonBuilder::new(OSMOSIS_1)
    //             .deployment_id("overspend-b")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let sender_a = daemon_a.sender_addr();
    //         let sender_b = daemon_b.sender_addr();

    //         // ── Deploy on chain B ──
    //         let ibc_denom = ibc_denom_hash("channel-0", "transfer", "uthiol");
    //         let strap_b = CwShitstrap::new(daemon_b.clone());
    //         strap_b.upload()?;
    //         strap_b.instantiate(
    //             &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //             None,
    //             &[],
    //         )?;

    //         let strap_b_addr = strap_b.address()?;

    //         // ── Fund strap B ──
    //         let wallet_b = daemon_b.sender();
    //         rt.block_on(wallet_b.bank_send(
    //             &Addr::unchecked(strap_b_addr.clone()),
    //             vec![Coin {
    //                 denom: "uterp".to_string(),
    //                 amount: Uint256::from(SHITMOS_FUND),
    //             }],
    //         ))?;

    //         // ── Memo requests 2x what's being transferred ──
    //         let transfer_amount = Uint256::from(100_000_000u128);
    //         let overspend_amount = Uint256::from(200_000_000u128);

    //         let memo = ShitstrapMemoAdr08 {
    //             ibc_callback: strap_b_addr.clone(),
    //             a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
    //                 contract_addr: strap_b_addr.clone(),
    //                 msg: to_json_binary(&ExecuteMsg::ShitStrap {
    //                     recp: None,
    //                     shit: AssetUnchecked::from_native(
    //                         &ibc_denom,
    //                         Uint256::from(overspend_amount.u128()),
    //                     ),
    //                     dao: None,
    //                 })?,
    //                 funds: vec![Coin {
    //                     denom: ibc_denom.clone(),
    //                     amount: Uint256::from(overspend_amount.u128()),
    //                 }],
    //             })],
    //         };
    //         let memo_str = serde_json::to_string(&memo)?;

    //         // ── Send IBC transfer with insufficient funds for action ──
    //         let wallet_a = daemon_a.sender();
    //         let timeout = std::time::SystemTime::now()
    //             .duration_since(std::time::UNIX_EPOCH)?
    //             .as_secs()
    //             + IBC_TIMEOUT_SECONDS;

    //         let ibc_msg = build_ibc_transfer_msg(
    //             "transfer",
    //             "channel-0",
    //             &sender_a.to_string(),
    //             &strap_b_addr,
    //             transfer_amount.u128(),
    //             "uthiol",
    //             &memo_str,
    //             timeout,
    //         )?;

    //         rt.block_on(wallet_a.commit_tx_any(vec![ibc_msg], None))?;

    //         daemon_b.next_block()?;
    //         tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    //         // ── Strap should NOT be full of shit ──
    //         let full: bool = strap_b.full_of_shit()?;
    //         assert!(!full, "Overspend action should be rejected in callback");

    //         Ok(())
    //     }

    //     /// Test: Callback rejects actions targeting wrong contract
    //     #[tokio::test]
    //     async fn test_ibc_callback_wrong_contract_rejected() -> Result<(), Box<dyn std::error::Error>> {
    //         let mut interchain =
    //             DaemonInterchain::new(vec![TERP_MAINNET, OSMOSIS_1], &ChannelCreationValidator)?;

    //         let terp = interchain.get_chain("terp")?;
    //         let osmo = interchain.get_chain("localosmosis")?;

    //         let rt = tokio::runtime::Handle::current();
    //         let state = DaemonState::new_empty();

    //         let daemon_a = DaemonBuilder::new(TERP_MAINNET)
    //             .deployment_id("wrong-addr-a")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let daemon_b = DaemonBuilder::new(OSMOSIS_1)
    //             .deployment_id("wrong-addr-b")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let sender_a = daemon_a.sender_addr();
    //         let sender_b = daemon_b.sender_addr();

    //         // ── Deploy on chain B ──
    //         let ibc_denom = ibc_denom_hash("channel-0", "transfer", "uthiol");
    //         let strap_b = CwShitstrap::new(daemon_b.clone());
    //         strap_b.upload()?;
    //         strap_b.instantiate(
    //             &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //             None,
    //             &[],
    //         )?;

    //         let strap_b_addr = strap_b.address()?;
    //         let wrong_addr = "terp1wrongcontractaddr";

    //         // ── Memo targets wrong contract ──
    //         let memo = ShitstrapMemoAdr08 {
    //             ibc_callback: strap_b_addr.clone(),
    //             a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
    //                 contract_addr: wrong_addr.to_string(), // NOT the callback contract
    //                 msg: to_json_binary(&ExecuteMsg::ShitStrap {
    //                     recp: None,
    //                     shit: AssetUnchecked::from_native(&ibc_denom, Uint128::new(100)),
    //                     dao: None,
    //                 })?,
    //                 funds: vec![Coin {
    //                     denom: ibc_denom.clone(),
    //                     amount: Uint128::new(100),
    //                 }],
    //             })],
    //         };
    //         let memo_str = serde_json::to_string(&memo)?;

    //         // ── Send IBC transfer ──
    //         let wallet_a = daemon_a.sender();
    //         let timeout = std::time::SystemTime::now()
    //             .duration_since(std::time::UNIX_EPOCH)?
    //             .as_secs()
    //             + IBC_TIMEOUT_SECONDS;

    //         let ibc_msg = build_ibc_transfer_msg(
    //             "transfer",
    //             "channel-0",
    //             &sender_a.to_string(),
    //             &strap_b_addr,
    //             100u128,
    //             "uthiol",
    //             &memo_str,
    //             timeout,
    //         )?;

    //         rt.block_on(wallet_a.commit_tx_any(vec![ibc_msg], None))?;

    //         daemon_b.next_block()?;
    //         tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    //         // ── Action should be rejected ──
    //         let full: bool = strap_b.full_of_shit()?;
    //         assert!(!full, "Wrong contract address should reject action");

    //         Ok(())
    //     }

    //     /// Test: Callback rejects actions with unaccepted denom
    //     #[tokio::test]
    //     async fn test_ibc_callback_unaccepted_denom_rejected() -> Result<(), Box<dyn std::error::Error>>
    //     {
    //         let mut interchain =
    //             DaemonInterchain::new(vec![TERP_MAINNET, OSMOSIS_1], &ChannelCreationValidator)?;

    //         let terp = interchain.get_chain("terp")?;
    //         let osmo = interchain.get_chain("localosmosis")?;

    //         let rt = tokio::runtime::Handle::current();
    //         let state = DaemonState::new_empty();

    //         let daemon_a = DaemonBuilder::new(TERP_MAINNET)
    //             .deployment_id("bad-denom-a")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let daemon_b = DaemonBuilder::new(OSMOSIS_1)
    //             .deployment_id("bad-denom-b")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let sender_a = daemon_a.sender_addr();
    //         let sender_b = daemon_b.sender_addr();

    //         // ── Deploy on chain B ──
    //         let ibc_denom = ibc_denom_hash("channel-0", "transfer", "uthiol");
    //         let strap_b = CwShitstrap::new(daemon_b.clone());
    //         strap_b.upload()?;
    //         strap_b.instantiate(
    //             &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //             None,
    //             &[],
    //         )?;

    //         let strap_b_addr = strap_b.address()?;

    //         // ── Action uses a denom the strap doesn't accept ──
    //         let memo = ShitstrapMemoAdr08 {
    //             ibc_callback: strap_b_addr.clone(),
    //             a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
    //                 contract_addr: strap_b_addr.clone(),
    //                 msg: to_json_binary(&ExecuteMsg::ShitStrap {
    //                     recp: None,
    //                     shit: AssetUnchecked::from_native("uosmo", Uint128::new(100)),
    //                     dao: None,
    //                 })?,
    //                 funds: vec![Coin {
    //                     denom: "uosmo".to_string(),
    //                     amount: Uint128::new(100),
    //                 }],
    //             })],
    //         };
    //         let memo_str = serde_json::to_string(&memo)?;

    //         // ── Send IBC transfer ──
    //         let wallet_a = daemon_a.sender();
    //         let timeout = std::time::SystemTime::now()
    //             .duration_since(std::time::UNIX_EPOCH)?
    //             .as_secs()
    //             + IBC_TIMEOUT_SECONDS;

    //         let ibc_msg = build_ibc_transfer_msg(
    //             "transfer",
    //             "channel-0",
    //             &sender_a.to_string(),
    //             &strap_b_addr,
    //             100u128,
    //             "uthiol",
    //             &memo_str,
    //             timeout,
    //         )?;

    //         rt.block_on(wallet_a.commit_tx_any(vec![ibc_msg], None))?;

    //         daemon_b.next_block()?;
    //         tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    //         // ── Action should be rejected ──
    //         let full: bool = strap_b.full_of_shit()?;
    //         assert!(!full, "Unaccepted denom should reject action");

    //         Ok(())
    //     }

    //     /// Test: Multiple actions in single callback, partial success
    //     #[tokio::test]
    //     async fn test_ibc_callback_multi_action_partial() -> Result<(), Box<dyn std::error::Error>> {
    //         let mut interchain =
    //             DaemonInterchain::new(vec![TERP_MAINNET, OSMOSIS_1], &ChannelCreationValidator)?;

    //         let terp = interchain.get_chain("terp")?;
    //         let osmo = interchain.get_chain("localosmosis")?;

    //         let rt = tokio::runtime::Handle::current();
    //         let state = DaemonState::new_empty();

    //         let daemon_a = DaemonBuilder::new(TERP_MAINNET)
    //             .deployment_id("multi-a")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let daemon_b = DaemonBuilder::new(OSMOSIS_1)
    //             .deployment_id("multi-b")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let sender_a = daemon_a.sender_addr();
    //         let sender_b = daemon_b.sender_addr();

    //         // ── Deploy on chain B ──
    //         let ibc_denom = ibc_denom_hash("channel-0", "transfer", "uthiol");
    //         let strap_b = CwShitstrap::new(daemon_b.clone());
    //         strap_b.upload()?;
    //         strap_b.instantiate(
    //             &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //             None,
    //             &[],
    //         )?;

    //         let strap_b_addr = strap_b.address()?;

    //         // ── Fund strap B ──
    //         let wallet_b = daemon_b.sender();
    //         rt.block_on(wallet_b.bank_send(
    //             &Addr::unchecked(strap_b_addr.clone()),
    //             vec![Coin {
    //                 denom: "uterp".to_string(),
    //                 amount: Uint256::from(SHITMOS_FUND),
    //             }],
    //         ))?;

    //         // ── Memo with valid action + invalid action (wrong denom) ──
    //         let valid_amount = Uint256::from(100_000_000u128);
    //         let memo = ShitstrapMemoAdr08 {
    //             ibc_callback: strap_b_addr.clone(),
    //             a: vec![
    //                 // Valid: uses accepted IBC denom
    //                 CosmosMsg::Wasm(WasmMsg::Execute {
    //                     contract_addr: strap_b_addr.clone(),
    //                     msg: to_json_binary(&ExecuteMsg::ShitStrap {
    //                         recp: None,
    //                         shit: AssetUnchecked::from_native(
    //                             &ibc_denom,
    //                             Uint256::from(valid_amount.u128()),
    //                         ),
    //                         dao: None,
    //                     })?,
    //                     funds: vec![Coin {
    //                         denom: ibc_denom.clone(),
    //                         amount: Uint256::from(valid_amount.u128()),
    //                     }],
    //                 }),
    //                 // Invalid: uses unaccepted denom
    //                 CosmosMsg::Wasm(WasmMsg::Execute {
    //                     contract_addr: strap_b_addr.clone(),
    //                     msg: to_json_binary(&ExecuteMsg::ShitStrap {
    //                         recp: None,
    //                         shit: AssetUnchecked::from_native("uosmo", Uint128::new(50)),
    //                         dao: None,
    //                     })?,
    //                     funds: vec![Coin {
    //                         denom: "uosmo".to_string(),
    //                         amount: Uint128::new(50),
    //                     }],
    //                 }),
    //             ],
    //         };
    //         let memo_str = serde_json::to_string(&memo)?;

    //         // ── Send IBC transfer with enough for both actions ──
    //         let wallet_a = daemon_a.sender();
    //         let timeout = std::time::SystemTime::now()
    //             .duration_since(std::time::UNIX_EPOCH)?
    //             .as_secs()
    //             + IBC_TIMEOUT_SECONDS;

    //         let ibc_msg = build_ibc_transfer_msg(
    //             "transfer",
    //             "channel-0",
    //             &sender_a.to_string(),
    //             &strap_b_addr,
    //             valid_amount.u128() + 50,
    //             "uthiol",
    //             &memo_str,
    //             timeout,
    //         )?;

    //         rt.block_on(wallet_a.commit_tx_any(vec![ibc_msg], None))?;

    //         daemon_b.next_block()?;
    //         tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    //         // ── Entire callback should fail (one action invalid) ──
    //         let full: bool = strap_b.full_of_shit()?;
    //         assert!(
    //             !full,
    //             "Invalid action in memo should reject entire callback"
    //         );

    //         Ok(())
    //     }

    //     /// Test: Simulate IBC transfer before sending
    //     #[tokio::test]
    //     async fn test_ibc_simulate_before_send() -> Result<(), Box<dyn std::error::Error>> {
    //         let mut interchain =
    //             DaemonInterchain::new(vec![TERP_MAINNET, OSMOSIS_1], &ChannelCreationValidator)?;

    //         let terp = interchain.get_chain("terp")?;
    //         let osmo = interchain.get_chain("localosmosis")?;

    //         let rt = tokio::runtime::Handle::current();
    //         let state = DaemonState::new_empty();

    //         let daemon_a = DaemonBuilder::new(TERP_MAINNET)
    //             .deployment_id("simulate-a")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let daemon_b = DaemonBuilder::new(OSMOSIS_1)
    //             .deployment_id("simulate-b")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let sender_a = daemon_a.sender_addr();
    //         let sender_b = daemon_b.sender_addr();

    //         // ── Deploy on chain B ──
    //         let ibc_denom = ibc_denom_hash("channel-0", "transfer", "uthiol");
    //         let strap_b = CwShitstrap::new(daemon_b.clone());
    //         strap_b.upload()?;
    //         strap_b.instantiate(
    //             &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //             None,
    //             &[],
    //         )?;

    //         let strap_b_addr = strap_b.address()?;

    //         // ── Build and simulate IBC transfer ──
    //         let timeout = std::time::SystemTime::now()
    //             .duration_since(std::time::UNIX_EPOCH)?
    //             .as_secs()
    //             + IBC_TIMEOUT_SECONDS;

    //         let ibc_msg = build_ibc_transfer_msg(
    //             "transfer",
    //             "channel-0",
    //             &sender_a.to_string(),
    //             &strap_b_addr,
    //             100u128,
    //             "uthiol",
    //             r#"{"ibc_callback":"callback_addr"}"#,
    //             timeout,
    //         )?;

    //         let wallet_a = daemon_a.sender();
    //         let (gas_needed, fee_needed) =
    //             rt.block_on(wallet_a.simulate(vec![ibc_msg.clone()], None))?;

    //         println!(
    //             "IBC transfer will require: {} gas, {} fee",
    //             gas_needed, fee_needed
    //         );

    //         assert!(gas_needed > 0, "Simulation should return non-zero gas");

    //         // ── Now actually send it ──
    //         rt.block_on(wallet_a.commit_tx_any(vec![ibc_msg], None))?;

    //         Ok(())
    //     }

    //     /// Test: Verify state file persistence across deployments
    //     #[tokio::test]
    //     async fn test_state_persistence_across_deployments() -> Result<(), Box<dyn std::error::Error>> {
    //         let mut interchain =
    //             DaemonInterchain::new(vec![TERP_MAINNET, OSMOSIS_1], &ChannelCreationValidator)?;

    //         let osmo = interchain.get_chain("localosmosis")?;

    //         let rt = tokio::runtime::Handle::current();
    //         let state = DaemonState::new_empty();

    //         // ── First deployment ──
    //         let daemon_b_v1 = DaemonBuilder::new(OSMOSIS_1)
    //             .deployment_id("v1")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let sender_b = daemon_b_v1.sender_addr();
    //         let ibc_denom = ibc_denom_hash("channel-0", "transfer", "uthiol");

    //         let strap_v1 = CwShitstrap::new(daemon_b_v1.clone());
    //         strap_v1.upload()?;
    //         strap_v1.instantiate(
    //             &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //             None,
    //             &[],
    //         )?;

    //         let v1_addr = strap_v1.address()?;
    //         let v1_code_id = strap_v1.code_id()?;

    //         // ── Second deployment (same code_id, different instance) ──
    //         let daemon_b_v2 = DaemonBuilder::new(OSMOSIS_1)
    //             .deployment_id("v2")
    //             .handle(&rt)
    //             .state(state.clone())
    //             .build()?;

    //         let strap_v2 = CwShitstrap::new(daemon_b_v2.clone());
    //         // Should reuse code_id from state
    //         strap_v2.upload()?;
    //         strap_v2.instantiate(
    //             &default_ibc_instantiate(sender_b.to_string(), &ibc_denom),
    //             None,
    //             &[],
    //         )?;

    //         let v2_addr = strap_v2.address()?;
    //         let v2_code_id = strap_v2.code_id()?;

    //         // ── Verify: same code_id, different addresses ──
    //         assert_eq!(
    //             v1_code_id, v2_code_id,
    //             "Code IDs should match from shared state"
    //         );
    //         assert_ne!(v1_addr, v2_addr, "Deployment IDs isolate instances");

    //         Ok(())
    //     }
    // }
}
