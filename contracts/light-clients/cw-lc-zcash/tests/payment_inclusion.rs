//! zap-1 payment inclusion against committee-attested payment roots.
//! The aggregator is stubbed (records set directly) so the test isolates the
//! client's checks; the aggregator's own committee logic is covered in
//! `cw-lc-aggregator/tests/committee.rs`.
use cosmwasm_std::{
    Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdError, StdResult, to_json_binary,
};
use cw_lc_aggregator::msg::{BestHeaderResponse, Header, HeaderSource, QueryMsg as AggQ};
use cw_lc_aggregator::state::{PreconfRecord, PreconfStatus};
use cw_lc_zcash::msg::*;
use cw_multi_test::{App, ContractWrapper, Executor};
use cw_storage_plus::Map;
use zap1_verify::{EventPayload, compute_leaf_hash, node_hash};

const RECS: Map<u64, PreconfRecord> = Map::new("r");

fn stub_exec(d: DepsMut, _e: Env, _i: MessageInfo, r: PreconfRecord) -> StdResult<Response> {
    RECS.save(d.storage, r.header.height, &r)?;
    Ok(Response::new())
}
fn stub_inst(_d: DepsMut, _e: Env, _i: MessageInfo, _m: Empty) -> StdResult<Response> {
    Ok(Response::new())
}
fn stub_query(d: Deps, _e: Env, q: AggQ) -> StdResult<Binary> {
    match q {
        AggQ::Preconf { height } => to_json_binary(&RECS.may_load(d.storage, height)?),
        AggQ::BestHeader {} => to_json_binary(&BestHeaderResponse {
            source: HeaderSource::Preconf,
            height: Some(7),
            hash: None,
            epoch: Some(1),
            status: None,
        }),
        _ => Err(StdError::msg("stub")),
    }
}

fn rec(height: u64, hash: u8, root: Option<[u8; 32]>, status: PreconfStatus) -> PreconfRecord {
    PreconfRecord {
        header: Header {
            height,
            hash: hex::encode([hash; 32]),
            prev_hash: hex::encode([0u8; 32]),
            chain_work: "0f".into(),
            payment_root: root.map(hex::encode),
        },
        epoch: 1,
        signature: Binary::default(),
        submitted_at: 1,
        status,
    }
}

#[test]
fn payment_inclusion_lifecycle() {
    let mut app = App::default();
    let admin = app.api().addr_make("admin");
    let stub = app.store_code(Box::new(ContractWrapper::new(stub_exec, stub_inst, stub_query)));
    let agg = app
        .instantiate_contract(stub, admin.clone(), &Empty {}, &[], "agg", None)
        .unwrap();
    let code = app.store_code(Box::new(ContractWrapper::new_with_empty(
        cw_lc_zcash::contract::execute,
        cw_lc_zcash::contract::instantiate,
        cw_lc_zcash::contract::query,
    )));
    let client = app
        .instantiate_contract(
            code,
            admin.clone(),
            &InstantiateMsg {
                label: "terp/lc/client/zcash-regtest/v1".into(),
                registry: agg.to_string(),
                aggregator: agg.to_string(),
                registry_client_id: "zcash-regtest-0".into(),
                ibc_client_id: "08-wasm-zcash-0".into(),
                accept_preconfirmed: false,
            },
            &[],
            "client",
            None,
        )
        .unwrap();

    // Two-leaf zap-1 tree: our payment + a sibling.
    let leaf = compute_leaf_hash(&EventPayload::HostingPayment {
        serial_number: b"auction-lot-42",
        month: 8,
        year: 2026,
    });
    let sib = [0x33u8; 32];
    let root = node_hash(&leaf, &sib);
    let (note, frost) = ([0x11u8; 32], [0x22u8; 32]);
    let adh = cw_ics08_wasm_zap1::app_data_hash(&note, 50_000, &frost, &leaf);
    let msg = |height: u64, seq: u64, adh: [u8; 32]| {
        ExecuteMsg::VerifyPaymentInclusion(Box::new(PaymentInclusionMsg {
            height,
            source_client: "zcash-0".into(),
            dest_client: "08-wasm-zcash-0".into(),
            sequence: seq,
            timeout_timestamp_ns: 0,
            app_data_hash_hex: hex::encode(adh),
            note_hex: hex::encode(note),
            amount_zat: 50_000,
            frost_group_hex: hex::encode(frost),
            leaf_hex: hex::encode(leaf),
            event_kind: "hosting_payment".into(),
            wallet_or_serial: "auction-lot-42".into(),
            proof: vec![Zap1Step {
                sibling_hex: hex::encode(sib),
                sibling_is_left: false,
            }],
        }))
    };
    let relayer = app.api().addr_make("relayer");
    let run = |app: &mut App, m: &ExecuteMsg| {
        app.execute_contract(relayer.clone(), client.clone(), m, &[])
            .map_err(|e| format!("{e:?}"))
    };
    let set = |app: &mut App, r: PreconfRecord| {
        app.execute_contract(admin.clone(), agg.clone(), &r, &[]).unwrap();
    };

    assert!(run(&mut app, &msg(7, 1, adh)).unwrap_err().contains("no preconfirmed header"));
    set(&mut app, rec(7, 0xa0, Some(root), PreconfStatus::Preconfirmed));
    assert!(run(&mut app, &msg(7, 1, adh)).unwrap_err().contains("requires Confirmed"));
    set(&mut app, rec(7, 0xa0, None, PreconfStatus::Confirmed));
    assert!(run(&mut app, &msg(7, 1, adh)).unwrap_err().contains("no payment root"));
    set(&mut app, rec(7, 0xa0, Some([9; 32]), PreconfStatus::Confirmed));
    assert!(run(&mut app, &msg(7, 1, adh)).unwrap_err().contains("merkle proof failed"));
    set(&mut app, rec(7, 0xa0, Some(root), PreconfStatus::Confirmed));
    assert!(run(&mut app, &msg(7, 1, [1; 32])).unwrap_err().contains("tampered"));
    run(&mut app, &msg(7, 1, adh)).unwrap();
    assert!(run(&mut app, &msg(7, 1, adh)).unwrap_err().contains("already used"));

    let q = |app: &App| -> Option<PaymentInclusionResponse> {
        app.wrap()
            .query_wasm_smart(
                &client,
                &QueryMsg::PaymentInclusion {
                    app_data_hash_hex: hex::encode(adh),
                },
            )
            .unwrap()
    };
    let r = q(&app).unwrap();
    assert_eq!(r.status, InclusionStatus::Confirmed);
    assert_eq!((r.inclusion.height, r.inclusion.amount_zat), (7, 50_000));
    // Header at 7 replaced → settlement must see Reorged.
    set(&mut app, rec(7, 0xb0, Some(root), PreconfStatus::Preconfirmed));
    assert_eq!(q(&app).unwrap().status, InclusionStatus::Reorged);
    let none: Option<PaymentInclusionResponse> = app
        .wrap()
        .query_wasm_smart(
            &client,
            &QueryMsg::PaymentInclusion {
                app_data_hash_hex: hex::encode([0; 32]),
            },
        )
        .unwrap();
    assert!(none.is_none());
    let best: BestHeaderResponse = app
        .wrap()
        .query_wasm_smart(&client, &QueryMsg::LatestHeader {})
        .unwrap();
    assert_eq!(best.height, Some(7));
}
