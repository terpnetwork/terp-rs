//! Pure IBC authenticity unit tests (no Docker).

use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use terp_scripts::ibc::{
    IBCAssetRoutingTable, IBCChannelGraph, PredictedWorld, build_channel_to_chain_map,
    check_invariants, compute_ibc_denom_hash, finalize_channels_for_ibc_entry,
    hop_count_from_trace_path, ordering_to_str, validate_ibc_data_entry,
};

#[test]
fn hash_format_and_known_akt_osmosis() {
    let h = compute_ibc_denom_hash("transfer/channel-1/uakt");
    assert!(h.starts_with("ibc/"));
    assert_eq!(h.len(), 68);
    assert_eq!(
        h,
        "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4"
    );
}

#[test]
fn schema_rejects_int_ordering() {
    let entry = json!({
        "$schema": "../ibc_data.schema.json",
        "chain_1": { "chain_name": "terp", "chain_id": "x", "client_id": "y", "connection_id": "z" },
        "chain_2": { "chain_name": "osmo", "chain_id": "x", "client_id": "y", "connection_id": "z" },
        "channels": [{
            "chain_1": { "channel_id": "channel-0", "port_id": "transfer" },
            "chain_2": { "channel_id": "channel-1", "port_id": "transfer" },
            "ordering": 1,
            "version": "ics20-1",
        }],
    });
    let errs = validate_ibc_data_entry(&entry, "terp-osmo");
    assert!(
        errs.iter().any(|e| e.contains("ordering")),
        "expected ordering violation: {errs:?}"
    );
}

#[test]
fn ordering_to_str_maps_grpc_ints() {
    assert_eq!(ordering_to_str(&json!(1)), "unordered");
    assert_eq!(ordering_to_str(&json!(2)), "ordered");
}

#[test]
fn hop_count_honesty_invariant() {
    let path = "transfer/channel-0/transfer/channel-1/uakt";
    assert_eq!(hop_count_from_trace_path(path), 2);
    // A world that sets hop_count=1 with this path is dishonest
    assert_ne!(1, hop_count_from_trace_path(path));
}

#[test]
fn preferred_non_transfer_is_error() {
    let ibc = json!({
        "a-b": {
            "chain_1": { "chain_name": "a", "chain_id": "a", "client_id": "c", "connection_id": "n" },
            "chain_2": { "chain_name": "b", "chain_id": "b", "client_id": "c", "connection_id": "n" },
            "channels": [{
                "chain_1": { "channel_id": "channel-0", "port_id": "wasm.x" },
                "chain_2": { "channel_id": "channel-0", "port_id": "wasm.x" },
                "ordering": "unordered",
                "version": "ics27-1",
                "tags": { "preferred": true, "status": "ACTIVE" }
            }]
        }
    });
    let world = PredictedWorld::from_inputs(ibc, &HashMap::new(), 1);
    let report = check_invariants(&world);
    assert!(
        report.errors().any(|e| e.code == "preferred_port"),
        "expected preferred_port: {:?}",
        report.items
    );
}

#[test]
fn prefer_direct_triangle() {
    let mut graph = IBCChannelGraph::new();
    graph.add_channel("a", "channel-ab", "b", "channel-ba", true, "ACTIVE".into());
    graph.add_channel("b", "channel-bc", "c", "channel-cb", true, "ACTIVE".into());
    graph.add_channel("a", "channel-ac", "c", "channel-ca", true, "ACTIVE".into());

    let mut assets = HashMap::new();
    assets.insert(
        "a".to_string(),
        vec![json!({"symbol": "TOKEN", "base": "utoken"})],
    );

    let table = IBCAssetRoutingTable::premine(&graph, &assets, 3);
    let preferred = table.lookup_ibc_denom("TOKEN", "c").expect("preferred");
    assert_eq!(preferred.hop_count, 1);
    assert!(preferred.preferred);
    assert!(
        !table
            .lookup_by_dest_chain("c")
            .iter()
            .filter(|r| r.hop_count > 1)
            .any(|r| r.preferred)
    );
}

#[test]
fn finalize_akash_terp_channel_side_roundtrip() {
    let raw = vec![json!({
        "chain_1": { "channel_id": "channel-6", "port_id": "transfer" },
        "chain_2": { "channel_id": "channel-115", "port_id": "transfer" },
        "ordering": 1,
        "version": "ics20-1",
        "tags": { "status": "ACTIVE" }
    })];
    // akash < terp → chain_1_name = akash
    let final_chs = finalize_channels_for_ibc_entry(&raw, "akash", "akash", "Active");
    assert_eq!(final_chs[0]["chain_1"]["channel_id"], "channel-115");
    assert_eq!(final_chs[0]["chain_2"]["channel_id"], "channel-6");
    assert_eq!(final_chs[0]["ordering"], "unordered");
    assert_eq!(final_chs[0]["tags"]["preferred"], true);

    let ibc_entry = json!({
        "$schema": "../ibc_data.schema.json",
        "chain_1": {
            "chain_name": "akash",
            "chain_id": "akashnet-2",
            "client_id": "c1",
            "connection_id": "conn1"
        },
        "chain_2": {
            "chain_name": "terp",
            "chain_id": "morocco-1",
            "client_id": "c2",
            "connection_id": "conn2"
        },
        "channels": final_chs
    });
    let mut map_obj = serde_json::Map::new();
    map_obj.insert("akash".into(), ibc_entry);
    let map = build_channel_to_chain_map(&serde_json::Value::Object(map_obj));
    let info = map.get("akash").expect("akash map entry");
    assert_eq!(info.terp_channel_id, "channel-6");
    assert_eq!(info.counterparty_channel_id, "channel-115");
}

#[test]
fn check_invariants_flags_hop_count_lie() {
    let mut world = PredictedWorld::from_inputs(json!({}), &HashMap::new(), 1);
    world.routes.routes.insert(
        "c".into(),
        vec![terp_scripts::ibc::IBCAssetRoute {
            symbol: "X".into(),
            origin_chain: "a".into(),
            origin_denom: "ux".into(),
            dest_chain: "c".into(),
            dest_denom: compute_ibc_denom_hash("transfer/channel-0/transfer/channel-1/ux"),
            trace_path: "transfer/channel-0/transfer/channel-1/ux".into(),
            route: vec![],
            hop_count: 1,
            preferred: false,
        }],
    );
    let report = check_invariants(&world);
    assert!(report.has_errors());
    assert!(report.errors().any(|e| e.code == "hop_count"));
}

/// The cw-orch state file is what app code reads. Every one-hop denom in it
/// has to be the SHA-256 of its trace, and no recorded client may still say Active.
#[test]
fn state_json_one_hop_denoms_match_hash() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../public/state.json");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let state: serde_json::Value = serde_json::from_str(&raw).expect("state.json");
    let morocco = &state["morocco-1"];
    let denoms = &morocco["ibc_denoms"];
    assert!(denoms["on_terp"].as_array().unwrap().len() >= 12);
    for side in ["on_terp", "terp_on_counterparty"] {
        for row in denoms[side].as_array().expect(side) {
            let trace = row["trace"].as_str().unwrap();
            let denom = row["denom"].as_str().unwrap();
            assert_eq!(compute_ibc_denom_hash(trace), denom, "{trace}");
            assert_eq!(hop_count_from_trace_path(trace), 1, "{trace}");
            if let Some(status) = row.get("client_status").and_then(|v| v.as_str()) {
                assert_eq!(status, "Expired", "{trace}");
            }
        }
    }
    let hermes: Vec<_> = denoms["on_terp"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["hermes_allow"] == true)
        .map(|row| row["trace"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        hermes,
        vec![
            "transfer/channel-9/uakt",
            "transfer/channel-11/uosmo",
            "transfer/channel-10/uatone",
            "transfer/channel-0/ujuno",
            "transfer/channel-7/uscrt",
            "transfer/channel-8/uscrt",
        ]
    );
    let temp = morocco["temp_mint_denoms"]["assets"]
        .as_array()
        .expect("temp_mint_denoms");
    let symbols: Vec<_> = temp
        .iter()
        .map(|row| row["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(symbols, vec!["AKT", "OSMO", "UM", "ETH", "ZEC", "BTC"]);
    for row in temp {
        let trace = row["trace"].as_str().unwrap();
        assert_eq!(
            compute_ibc_denom_hash(trace),
            row["denom"].as_str().unwrap(),
            "{trace}"
        );
        assert_eq!(row["temporary"], true, "{trace}");
        assert_eq!(row["client_status"], "Expired", "{trace}");
        if let Some(canonical) = row["canonical_trace"].as_str() {
            assert_eq!(
                compute_ibc_denom_hash(canonical),
                row["canonical_denom"].as_str().unwrap(),
                "{canonical}"
            );
        }
    }
    let on_terp = denoms["on_terp"].as_array().unwrap();
    for kept in [
        "transfer/channel-9/uakt",
        "transfer/channel-6/uakt",
        "transfer/channel-11/uosmo",
        "transfer/channel-1/uosmo",
    ] {
        assert!(
            on_terp.iter().any(|row| row["trace"] == kept),
            "canonical one-hop row removed: {kept}"
        );
    }
    assert_eq!(temp[2]["hop_count"], 2);
    assert!(temp[2]["canonical_trace"].is_null());
    fn walk(v: &serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(status) = map.get("client_status").and_then(|s| s.as_str()) {
                    assert_ne!(status, "Active", "state.json still marks a client Active");
                }
                for child in map.values() {
                    walk(child);
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    walk(child);
                }
            }
            _ => {}
        }
    }
    walk(morocco);
}
