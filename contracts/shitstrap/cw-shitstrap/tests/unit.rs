// ── Memo Construction Tests ──
use cosmwasm_std::{coin, coins, Addr, Coin, CosmosMsg, Uint128, Uint256, WasmMsg};
use cw_shit_denom::AssetUnchecked;
use cw_shitstrap::contract::msg::{ExecuteMsg, ShitstrapMemoAdr08};

#[cfg(test)]
mod memo {
    use super::*;
    use cosmwasm_std::to_json_binary;

    #[test]
    fn test_shitstrap_memo_serialization() {
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: "terp1contract".to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "terp1contract".to_string(),
                msg: to_json_binary(&ExecuteMsg::ShitStrap {
                    recp: None,
                    shit: AssetUnchecked::from_native("uatom", 100u128),
                    dao: None,
                })
                .unwrap(),
                funds: vec![coin(100, "uatom")],
            })],
        };

        let json = serde_json::to_string(&memo).unwrap();
        assert!(json.contains("ibc_callback"));
        assert!(json.contains("terp1contract"));

        let parsed: ShitstrapMemoAdr08 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.ibc_callback, "terp1contract");
        assert_eq!(parsed.a.len(), 1);
    }

    #[test]
    fn test_memo_with_multiple_actions() {
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: "terp1contract".to_string(),
            a: vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "terp1contract".to_string(),
                    msg: to_json_binary(&ExecuteMsg::ShitStrap {
                        recp: None,
                        shit: AssetUnchecked::from_native("uatom", 100u128),
                        dao: None,
                    })
                    .unwrap(),
                    funds: vec![coin(100, "uatom")],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "terp1contract".to_string(),
                    msg: to_json_binary(&ExecuteMsg::ShitStrap {
                        recp: Some("terp1recipient".to_string()),
                        shit: AssetUnchecked::from_native("uatom", 200u128),
                        dao: None,
                    })
                    .unwrap(),
                    funds: vec![coin(200, "uatom")],
                }),
            ],
        };

        assert_eq!(memo.a.len(), 2);
    }

    #[test]
    fn test_memo_funds_exceed_transfer_validation() {
        // This tests the logic: if transfer sends 100uatom,
        // actions requesting 150uatom should fail validation
        let transfer_funds = vec![coin(100, "uatom")];
        let action_funds = vec![coin(150, "uatom")];

        let mut available: std::collections::HashMap<String, Uint256> =
            std::collections::HashMap::new();
        for c in &transfer_funds {
            *available.entry(c.denom.clone()).or_insert(Uint256::zero()) += c.amount;
        }

        for c in &action_funds {
            let remaining = available.get(&c.denom).copied().unwrap_or(Uint256::zero());
            let result = remaining.checked_sub(c.amount);
            assert!(result.is_err(), "Should fail: action exceeds transfer");
        }
    }
}

#[cfg(test)]
mod ibc_callback_tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{
        to_json_binary, BankMsg, Binary, Coin, Env, IbcAcknowledgement, IbcDestinationCallbackMsg,
        IbcEndpoint, IbcPacket, IbcTimeoutBlock, IbcTransferCallback, OwnedDeps, StdAck, Uint256,
        WasmMsg,
    };
    use cw_shit_denom::CheckedDenom;
    use cw_shitstrap::contract::state::{Config, CONFIG, CURRENT_SHITSTRAP_VALUE, SHITSTRAP_STATE};
    use cw_shitstrap::contract::{ibc_destination_callback, ContractError};
    use terp_rs::ibc::applications::transfer::v1::FungibleTokenPacketData;

    /// Helper: build a minimal IbcDestinationCallbackMsg with sensible defaults.
    /// Override fields by mutating the returned struct.
    fn make_callback_msg(
        receiver: &str,
        memo: &str,
        funds: Vec<Coin>,
        ack_success: bool,
        dest_port: &str,
    ) -> IbcDestinationCallbackMsg {
        let packet_data = FungibleTokenPacketData {
            denom: "uatom".to_string(),
            amount: "1000000".to_string(),
            sender: "cosmos1sender".to_string(),
            receiver: receiver.to_string(),
            memo: memo.to_string(),
        };

        println!("=== DEBUG: FungibleTokenPacketData ===");
        println!("  denom:   {}", packet_data.denom);
        println!("  amount:  {}", packet_data.amount);
        println!("  sender:  {}", packet_data.sender);
        println!("  receiver:{}", packet_data.receiver);
        println!("  memo:    {:?}", packet_data.memo);
        println!("  memo bytes: {:?}", packet_data.memo.as_bytes());

        let serialized_data = to_json_binary(&packet_data).unwrap();
        println!(
            "  serialized packet.data (hex): {}",
            hex::encode(&serialized_data)
        );
        println!(
            "  serialized packet.data (json): {}",
            String::from_utf8_lossy(&serialized_data)
        );

        let ack = if ack_success {
            StdAck::success(Binary::from(b"{}"))
        } else {
            StdAck::error("ack failure")
        };
        let ack_binary = to_json_binary(&ack).unwrap();

        println!("=== DEBUG: StdAck ===");
        println!("  is_success: {}", ack.is_success());
        println!("  ack binary (hex): {}", hex::encode(&ack_binary));

        let packet = IbcPacket::new(
            serialized_data,
            IbcEndpoint {
                port_id: "transfer".to_string(),
                channel_id: "channel-0".to_string(),
            },
            IbcEndpoint {
                port_id: dest_port.to_string(),
                channel_id: "channel-1".to_string(),
            },
            1,
            IbcTimeoutBlock {
                revision: 1,
                height: 999_999,
            }
            .into(),
        );
        IbcDestinationCallbackMsg {
            packet,
            ack: IbcAcknowledgement::new(ack_binary),
            transfer: Some(IbcTransferCallback {
                sender: packet_data.sender,
                funds,
                receiver: Addr::unchecked(&packet_data.receiver),
            }),
        }
    }

    /// Helper: set up deps with SHITSTRAP_STATE entries for accepted denoms.
    fn setup_deps_with_accepted_shit(
        denoms: &[&str],
    ) -> (OwnedDeps<MockStorage, MockApi, MockQuerier>, Env) {
        let mut deps = mock_dependencies();

        // Save a minimal CONFIG so the contract address is known
        CONFIG
            .save(
                &mut deps.storage,
                &Config {
                    owner: deps.api.addr_make("owner"),
                    cutoff: Uint256::from(1_000_000u128),
                    shitmos_addr: CheckedDenom::Native("ushit".to_string()),
                    full_of_shit: false,
                    title: "test".to_string(),
                    description: "test".to_string(),
                },
            )
            .unwrap();

        CURRENT_SHITSTRAP_VALUE
            .save(&mut deps.storage, &Uint256::zero())
            .unwrap();

        for denom in denoms {
            SHITSTRAP_STATE
                .save(&mut deps.storage, denom.to_string(), &Uint256::zero())
                .unwrap();
        }

        (deps, mock_env()) // Return the OwnedDeps
    }

    // ─── HAPPY PATH TESTS ───────────────────────────────────────────

    #[test]
    fn ibc_callback_happy_path_empty_memo() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);

        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_happy_path_empty_memo ===");
        println!("  contract address: {}", me);

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let msg = make_callback_msg(me.as_str(), "", funds, true, "transfer");

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());

        let resp = res.unwrap();
        assert_eq!(
            resp.attributes
                .iter()
                .find(|a| a.key == "action")
                .map(|a| a.value.clone()),
            Some("ibc_destination_callback".to_string())
        );
        assert_eq!(
            resp.attributes
                .iter()
                .find(|a| a.key == "receiver")
                .map(|a| a.value.clone()),
            Some(me.to_string())
        );
        assert_eq!(
            resp.attributes
                .iter()
                .find(|a| a.key == "num_transfers")
                .map(|a| a.value.clone()),
            Some("1".to_string())
        );
    }

    #[test]
    fn ibc_callback_happy_path_with_memo_actions() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);

        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_happy_path_with_memo_actions ===");
        println!("  contract address: {}", me);

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(5000000u128),
        }];

        // Build a valid ShitstrapMemoAdr08
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: me.to_string(),
                msg: to_json_binary(&"some_msg").unwrap(),
                funds: vec![Coin {
                    denom: "uatom".to_string(),
                    amount: Uint256::from(1000000u128),
                }],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  memo JSON: {}", String::from_utf8_lossy(&memo_json));
        println!("  memo JSON (hex): {}", hex::encode(&memo_json));

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());

        let resp = res.unwrap();
        assert_eq!(resp.messages.len(), 1); // one submessage from memo
    }

    #[test]
    fn ibc_callback_happy_path_multiple_actions_within_funds() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom", "uosmo"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_happy_path_multiple_actions_within_funds ===");

        let funds = vec![
            Coin {
                denom: "uatom".to_string(),
                amount: Uint256::from(5000000u128),
            },
            Coin {
                denom: "uosmo".to_string(),
                amount: Uint256::from(3000000u128),
            },
        ];

        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: me.to_string(),
                    msg: to_json_binary(&"action1").unwrap(),
                    funds: vec![Coin {
                        denom: "uatom".to_string(),
                        amount: Uint256::from(2000000u128),
                    }],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: me.to_string(),
                    msg: to_json_binary(&"action2").unwrap(),
                    funds: vec![Coin {
                        denom: "uosmo".to_string(),
                        amount: Uint256::from(1000000u128),
                    }],
                }),
            ],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  memo JSON: {}", String::from_utf8_lossy(&memo_json));

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());
    }

    // ─── PORT ID VALIDATION ──────────────────────────────────────────

    #[test]
    fn ibc_callback_wrong_port_id() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_wrong_port_id ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let msg = make_callback_msg(me.as_str(), "", funds, true, "wasm");

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        let err = res.unwrap_err();
        println!("  error: {}", err);
        assert!(err
            .to_string()
            .contains("only want to handle transfer packets"));
    }

    // ─── ACK VALIDATION ─────────────────────────────────────────────

    #[test]
    fn ibc_callback_ack_not_success() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_ack_not_success ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let msg = make_callback_msg(me.as_str(), "", funds, false, "transfer");

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), ContractError::AckNotSuccess {});
    }

    // ─── RECEIVER VALIDATION ────────────────────────────────────────

    #[test]
    fn ibc_callback_receiver_mismatch() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_receiver_mismatch ===");
        println!("  contract address: {}", me);

        let wrong_receiver = "cosmos1wrongreceiver123456789";
        println!("  wrong receiver: {}", wrong_receiver);

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let msg = make_callback_msg(wrong_receiver, "", funds, true, "transfer");

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        let err = res.unwrap_err();
        println!("  error: {}", err);
        assert!(err.to_string().contains("invalid checksum"));
    }

    // ─── MEMO PARSING ───────────────────────────────────────────────

    #[test]
    fn ibc_callback_memo_parse_error() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_memo_parse_error ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        // Invalid JSON as memo
        let bad_memo = "{ not valid json }}}";
        println!("  bad memo: {:?}", bad_memo);

        let msg = make_callback_msg(me.as_str(), bad_memo, funds, true, "transfer");

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        let err = res.unwrap_err();
        println!("  error: {}", err);
        assert!(matches!(err, ContractError::MemoParseError { .. }));
    }

    #[test]
    fn ibc_callback_memo_callback_addr_mismatch() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_memo_callback_addr_mismatch ===");
        println!("  contract address: {}", me);

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let wrong_callback = "cosmos1wrongcallback";
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: wrong_callback.to_string(),
            a: vec![],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  memo ibc_callback: {}", wrong_callback);
        println!("  memo JSON: {}", String::from_utf8_lossy(&memo_json));

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), ContractError::CallbackAddrMismatch {});
    }

    // ─── DEBUG: RAW MEMO STRUCTURE EXPLORATION ──────────────────────

    #[test]
    fn debug_print_memo_structures() {
        println!("\n=== DEBUG: Exploring ShitstrapMemoAdr08 structures ===");

        let contract_addr = "cosmos1contract123456789";
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: contract_addr.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: contract_addr.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![Coin {
                    denom: "uatom".to_string(),
                    amount: Uint256::from(1000000u128),
                }],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  ShitstrapMemoAdr08 as JSON:");
        println!("    {}", String::from_utf8_lossy(&memo_json));
        println!("  ShitstrapMemoAdr08 as hex:");
        println!("    {}", hex::encode(&memo_json));

        // Also print what FungibleTokenPacketData looks like
        let packet_data = FungibleTokenPacketData {
            denom: "uatom".to_string(),
            amount: "1000000".to_string(),
            sender: "cosmos1sender".to_string(),
            receiver: contract_addr.to_string(),
            memo: String::from_utf8_lossy(&memo_json).to_string(),
        };

        let packet_json = to_json_binary(&packet_data).unwrap();
        println!("\n  FungibleTokenPacketData with embedded memo as JSON:");
        println!("    {}", String::from_utf8_lossy(&packet_json));

        // Print what the memo field looks like when it's a raw string
        let packet_data_raw = FungibleTokenPacketData {
            denom: "uatom".to_string(),
            amount: "1000000".to_string(),
            sender: "cosmos1sender".to_string(),
            receiver: contract_addr.to_string(),
            memo: "just a plain string memo".to_string(),
        };

        let packet_json_raw = to_json_binary(&packet_data_raw).unwrap();
        println!("\n  FungibleTokenPacketData with plain string memo:");
        println!("    {}", String::from_utf8_lossy(&packet_json_raw));

        // Print StdAck structures
        let success_ack = StdAck::success(Binary::from(b"{}"));
        let success_ack_json = to_json_binary(&success_ack).unwrap();
        println!("\n  StdAck::success JSON:");
        println!("    {}", String::from_utf8_lossy(&success_ack_json));

        let error_ack = StdAck::error("test error");
        let error_ack_json = to_json_binary(&error_ack).unwrap();
        println!("\n  StdAck::error JSON:");
        println!("    {}", String::from_utf8_lossy(&error_ack_json));
    }

    #[test]
    fn debug_print_ibc_callback_msg_structure() {
        println!("\n=== DEBUG: Full IbcDestinationCallbackMsg structure ===");

        let contract_addr = "cosmos1contract123456789";
        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let msg = make_callback_msg(contract_addr, "", funds, true, "transfer");

        println!("  msg.packet.src.port_id:    {}", msg.packet.src.port_id);
        println!("  msg.packet.src.channel_id: {}", msg.packet.src.channel_id);
        println!("  msg.packet.dest.port_id:   {}", msg.packet.dest.port_id);
        println!(
            "  msg.packet.dest.channel_id:{}",
            msg.packet.dest.channel_id
        );
        println!("  msg.packet.sequence:       {}", msg.packet.sequence);
        println!(
            "  msg.packet.data (hex):     {}",
            hex::encode(&msg.packet.data)
        );
        println!(
            "  msg.packet.data (json):    {}",
            String::from_utf8_lossy(&msg.packet.data)
        );
        println!(
            "  msg.ack.data (hex):        {}",
            hex::encode(&msg.ack.data)
        );
        println!(
            "  msg.ack.data (json):       {}",
            String::from_utf8_lossy(&msg.ack.data)
        );

        if let Some(transfer) = &msg.transfer {
            println!("  msg.transfer.funds:");
            for coin in &transfer.funds {
                println!("    denom: {}, amount: {}", coin.denom, coin.amount);
            }
        }
    }

    // ─── ACTION VALIDATION ──────────────────────────────────────────

    #[test]
    fn ibc_callback_non_wasm_cosmos_msg_rejected() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_non_wasm_cosmos_msg_rejected ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(5000000u128),
        }];

        // BankMsg is not Wasm, should be rejected
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Bank(BankMsg::Send {
                to_address: "someone".to_string(),
                amount: vec![Coin {
                    denom: "uatom".to_string(),
                    amount: Uint256::from(1000000u128),
                }],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!(
            "  memo with BankMsg: {}",
            String::from_utf8_lossy(&memo_json)
        );

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        let err = res.unwrap_err();
        println!("  error: {}", err);
        assert!(matches!(err, ContractError::IbcCallbackError { .. }));
        assert!(err.to_string().contains("only CosmosMsg::Wasm accepted"));
    }

    #[test]
    fn ibc_callback_non_execute_wasm_msg_rejected() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_non_execute_wasm_msg_rejected ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(5000000u128),
        }];

        // WasmMsg::Instantiate is not Execute, should be rejected
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
                admin: None,
                code_id: 1,
                msg: Binary::from(b"{}"),
                funds: vec![],
                label: "test".to_string(),
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!(
            "  memo with WasmMsg::Instantiate: {}",
            String::from_utf8_lossy(&memo_json)
        );

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        let err = res.unwrap_err();
        println!("  error: {}", err);
        assert!(matches!(err, ContractError::IbcCallbackError { .. }));
        assert!(err.to_string().contains("only WasmMsg::Execute accepted"));
    }

    #[test]
    fn ibc_callback_msg_to_wrong_contract_rejected() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_msg_to_wrong_contract_rejected ===");
        println!("  contract address: {}", me);

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(5000000u128),
        }];

        let wrong_contract = "cosmos1wrongcontract";
        println!("  wrong contract in action: {}", wrong_contract);

        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: wrong_contract.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![Coin {
                    denom: "uatom".to_string(),
                    amount: Uint256::from(1000000u128),
                }],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        let err = res.unwrap_err();
        println!("  error: {}", err);
        assert!(matches!(err, ContractError::IbcCallbackError { .. }));
        assert!(err
            .to_string()
            .contains(&format!("only msgs to {} accepted", me)));
    }

    // ─── DENOM VALIDATION ───────────────────────────────────────────

    #[test]
    fn ibc_callback_unaccepted_denom_rejected() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]); // only uatom accepted
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_unaccepted_denom_rejected ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(5000000u128),
        }];

        // Try to use "ujuno" in action funds, which is not in SHITSTRAP_STATE
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: me.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![Coin {
                    denom: "ujuno".to_string(),
                    amount: Uint256::from(1000000u128),
                }],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!(
            "  memo with unaccepted denom 'ujuno': {}",
            String::from_utf8_lossy(&memo_json)
        );

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        let err = res.unwrap_err();
        println!("  error: {}", err);
        assert!(matches!(err, ContractError::IbcCallbackError { .. }));
        assert!(err.to_string().contains("sent shit not accepted: ujuno"));
    }

    // ─── FUND ALLOCATION / UNDERFLOW ────────────────────────────────

    #[test]
    fn ibc_callback_overspend_single_denom_rejected() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_overspend_single_denom_rejected ===");

        // Transfer brings 1M uatom
        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        // But memo action tries to spend 5M uatom
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: me.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![Coin {
                    denom: "uatom".to_string(),
                    amount: Uint256::from(5000000u128),
                }],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  available: 1000000uatom, action wants: 5000000uatom");

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        // Should be an overflow/subtraction error
        println!("  error: {}", res.unwrap_err());
    }

    #[test]
    fn ibc_callback_overspend_across_multiple_actions_rejected() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_overspend_across_multiple_actions_rejected ===");

        // Transfer brings 3M uatom
        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(3000000u128),
        }];

        // Two actions that together exceed 3M
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: me.to_string(),
                    msg: Binary::from(b"{}"),
                    funds: vec![Coin {
                        denom: "uatom".to_string(),
                        amount: Uint256::from(2000000u128),
                    }],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: me.to_string(),
                    msg: Binary::from(b"{}"),
                    funds: vec![Coin {
                        denom: "uatom".to_string(),
                        amount: Uint256::from(2000000u128), // 2M + 2M = 4M > 3M
                    }],
                }),
            ],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  available: 3000000uatom, actions want: 2000000 + 2000000 = 4000000");

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
        println!("  error: {}", res.unwrap_err());
    }

    #[test]
    fn ibc_callback_exact_spend_succeeds() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_exact_spend_succeeds ===");

        // Transfer brings exactly 3M uatom
        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(3000000u128),
        }];

        // Two actions that exactly total 3M
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: me.to_string(),
                    msg: Binary::from(b"{}"),
                    funds: vec![Coin {
                        denom: "uatom".to_string(),
                        amount: Uint256::from(1000000u128),
                    }],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: me.to_string(),
                    msg: Binary::from(b"{}"),
                    funds: vec![Coin {
                        denom: "uatom".to_string(),
                        amount: Uint256::from(2000000u128),
                    }],
                }),
            ],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  available: 3000000uatom, actions want: 1000000 + 2000000 = 3000000 (exact)");

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());
    }

    #[test]
    fn ibc_callback_multi_denom_partial_spend_succeeds() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom", "uosmo"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_multi_denom_partial_spend_succeeds ===");

        let funds = vec![
            Coin {
                denom: "uatom".to_string(),
                amount: Uint256::from(5000000u128),
            },
            Coin {
                denom: "uosmo".to_string(),
                amount: Uint256::from(3000000u128),
            },
        ];

        // Only spend some of each
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: me.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![
                    Coin {
                        denom: "uatom".to_string(),
                        amount: Uint256::from(2000000u128),
                    },
                    Coin {
                        denom: "uosmo".to_string(),
                        amount: Uint256::from(1000000u128),
                    },
                ],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());
    }

    #[test]
    fn ibc_callback_overspend_one_denom_while_other_ok_rejected() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom", "uosmo"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_overspend_one_denom_while_other_ok_rejected ===");

        let funds = vec![
            Coin {
                denom: "uatom".to_string(),
                amount: Uint256::from(5000000u128),
            },
            Coin {
                denom: "uosmo".to_string(),
                amount: Uint256::from(1000000u128), // only 1M uosmo
            },
        ];

        // uatom is fine, but uosmo overspends
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: me.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![
                    Coin {
                        denom: "uatom".to_string(),
                        amount: Uint256::from(2000000u128), // OK
                    },
                    Coin {
                        denom: "uosmo".to_string(),
                        amount: Uint256::from(5000000u128), // OVERSPEND
                    },
                ],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  available: 5000000uatom + 1000000uosmo");
        println!("  action wants: 2000000uatom + 5000000uosmo (uosmo overspends)");

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_err());
    }

    // ─── EDGE CASES ─────────────────────────────────────────────────

    #[test]
    fn ibc_callback_no_transfer_data_panics() {
        // The code does .expect("msg") on msg.transfer, so None will panic.
        // This test documents that behavior.
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_no_transfer_data (expect panic) ===");

        let packet_data = FungibleTokenPacketData {
            denom: "uatom".to_string(),
            amount: "1000000".to_string(),
            sender: "sender".to_string(),
            receiver: me.to_string(),
            memo: "".to_string(),
        };

        let ack = StdAck::success(Binary::from(b"{}"));
        let msg = IbcDestinationCallbackMsg {
            packet: IbcPacket::new(
                to_json_binary(&packet_data).unwrap(),
                IbcEndpoint {
                    port_id: "transfer".to_string(),
                    channel_id: "channel-0".to_string(),
                },
                IbcEndpoint {
                    port_id: "transfer".to_string(),
                    channel_id: "channel-1".to_string(),
                },
                1,
                IbcTimeoutBlock {
                    revision: 1,
                    height: 999_999,
                }
                .into(),
            ),
            ack: IbcAcknowledgement::new(to_json_binary(&ack).unwrap()),
            transfer: None, // This will cause .expect("msg") to panic
        };

        let r = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  did panic: {}", r.is_err());
        assert!(r.is_err(), "Expected panic when transfer is None");
    }

    #[test]
    fn ibc_callback_empty_actions_list_succeeds() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_empty_actions_list_succeeds ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![], // empty actions
        };

        let memo_json = to_json_binary(&memo).unwrap();
        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());

        let resp = res.unwrap();
        assert_eq!(resp.messages.len(), 0); // no submessages
    }

    #[test]
    fn ibc_callback_action_with_no_funds_succeeds() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_action_with_no_funds_succeeds ===");

        let funds = vec![Coin {
            denom: "uatom".to_string(),
            amount: Uint256::from(1000000u128),
        }];

        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: me.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![], // no funds in action
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());
    }

    #[test]
    fn ibc_callback_multiple_coins_same_denom_in_transfer() {
        let (mut deps, env) = setup_deps_with_accepted_shit(&["uatom"]);
        let me = env.contract.address.clone();
        println!("\n=== TEST: ibc_callback_multiple_coins_same_denom_in_transfer ===");

        // Two coins of same denom in transfer (should be summed in available)
        let funds = vec![
            Coin {
                denom: "uatom".to_string(),
                amount: Uint256::from(2000000u128),
            },
            Coin {
                denom: "uatom".to_string(),
                amount: Uint256::from(3000000u128),
            },
        ];

        // Action wants 4M uatom, and we have 2M + 3M = 5M total
        let memo = ShitstrapMemoAdr08 {
            ibc_callback: me.to_string(),
            a: vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: me.to_string(),
                msg: Binary::from(b"{}"),
                funds: vec![Coin {
                    denom: "uatom".to_string(),
                    amount: Uint256::from(4000000u128),
                }],
            })],
        };

        let memo_json = to_json_binary(&memo).unwrap();
        println!("  available: 2000000 + 3000000 = 5000000uatom, action wants: 4000000");

        let msg = make_callback_msg(
            me.as_str(),
            &String::from_utf8_lossy(&memo_json),
            funds,
            true,
            "transfer",
        );

        let res = ibc_destination_callback(deps.as_mut(), env, msg);
        println!("  result: {:?}", res);
        assert!(res.is_ok());
    }

    // ─── UINT256 vs UINT128 CONVERSION EDGE CASES ───────────────────

    #[test]
    fn debug_uint256_coin_amounts() {
        println!("\n=== DEBUG: Uint256 Coin amounts ===");
        println!("  Note: Coin.amount is now Uint256 in cosmwasm-std");
        println!("  SHITSTRAP_STATE and shit_rate use Uint128/Uint256");

        let small = Uint256::from(1000000u128);
        let large = Uint256::from(1_000_000_000_000_000_000u128);
        let max_u128 = Uint256::from(u128::MAX);

        println!("  small:  {}", small);
        println!("  large:  {}", large);
        println!("  u128::MAX as Uint256: {}", max_u128);

        // Demonstrate that Uint256 can hold values larger than u128
        let bigger_than_u128 = max_u128 + Uint256::one();
        println!("  u128::MAX + 1: {}", bigger_than_u128);
        println!("  This is a Uint256 value that cannot fit in u128");
    }
}
