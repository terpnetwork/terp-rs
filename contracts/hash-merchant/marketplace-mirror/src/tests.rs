use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi};
use cosmwasm_std::{Addr, Binary, Coin, SubMsg, Uint128};

use crate::contract::{execute, instantiate, query, sudo};
use crate::error::ContractError;
use crate::msg::*;
use crate::state::{OrderStatus, DEFAULT_MIN_BUFFER_BLOCKS};

type TestDeps = cosmwasm_std::OwnedDeps<
    cosmwasm_std::MemoryStorage,
    cosmwasm_std::testing::MockApi,
    cosmwasm_std::testing::MockQuerier,
>;

const DENOM: &str = "uterp";

fn admin_addr() -> Addr {
    MockApi::default().addr_make("admin")
}
fn seller_addr() -> Addr {
    MockApi::default().addr_make("seller")
}
fn buyer_addr() -> Addr {
    MockApi::default().addr_make("buyer")
}
fn buyer2_addr() -> Addr {
    MockApi::default().addr_make("buyer2")
}

fn setup(deps: &mut TestDeps) {
    setup_with_buffer(deps, DEFAULT_MIN_BUFFER_BLOCKS);
}

fn setup_with_buffer(deps: &mut TestDeps, min_buffer_blocks: u64) {
    let admin = admin_addr();
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin, &[]),
        InstantiateMsg {
            admin: admin.to_string(),
            min_buffer_blocks,
            accepted_denom: DENOM.into(),
            protocol_fee_bps: None,
            auto_execute_ingress: false,
        },
    )
    .unwrap();
}

fn list_sku(deps: &mut TestDeps, sku: &str, stock: u64, price: u128) {
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&seller_addr(), &[]),
        ExecuteMsg::ListProduct {
            sku: sku.into(),
            stock,
            unit_price: Uint128::new(price),
            content_cid: Some("aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899".into()),
            d_tag: Some(format!("product-{}", sku)),
        },
    )
    .unwrap();
}

fn funds(amount: u128) -> Vec<Coin> {
    vec![Coin::new(amount, DENOM)]
}

// ── SANITY ──────────────────────────────────────────────────────────────

#[test]
fn instantiate_defaults_buffer_7() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let bin = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let cfg: ConfigResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(cfg.min_buffer_blocks, 7);
    assert_eq!(cfg.accepted_denom, DENOM);
    assert_eq!(cfg.next_order_id, 1);
    assert!(!cfg.auto_execute_ingress, "ingress auto-execute defaults off");
}

#[test]
fn purchase_reserves_inventory() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 10, 100);

    let res = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &funds(300)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 3,
        },
    )
    .unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "status" && a.value == "reserved"));

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ReservedQty {
            sku: "tee".into(),
        },
    )
    .unwrap();
    let r: ReservedQtyResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(r.reserved, 3);

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Product {
            sku: "tee".into(),
        },
    )
    .unwrap();
    let p: ProductResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(p.product.stock, 10);
    assert_eq!(p.reserved, 3);
    assert_eq!(p.available, 7);

    let bin = query(deps.as_ref(), mock_env(), QueryMsg::Order { id: 1 }).unwrap();
    let o: OrderResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(o.order.status, OrderStatus::Reserved);
    assert_eq!(o.order.qty, 3);
    assert_eq!(o.order.amount, Uint128::new(300));
    // mock_env height is 12345; buffer 7 → settle_after 12352
    assert_eq!(o.order.settle_after, o.order.reserve_height + 7);
}

#[test]
fn settle_before_buffer_fails() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 50);

    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&buyer_addr(), &funds(50)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 1,
        },
    )
    .unwrap();

    // Same height — buffer not elapsed
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&buyer_addr(), &[]),
        ExecuteMsg::Settle { order_id: 1 },
    )
    .unwrap_err();
    match err {
        ContractError::BufferNotElapsed { .. } => {}
        other => panic!("expected BufferNotElapsed, got {other:?}"),
    }
}

#[test]
fn settle_after_buffer_succeeds() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 50);

    let mut env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&buyer_addr(), &funds(100)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 2,
        },
    )
    .unwrap();

    env.block.height += 7; // exactly settle_after
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&buyer_addr(), &[]),
        ExecuteMsg::Settle { order_id: 1 },
    )
    .unwrap();

    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "status" && a.value == "settled"));
    assert_eq!(res.messages.len(), 1);
    match &res.messages[0] {
        SubMsg {
            msg: cosmwasm_std::CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
                to_address,
                amount,
            }),
            ..
        } => {
            assert_eq!(to_address, seller_addr().as_str());
            assert_eq!(amount[0].amount, Uint128::new(100));
        }
        other => panic!("unexpected msg: {other:?}"),
    }

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Product {
            sku: "tee".into(),
        },
    )
    .unwrap();
    let p: ProductResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(p.product.stock, 3);
    assert_eq!(p.reserved, 0);
    assert_eq!(p.available, 3);
}

#[test]
fn update_stock_cannot_free_reserved_units() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 10, 10);

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &funds(40)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 4,
        },
    )
    .unwrap();

    // Try to set stock below reserved (4)
    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&seller_addr(), &[]),
        ExecuteMsg::UpdateStock {
            sku: "tee".into(),
            stock: 3,
        },
    )
    .unwrap_err();
    match err {
        ContractError::StockBelowReserved {
            stock: 3,
            reserved: 4,
            ..
        } => {}
        other => panic!("expected StockBelowReserved, got {other:?}"),
    }

    // Exactly reserved is OK
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&seller_addr(), &[]),
        ExecuteMsg::UpdateStock {
            sku: "tee".into(),
            stock: 4,
        },
    )
    .unwrap();
}

#[test]
fn double_purchase_oversell_fails() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 10);

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &funds(40)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 4,
        },
    )
    .unwrap();

    // Only 1 available; requesting 2 must fail
    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer2_addr(), &funds(20)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 2,
        },
    )
    .unwrap_err();
    match err {
        ContractError::InsufficientStock {
            need: 2,
            available: 1,
            ..
        } => {}
        other => panic!("expected InsufficientStock, got {other:?}"),
    }

    // Taking the last unit succeeds
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer2_addr(), &funds(10)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 1,
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ReservedQty {
            sku: "tee".into(),
        },
    )
    .unwrap();
    let r: ReservedQtyResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(r.reserved, 5);
}

#[test]
fn sudo_stores_root() {
    let mut deps = mock_dependencies();
    setup(&mut deps);

    // 32 zero bytes as base64
    let root_b64 = Binary::from(vec![0u8; 32]).to_base64();
    let res = sudo(
        deps.as_mut(),
        mock_env(),
        SudoMsg::HashMerchant {
            chain_uid: "nostr-marketplace".into(),
            algo: "marketplace-inv-v1".into(),
            root: root_b64,
            height: 42,
            attestation_count: 3,
            block_time: 1_700_000_000,
        },
    )
    .unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "action" && a.value == "store_root"));

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::GetRoot {
            chain_uid: "nostr-marketplace".into(),
            algo: "marketplace-inv-v1".into(),
        },
    )
    .unwrap();
    let root: RootResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(root.height, 42);
    assert_eq!(root.attestation_count, 3);
    assert_eq!(root.root, "00".repeat(32));
    // Sudo must not invent stock
    assert!(query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Product {
            sku: "ghost".into(),
        },
    )
    .is_err());
}

#[test]
fn cancel_refunds_and_releases_reservation() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 25);

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &funds(50)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 2,
        },
    )
    .unwrap();

    let res = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &[]),
        ExecuteMsg::Cancel { order_id: 1 },
    )
    .unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "status" && a.value == "cancelled"));
    assert_eq!(res.messages.len(), 1);

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ReservedQty {
            sku: "tee".into(),
        },
    )
    .unwrap();
    let r: ReservedQtyResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(r.reserved, 0);

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Product {
            sku: "tee".into(),
        },
    )
    .unwrap();
    let p: ProductResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(p.available, 5);
}

#[test]
fn seller_cannot_cancel_reserved_order() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 25);

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &funds(25)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 1,
        },
    )
    .unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&seller_addr(), &[]),
        ExecuteMsg::Cancel { order_id: 1 },
    )
    .unwrap_err();
    match err {
        ContractError::Unauthorized {} => {}
        other => panic!("expected Unauthorized for seller cancel, got {other:?}"),
    }

    // Reservation still held
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ReservedQty {
            sku: "tee".into(),
        },
    )
    .unwrap();
    let r: ReservedQtyResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(r.reserved, 1);
}

#[test]
fn admin_can_cancel_reserved_order() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 25);

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &funds(25)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 1,
        },
    )
    .unwrap();

    let res = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin_addr(), &[]),
        ExecuteMsg::Cancel { order_id: 1 },
    )
    .unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "status" && a.value == "cancelled"));
}

#[test]
fn cancel_after_settle_fails() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 50);

    let mut env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&buyer_addr(), &funds(50)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 1,
        },
    )
    .unwrap();

    env.block.height += 7;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&buyer_addr(), &[]),
        ExecuteMsg::Settle { order_id: 1 },
    )
    .unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&buyer_addr(), &[]),
        ExecuteMsg::Cancel { order_id: 1 },
    )
    .unwrap_err();
    match err {
        ContractError::BadOrderStatus { .. } => {}
        other => panic!("expected BadOrderStatus after settle, got {other:?}"),
    }
}

#[test]
fn invalid_buffer_rejected() {
    let mut deps = mock_dependencies();
    let admin = admin_addr();
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin, &[]),
        InstantiateMsg {
            admin: admin.to_string(),
            min_buffer_blocks: 3,
            accepted_denom: DENOM.into(),
            protocol_fee_bps: None,
            auto_execute_ingress: false,
        },
    )
    .unwrap_err();
    match err {
        ContractError::InvalidBuffer { got: 3, .. } => {}
        other => panic!("expected InvalidBuffer, got {other:?}"),
    }
}

#[test]
fn settle_with_protocol_fee() {
    let mut deps = mock_dependencies();
    let admin = admin_addr();
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin, &[]),
        InstantiateMsg {
            admin: admin.to_string(),
            min_buffer_blocks: 5,
            accepted_denom: DENOM.into(),
            protocol_fee_bps: Some(1000), // 10%
            auto_execute_ingress: false,
        },
    )
    .unwrap();
    list_sku(&mut deps, "tee", 2, 100);

    let mut env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&buyer_addr(), &funds(100)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 1,
        },
    )
    .unwrap();
    env.block.height += 5;
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&buyer_addr(), &[]),
        ExecuteMsg::Settle { order_id: 1 },
    )
    .unwrap();
    // seller 90 + admin fee 10
    assert_eq!(res.messages.len(), 2);
}

#[test]
fn insufficient_funds_rejected() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 5, 100);
    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &funds(50)),
        ExecuteMsg::Purchase {
            sku: "tee".into(),
            qty: 1,
        },
    )
    .unwrap_err();
    match err {
        ContractError::InsufficientFunds { .. } => {}
        other => panic!("expected InsufficientFunds, got {other:?}"),
    }
}

#[test]
fn list_orders_query() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    list_sku(&mut deps, "tee", 10, 10);
    for _ in 0..3 {
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&buyer_addr(), &funds(10)),
            ExecuteMsg::Purchase {
                sku: "tee".into(),
                qty: 1,
            },
        )
        .unwrap();
    }
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ListOrders {
            start_after: None,
            limit: Some(10),
        },
    )
    .unwrap();
    let list: OrdersResponse = cosmwasm_std::from_json(bin).unwrap();
    assert_eq!(list.orders.len(), 3);
}

#[test]
fn admin_sets_auto_execute_ingress_non_admin_denied() {
    let mut deps = mock_dependencies();
    setup(&mut deps);

    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&buyer_addr(), &[]),
        ExecuteMsg::SetAutoExecuteIngress { enabled: true },
    )
    .unwrap_err();
    match err {
        ContractError::Unauthorized {} => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin_addr(), &[]),
        ExecuteMsg::SetAutoExecuteIngress { enabled: true },
    )
    .unwrap();
    let bin = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let cfg: ConfigResponse = cosmwasm_std::from_json(bin).unwrap();
    assert!(cfg.auto_execute_ingress);
}
