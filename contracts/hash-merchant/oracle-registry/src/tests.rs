use cosmwasm_std::testing::{MockApi, message_info, mock_dependencies, mock_env};
use cosmwasm_std::{
    Addr, Binary, Coin, CosmosMsg, Decimal, Ibc2Msg, Ibc2PacketAckMsg, Ibc2PacketSendMsg,
    Ibc2PacketTimeoutMsg, Ibc2Payload, StdAck, from_json, to_json_binary,
};

use crate::contract::{
    ALGO_PRICE_MEDIAN_V1, UNIVERSAL_ERROR_ACK, execute, height_weighted_twap, ibc2_packet_ack,
    ibc2_packet_send, ibc2_packet_timeout, ibc2_source_port, instantiate, mid_from_posted_price,
    parse_dex_quote_ack, query, sudo,
};
use crate::error::ContractError;
use crate::msg::*;
use crate::state::{Observation, SourceKind};

fn admin() -> Addr {
    MockApi::default().addr_make("admin")
}
fn relayer() -> Addr {
    MockApi::default().addr_make("relayer")
}

fn init_open(
    deps: &mut cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >,
) {
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        InstantiateMsg {
            admin: admin().to_string(),
            open_submit: true,
            bounty: Some(Coin::new(1u128, "uterp")),
            note: None,
            ibc2_source_client: None,
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
        position_id: None,
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
            ibc2_source_client: None,
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
            ibc2_source_client: None,
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
            result: PolytoneCallback::Query(PolytoneQueryResult::Success(vec![Binary::from(
                twap.to_vec(),
            )])),
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
            ibc2_source_client: None,
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

const CLIENT: &str = "07-tendermint-0";

type Deps = cosmwasm_std::OwnedDeps<
    cosmwasm_std::testing::MockStorage,
    cosmwasm_std::testing::MockApi,
    cosmwasm_std::testing::MockQuerier,
>;

fn pair() -> DexPair {
    DexPair {
        asset_1: "penumbra".into(),
        asset_2: "usdc".into(),
    }
}

fn sample_ack() -> DexQuoteAck {
    DexQuoteAck {
        position_id: "pos-1".into(),
        pair: pair(),
        reserves: DexReserves {
            r1: "1000".into(),
            r2: "2500".into(),
        },
        p: "5".into(),
        q: "2".into(),
    }
}

fn register(deps: &mut Deps) {
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::RegisterIbcV2 {
            source_client: CLIENT.into(),
        },
    )
    .unwrap();
}

fn request(deps: &mut Deps) -> DexQuoteRequest {
    let res = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&relayer(), &[]),
        ExecuteMsg::RequestQuote {
            market_id: "PEN/USD".into(),
            pair: pair(),
            position_id: String::new(),
        },
    )
    .unwrap();
    let CosmosMsg::Ibc2(Ibc2Msg::SendPacket {
        source_client,
        payloads,
        ..
    }) = &res.messages[0].msg
    else {
        panic!("expected Ibc2Msg::SendPacket");
    };
    assert_eq!(source_client, CLIENT);
    assert_eq!(payloads[0].destination_port, DEX_QUOTE_PORT);
    assert_eq!(payloads[0].version, DEX_QUOTE_VERSION);
    assert_eq!(payloads[0].encoding, DEX_QUOTE_ENCODING);
    assert_eq!(
        payloads[0].source_port,
        ibc2_source_port(&mock_env().contract.address)
    );
    from_json(&payloads[0].value).unwrap()
}

fn ack_msg(req: &DexQuoteRequest, raw: impl Into<Binary>) -> Ibc2PacketAckMsg {
    Ibc2PacketAckMsg::new(
        CLIENT.into(),
        "dest-client".into(),
        Ibc2Payload::new(
            ibc2_source_port(&mock_env().contract.address),
            DEX_QUOTE_PORT.into(),
            DEX_QUOTE_VERSION.into(),
            DEX_QUOTE_ENCODING.into(),
            to_json_binary(req).unwrap(),
        ),
        raw.into(),
        Addr::unchecked("relayer"),
    )
}

#[test]
fn posted_price_is_p_over_q() {
    assert_eq!(
        parse_dex_quote_ack(&to_json_binary(&sample_ack()).unwrap())
            .unwrap()
            .position_id,
        "pos-1"
    );
    assert!(matches!(
        parse_dex_quote_ack(&UNIVERSAL_ERROR_ACK),
        Err(ContractError::NoQuote {})
    ));
    assert!(matches!(
        parse_dex_quote_ack(StdAck::error("missing position").to_binary().as_slice()),
        Err(ContractError::NoQuote {})
    ));
}

#[test]
fn dex_quote_ack_records_same_observation_shape() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    register(&mut deps);
    let req = request(&mut deps);
    let send = Ibc2PacketSendMsg::new(
        Ibc2Payload::new(
            ibc2_source_port(&mock_env().contract.address),
            DEX_QUOTE_PORT.into(),
            DEX_QUOTE_VERSION.into(),
            DEX_QUOTE_ENCODING.into(),
            to_json_binary(&req).unwrap(),
        ),
        CLIENT.into(),
        "dest-client".into(),
        1,
        relayer(),
    );
    ibc2_packet_send(deps.as_mut(), mock_env(), send).unwrap();

    ibc2_packet_ack(
        deps.as_mut(),
        mock_env(),
        ack_msg(&req, to_json_binary(&sample_ack()).unwrap()),
    )
    .unwrap();
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Latest {
            market_id: "PEN/USD".into(),
        },
    )
    .unwrap();
    let latest: ObservationResponse = from_json(bin).unwrap();
    assert_eq!(latest.source, SourceKind::DexQuote);
    assert_eq!(latest.mid, Decimal::from_ratio(5u128, 2u128));
    assert_eq!(latest.pool_id, None);
    assert_eq!(latest.source_chain, CLIENT);
    assert_eq!(latest.position_id.as_deref(), Some("pos-1"));
    assert_eq!(latest.height, mock_env().block.height);
    assert_eq!(latest.recorder, mock_env().contract.address);
}

#[test]
fn five_penumbra_display_price_is_base_unit_mid() {
    // 5 penumbra = 5_000_000 base. Selling 100 base uterp posts
    // p = 500_000_000, q = 100. mid is that ratio, not the display number 5.
    let mid = mid_from_posted_price("500000000", "100").unwrap();
    assert_eq!(mid, "5000000");
}

#[test]
fn universal_error_ack_does_not_record() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    register(&mut deps);
    let req = request(&mut deps);
    ibc2_packet_ack(
        deps.as_mut(),
        mock_env(),
        ack_msg(&req, Binary::from(UNIVERSAL_ERROR_ACK.to_vec())),
    )
    .unwrap();
    let err = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Latest {
            market_id: "PEN/USD".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::Std(_)));
}

#[test]
fn timeout_does_not_record() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    register(&mut deps);
    let req = request(&mut deps);
    let timeout = Ibc2PacketTimeoutMsg::new(
        Ibc2Payload::new(
            "src".into(),
            DEX_QUOTE_PORT.into(),
            DEX_QUOTE_VERSION.into(),
            DEX_QUOTE_ENCODING.into(),
            to_json_binary(&req).unwrap(),
        ),
        CLIENT.into(),
        "dest-client".into(),
        1,
        Addr::unchecked("relayer"),
    );
    ibc2_packet_timeout(deps.as_mut(), mock_env(), timeout).unwrap();
    let err = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Latest {
            market_id: "PEN/USD".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::Std(_)));
}

#[test]
fn register_is_admin_only() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&relayer(), &[]),
        ExecuteMsg::RegisterIbcV2 {
            source_client: CLIENT.into(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::Unauthorized {}));
}

#[test]
fn batch_ack_files_each_hit_and_skips_the_miss() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    register(&mut deps);
    let res = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&relayer(), &[]),
        ExecuteMsg::RequestQuotes {
            quotes: vec![
                DexQuoteRequestItem {
                    market_id: "PEN/USD".into(),
                    pair: pair(),
                    position_id: String::new(),
                },
                DexQuoteRequestItem {
                    market_id: "PEN/ATOM".into(),
                    pair: DexPair {
                        asset_1: "penumbra".into(),
                        asset_2: "atom".into(),
                    },
                    position_id: String::new(),
                },
            ],
        },
    )
    .unwrap();
    let CosmosMsg::Ibc2(Ibc2Msg::SendPacket { payloads, .. }) = &res.messages[0].msg else {
        panic!("expected Ibc2Msg::SendPacket");
    };
    assert_eq!(payloads.len(), 1);
    let batch: DexQuoteBatch = from_json(&payloads[0].value).unwrap();
    assert_eq!(batch.quotes.len(), 2);
    let ack = DexQuoteBatchAck {
        quotes: vec![
            DexQuoteBatchItem {
                market_id: "PEN/USD".into(),
                height: batch.quotes[0].height,
                error: String::new(),
                position_id: "pos-1".into(),
                pair: Some(pair()),
                reserves: Some(sample_ack().reserves),
                p: "5".into(),
                q: "2".into(),
            },
            DexQuoteBatchItem {
                market_id: "PEN/ATOM".into(),
                height: batch.quotes[1].height,
                error: "missing position".into(),
                position_id: String::new(),
                pair: None,
                reserves: None,
                p: String::new(),
                q: String::new(),
            },
        ],
    };
    ibc2_packet_ack(
        deps.as_mut(),
        mock_env(),
        Ibc2PacketAckMsg::new(
            CLIENT.into(),
            "dest-client".into(),
            payloads[0].clone(),
            to_json_binary(&ack).unwrap(),
            Addr::unchecked("relayer"),
        ),
    )
    .unwrap();
    let latest: ObservationResponse = from_json(
        query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Latest {
                market_id: "PEN/USD".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(latest.mid, Decimal::from_ratio(5u128, 2u128));
    assert_eq!(latest.source, SourceKind::DexQuote);
    let missing = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Latest {
            market_id: "PEN/ATOM".into(),
        },
    );
    assert!(missing.is_err());
}

/// ICS-20 traces from a Terp→Penumbra transfer. UM is the Penumbra staking
/// token, quoted as the `penumbra` unit. The two mids stay in their own
/// markets: thiol per terp is not UM per terp.
fn bridged_terp() -> DexPair {
    DexPair {
        asset_1: "transfer/channel-0/uterp".into(),
        asset_2: "penumbra".into(),
    }
}

fn bridged_thiol() -> DexPair {
    DexPair {
        asset_1: "transfer/channel-0/uterp".into(),
        asset_2: "transfer/channel-0/uthiol".into(),
    }
}

fn request_both(deps: &mut Deps, env: &cosmwasm_std::Env) -> Ibc2Payload {
    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&relayer(), &[]),
        ExecuteMsg::RequestQuotes {
            quotes: vec![
                DexQuoteRequestItem {
                    market_id: "TERP/UM".into(),
                    pair: bridged_terp(),
                    position_id: String::new(),
                },
                DexQuoteRequestItem {
                    market_id: "TERP/THIOL".into(),
                    pair: bridged_thiol(),
                    position_id: String::new(),
                },
            ],
        },
    )
    .unwrap();
    let CosmosMsg::Ibc2(Ibc2Msg::SendPacket { payloads, .. }) = &res.messages[0].msg else {
        panic!("expected one dex-quote packet");
    };
    assert_eq!(payloads.len(), 1);
    payloads[0].clone()
}

fn file_batch(
    deps: &mut Deps,
    env: &cosmwasm_std::Env,
    payload: Ibc2Payload,
    ack: DexQuoteBatchAck,
) {
    ibc2_packet_ack(
        deps.as_mut(),
        env.clone(),
        Ibc2PacketAckMsg::new(
            CLIENT.into(),
            "dest-client".into(),
            payload,
            to_json_binary(&ack).unwrap(),
            Addr::unchecked("relayer"),
        ),
    )
    .unwrap();
}

fn hit(
    market_id: &str,
    height: u64,
    pair: DexPair,
    position_id: &str,
    p: &str,
    q: &str,
    r2: &str,
) -> DexQuoteBatchItem {
    DexQuoteBatchItem {
        market_id: market_id.into(),
        height,
        error: String::new(),
        position_id: position_id.into(),
        pair: Some(pair),
        reserves: Some(DexReserves {
            r1: "1000".into(),
            r2: r2.into(),
        }),
        p: p.into(),
        q: q.into(),
    }
}

fn latest(deps: &Deps, market_id: &str) -> ObservationResponse {
    from_json(
        query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Latest {
                market_id: market_id.into(),
            },
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn terp_um_and_terp_thiol_follow_the_penumbra_positions() {
    let mut deps = mock_dependencies();
    init_open(&mut deps);
    register(&mut deps);

    let mut opened = mock_env();
    opened.block.height = 10;
    let payload = request_both(&mut deps, &opened);
    let batch: DexQuoteBatch = from_json(&payload.value).unwrap();
    assert_eq!(batch.quotes.len(), 2);
    file_batch(
        &mut deps,
        &opened,
        payload,
        DexQuoteBatchAck {
            quotes: vec![
                hit(
                    "TERP/UM",
                    10,
                    bridged_terp(),
                    "plpid-um-1",
                    "2",
                    "1",
                    "2000",
                ),
                hit(
                    "TERP/THIOL",
                    10,
                    bridged_thiol(),
                    "plpid-thiol-1",
                    "4",
                    "1",
                    "4000",
                ),
            ],
        },
    );
    let um = latest(&deps, "TERP/UM");
    let thiol = latest(&deps, "TERP/THIOL");
    assert_eq!(um.mid, Decimal::from_ratio(2u128, 1u128));
    assert_eq!(um.position_id.as_deref(), Some("plpid-um-1"));
    assert_eq!(um.source, SourceKind::DexQuote);
    assert_eq!(thiol.mid, Decimal::from_ratio(4u128, 1u128));
    assert_eq!(thiol.position_id.as_deref(), Some("plpid-thiol-1"));

    // The UM position was closed and a new one opened. Thiol did not move.
    let mut moved = mock_env();
    moved.block.height = 20;
    let payload = request_both(&mut deps, &moved);
    file_batch(
        &mut deps,
        &moved,
        payload,
        DexQuoteBatchAck {
            quotes: vec![
                hit(
                    "TERP/UM",
                    20,
                    bridged_terp(),
                    "plpid-um-2",
                    "5",
                    "1",
                    "5000",
                ),
                hit(
                    "TERP/THIOL",
                    20,
                    bridged_thiol(),
                    "plpid-thiol-1",
                    "4",
                    "1",
                    "4000",
                ),
            ],
        },
    );
    let um = latest(&deps, "TERP/UM");
    let thiol = latest(&deps, "TERP/THIOL");
    assert_eq!(um.mid, Decimal::from_ratio(5u128, 1u128));
    assert_eq!(um.position_id.as_deref(), Some("plpid-um-2"));
    assert_eq!(um.height, 20);
    assert_eq!(thiol.mid, Decimal::from_ratio(4u128, 1u128));
    assert_eq!(thiol.position_id.as_deref(), Some("plpid-thiol-1"));

    let twap: TwapResponse = from_json(
        query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Twap {
                market_id: "TERP/UM".into(),
                from_height: 10,
                to_height: 20,
            },
        )
        .unwrap(),
    )
    .unwrap();
    // Height 10 holds mid 2 for 10 blocks, then mid 5 for the closing block.
    assert_eq!(twap.n, 2);
    assert_eq!(twap.twap, Decimal::from_ratio(25u128, 11u128));

    let thiol_twap: TwapResponse = from_json(
        query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Twap {
                market_id: "TERP/THIOL".into(),
                from_height: 10,
                to_height: 20,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(thiol_twap.twap, Decimal::from_ratio(4u128, 1u128));
}
