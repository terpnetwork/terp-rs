//! Multitest: mock_verify settle path + seam rejections.
//! Patterned on `crates/dex/contracts/pair` testing style (create → swap → sim)
//! and headstash multi-test addr_make usage.

use cosmwasm_std::{Addr, Binary, Empty, Uint128};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};

use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, Pool, PoolStatus, QueryMsg, QuoteResponse,
    SwapStatementPublic,
};
use crate::seams::quote_exact_in;
use crate::{ContractError, execute, instantiate, query};

fn contract() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new(execute, instantiate, query))
}

fn asset_hub() -> Binary {
    let mut a = [0u8; 32];
    a[0] = b'H';
    a[1] = b'U';
    a[2] = b'B';
    Binary::from(a.to_vec())
}
fn asset_b() -> Binary {
    let mut a = [0u8; 32];
    a[0] = b'B';
    Binary::from(a.to_vec())
}

fn setup() -> (App, Addr, Addr) {
    let mut app = App::default();
    let owner = app.api().addr_make("owner");
    let trader = app.api().addr_make("trader");
    let code_id = app.store_code(contract());
    let addr = app
        .instantiate_contract(
            code_id,
            owner.clone(),
            &InstantiateMsg {
                mock_verify: true,
                zkid: Some(42),
                allowed_root: Some(Binary::from(vec![7u8; 32])),
            },
            &[],
            "private-dex",
            None,
        )
        .unwrap();
    (app, addr, trader)
}

fn create_demo_pool(app: &mut App, addr: &Addr, owner: &Addr) {
    app.execute_contract(
        owner.clone(),
        addr.clone(),
        &ExecuteMsg::CreatePool {
            asset_a: asset_hub(),
            asset_b: asset_b(),
            r_a: Uint128::new(1_000_000),
            r_b: Uint128::new(2_000_000),
            gamma: 997,
            gamma_den: 1000,
        },
        &[],
    )
    .unwrap();
}

fn statement_for(delta_in: u128, nf_byte: u8) -> SwapStatementPublic {
    let r_in = 1_000_000u128;
    let r_out = 2_000_000u128;
    let delta_out = quote_exact_in(r_in, r_out, delta_in, 997, 1000).unwrap();
    SwapStatementPublic {
        pool_id: 1,
        asset_in: asset_hub(),
        asset_out: asset_b(),
        root: Binary::from(vec![7u8; 32]),
        nullifiers: vec![Binary::from(vec![nf_byte; 32])],
        cm_out: vec![Binary::from(vec![9u8; 32])],
        delta_r_in: Uint128::new(delta_in),
        delta_r_out: Uint128::new(delta_out),
        min_out: Uint128::new(1),
        gamma: 997,
        gamma_den: 1000,
        r_in_before: Uint128::new(r_in),
        r_out_before: Uint128::new(r_out),
        oracle_mid: None,
        oracle_params: None,
    }
}

#[test]
fn instantiate_and_config() {
    let (app, addr, _) = setup();
    let cfg: ConfigResponse = app
        .wrap()
        .query_wasm_smart(addr, &QueryMsg::Config {})
        .unwrap();
    assert!(cfg.mock_verify);
    assert_eq!(cfg.zkid, Some(42));
    assert_eq!(cfg.next_pool_id, 1);
    assert!(!cfg.zk_api_compiled);
}

#[test]
fn create_pool_and_quote() {
    let (mut app, addr, _) = setup();
    let owner = app.api().addr_make("owner");
    create_demo_pool(&mut app, &addr, &owner);

    let pool: Pool = app
        .wrap()
        .query_wasm_smart(addr.clone(), &QueryMsg::Pool { pool_id: 1 })
        .unwrap();
    assert_eq!(pool.r_a, Uint128::new(1_000_000));
    assert!(matches!(pool.status, PoolStatus::Active));

    let q: QuoteResponse = app
        .wrap()
        .query_wasm_smart(
            addr,
            &QueryMsg::QuoteExactIn {
                pool_id: 1,
                asset_in: asset_hub(),
                delta_in: Uint128::new(1000),
            },
        )
        .unwrap();
    assert!(q.delta_out.u128() > 0);
}

#[test]
fn settle_swap_mock_verify_updates_reserves_and_nullifier() {
    let (mut app, addr, trader) = setup();
    let owner = app.api().addr_make("owner");
    create_demo_pool(&mut app, &addr, &owner);

    let st = statement_for(1000, 0xAB);
    let delta_out = st.delta_r_out.u128();

    app.execute_contract(
        trader.clone(),
        addr.clone(),
        &ExecuteMsg::SettleSwap {
            statement: Box::new(st.clone()),
            proof: Binary::from(b"mock-proof".as_slice()),
        },
        &[],
    )
    .unwrap();

    let pool: Pool = app
        .wrap()
        .query_wasm_smart(addr.clone(), &QueryMsg::Pool { pool_id: 1 })
        .unwrap();
    assert_eq!(pool.r_a, Uint128::new(1_001_000));
    assert_eq!(pool.r_b, Uint128::new(2_000_000 - delta_out));

    let spent: bool = app
        .wrap()
        .query_wasm_smart(
            addr.clone(),
            &QueryMsg::IsNullifierSpent {
                nullifier: st.nullifiers[0].clone(),
            },
        )
        .unwrap();
    assert!(spent);

    // double-spend rejected
    let err = app
        .execute_contract(
            trader,
            addr,
            &ExecuteMsg::SettleSwap {
                statement: Box::new(statement_for(1000, 0xAB)),
                proof: Binary::from(b"mock-proof-2".as_slice()),
            },
            &[],
        )
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("nullifier"),
        "unexpected err: {msg}"
    );
}

#[test]
fn settle_rejects_empty_proof_under_mock() {
    let (mut app, addr, trader) = setup();
    let owner = app.api().addr_make("owner");
    create_demo_pool(&mut app, &addr, &owner);
    let err = app
        .execute_contract(
            trader,
            addr,
            &ExecuteMsg::SettleSwap {
                statement: Box::new(statement_for(1000, 0x01)),
                proof: Binary::default(),
            },
            &[],
        )
        .unwrap_err();
    assert!(err.to_string().contains("empty proof"));
}

#[test]
fn settle_rejects_curve_mismatch() {
    let (mut app, addr, trader) = setup();
    let owner = app.api().addr_make("owner");
    create_demo_pool(&mut app, &addr, &owner);
    let mut st = statement_for(1000, 0x02);
    st.delta_r_out = Uint128::new(1); // wrong
    let err = app
        .execute_contract(
            trader,
            addr,
            &ExecuteMsg::SettleSwap {
                statement: Box::new(st),
                proof: Binary::from(b"p".as_slice()),
            },
            &[],
        )
        .unwrap_err();
    let s = err.to_string();
    assert!(s.to_lowercase().contains("curve"), "unexpected: {s}");
}

#[test]
fn paused_pool_rejects_swap() {
    let (mut app, addr, trader) = setup();
    let owner = app.api().addr_make("owner");
    create_demo_pool(&mut app, &addr, &owner);
    app.execute_contract(
        owner,
        addr.clone(),
        &ExecuteMsg::SetPoolStatus {
            pool_id: 1,
            status: PoolStatus::Paused,
        },
        &[],
    )
    .unwrap();
    let err = app
        .execute_contract(
            trader,
            addr,
            &ExecuteMsg::SettleSwap {
                statement: Box::new(statement_for(1000, 0x03)),
                proof: Binary::from(b"p".as_slice()),
            },
            &[],
        )
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("pause"));
}

#[test]
fn bad_root_rejected() {
    let (mut app, addr, trader) = setup();
    let owner = app.api().addr_make("owner");
    create_demo_pool(&mut app, &addr, &owner);
    let mut st = statement_for(1000, 0x04);
    st.root = Binary::from(vec![0u8; 32]);
    let err = app
        .execute_contract(
            trader,
            addr,
            &ExecuteMsg::SettleSwap {
                statement: Box::new(st),
                proof: Binary::from(b"p".as_slice()),
            },
            &[],
        )
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("root"));
}

#[test]
fn production_cfg_cfg_test_still_allows_mock_path() {
    // Honest: under cfg!(test), allow_mock is true even if mock_verify=false.
    let mut app = App::default();
    let owner = app.api().addr_make("owner");
    let trader = app.api().addr_make("trader");
    let code_id = app.store_code(contract());
    let addr = app
        .instantiate_contract(
            code_id,
            owner.clone(),
            &InstantiateMsg {
                mock_verify: false,
                zkid: Some(1),
                allowed_root: None,
            },
            &[],
            "private-dex-prod",
            None,
        )
        .unwrap();
    create_demo_pool(&mut app, &addr, &owner);

    app.execute_contract(
        trader,
        addr,
        &ExecuteMsg::SettleSwap {
            statement: Box::new(statement_for(500, 0x55)),
            proof: Binary::from(b"test-proof".as_slice()),
        },
        &[],
    )
    .unwrap();
}

// Keep ContractError in tree for compile.
#[allow(dead_code)]
fn _err_variant() -> ContractError {
    ContractError::BadAmount {}
}
