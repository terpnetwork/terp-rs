use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi};
use cosmwasm_std::{from_json, to_json_binary, Addr, Binary, Coin, Decimal};

use crate::contract::{execute, height_weighted_twap, instantiate, query, sudo, ALGO_PRICE_MEDIAN_V1};
use crate::error::ContractError;
use crate::msg::*;
use crate::state::{Observation, SourceKind};

fn admin() -> Addr {
    MockApi::default().addr_make("admin")
}
fn relayer() -> Addr {
    MockApi::default().addr_make("relayer")
}

fn init_open(deps: &mut cosmwasm_std::OwnedDeps<
    cosmwasm_std::testing::MockStorage,
    cosmwasm_std::testing::MockApi,
    cosmwasm_std::testing::MockQuerier,
>) {
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        InstantiateMsg {
            admin: admin().to_string(),
            open_submit: true,
            bounty: Some(Coin::new(1u128, "uterp")),
            note: None,
        },
    )
    .unwrap();
}

#[test]
fn first_submit_wins() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&relayer(), &[]),
        ExecuteMsg::Submit {
            market_id: "OSMO/USD".into(),
            source: SourceKind::Relayer,
            source_chain: "osmosis-test-1".into(),
            pool_id: Some(1),
            height: 10,
            mid: "1.50".into(),
        },
    )
    .unwrap();
    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::Submit {
            market_id: "OSMO/USD".into(),
            source: SourceKind::Polytone,
            source_chain: "osmosis-test-1".into(),
            pool_id: Some(1),
            height: 10,
            mid: "9.99".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::AlreadyRecorded {}));

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Latest {
            market_id: "OSMO/USD".into(),
        },
    )
    .unwrap();
    let latest: ObservationResponse = from_json(bin).unwrap();
    assert_eq!(latest.mid, Decimal::percent(150));
    assert_eq!(latest.recorder, relayer());
}

#[test]
fn twap_weights_by_height() {
    let a = Observation {
        market_id: "OSMO/USD".into(),
        source: SourceKind::Relayer,
        source_chain: "osmosis-test-1".into(),
        pool_id: None,
        height: 10,
        mid: Decimal::percent(100),
        recorder: relayer(),
    };
    let b = Observation {
        height: 12,
        mid: Decimal::percent(200),
        ..a.clone()
    };
    // mid 1.00 for 2 heights, then 2.00 for 1 height → (2*1 + 1*2) / 3 = 4/3
    let t = height_weighted_twap(&[a, b]);
    assert_eq!(t, Decimal::from_ratio(4u128, 3u128));
}

#[test]
fn history_and_twap_query() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    for (h, mid) in [(10u64, "1.0"), (12, "2.0")] {
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&relayer(), &[]),
            ExecuteMsg::Submit {
                market_id: "OSMO/USD".into(),
                source: SourceKind::Relayer,
                source_chain: "osmosis-test-1".into(),
                pool_id: None,
                height: h,
                mid: mid.into(),
            },
        )
        .unwrap();
    }
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Twap {
            market_id: "OSMO/USD".into(),
            from_height: 10,
            to_height: 12,
        },
    )
    .unwrap();
    let tw: TwapResponse = from_json(bin).unwrap();
    assert_eq!(tw.n, 2);
    assert_eq!(tw.twap, Decimal::from_ratio(4u128, 3u128));
}

#[test]
fn sudo_price_median_records_ve() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    sudo(
        deps.as_mut(),
        mock_env(),
        SudoMsg::HashMerchant {
            chain_uid: "OSMO/USD".into(),
            algo: ALGO_PRICE_MEDIAN_V1.into(),
            root: "3.25".into(),
            height: 99,
            attestation_count: 3,
            block_time: 0,
        },
    )
    .unwrap();
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Latest {
            market_id: "OSMO/USD".into(),
        },
    )
    .unwrap();
    let latest: ObservationResponse = from_json(bin).unwrap();
    assert_eq!(latest.source, SourceKind::VeQuorum);
    assert_eq!(latest.mid, Decimal::from_atomics(325u128, 2).unwrap());
}

#[test]
fn closed_submit_needs_relayer() {
    let mut deps = mock_dependencies();
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        InstantiateMsg {
            admin: admin().to_string(),
            open_submit: false,
            bounty: None,
            note: None,
        },
    )
    .unwrap();
    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&relayer(), &[]),
        ExecuteMsg::Submit {
            market_id: "OSMO/USD".into(),
            source: SourceKind::Relayer,
            source_chain: "osmosis-test-1".into(),
            pool_id: None,
            height: 1,
            mid: "1".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::Unauthorized {}));
}

#[test]
fn polytone_callback_records_query_mid() {
    let mut deps = mock_dependencies();
    let note = MockApi::default().addr_make("note");
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        InstantiateMsg {
            admin: admin().to_string(),
            open_submit: false,
            bounty: None,
            note: Some(note.to_string()),
        },
    )
    .unwrap();
    let hint = CallbackHint {
        market_id: "OSMO/USD".into(),
        source_chain: "osmosis-test-1".into(),
        pool_id: Some(1),
        height: 42,
    };
    let twap = br#"{"arithmetic_twap":"1.25"}"#;
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&note, &[]),
        ExecuteMsg::Callback(PolytoneCallbackMessage {
            initiator: relayer(),
            initiator_msg: to_json_binary(&hint).unwrap(),
            result: PolytoneCallback::Query(PolytoneQueryResult::Success(vec![
                Binary::from(twap.to_vec()),
            ])),
        }),
    )
    .unwrap();
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Latest {
            market_id: "OSMO/USD".into(),
        },
    )
    .unwrap();
    let latest: ObservationResponse = from_json(bin).unwrap();
    assert_eq!(latest.source, SourceKind::Polytone);
    assert_eq!(latest.mid, Decimal::from_atomics(125u128, 2).unwrap());
    assert_eq!(latest.height, 42);
    assert_eq!(latest.recorder, relayer());
}

#[test]
fn polytone_callback_rejects_wrong_note() {
    let mut deps = mock_dependencies();
    let note = MockApi::default().addr_make("note");
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        InstantiateMsg {
            admin: admin().to_string(),
            open_submit: true,
            bounty: None,
            note: Some(note.to_string()),
        },
    )
    .unwrap();
    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&relayer(), &[]),
        ExecuteMsg::Callback(PolytoneCallbackMessage {
            initiator: relayer(),
            initiator_msg: to_json_binary(&CallbackHint {
                market_id: "OSMO/USD".into(),
                source_chain: "osmosis-test-1".into(),
                pool_id: None,
                height: 1,
            })
            .unwrap(),
            result: PolytoneCallback::Query(PolytoneQueryResult::Success(vec![Binary::from(
                br#""1""#.to_vec(),
            )])),
        }),
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::Unauthorized {}));
}


