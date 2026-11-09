//! Golden fixture tests for IBC authenticity (offline, no Docker).

use terp_scripts::ibc::{
    FixtureBackend, ObserveBackend, PredictedWorld, check_invariants, compare_predict_observe,
    compute_ibc_denom_hash, golden_chain_assets, golden_root, hop_count_from_trace_path,
    load_audited_known_hashes, load_expected_lookup, load_golden_ibc_data, load_known_hashes,
    load_synthetic_known_hashes,
};

#[test]
fn known_hashes_match_compute_and_sections() {
    let all = load_known_hashes().expect("load known_hashes");
    let audited = load_audited_known_hashes().expect("audited");
    let synthetic = load_synthetic_known_hashes().expect("synthetic");

    assert!(
        audited.contains_key("transfer/channel-1/uakt"),
        "AKT osmosis must be audited_mainnet"
    );
    assert!(
        !audited.contains_key("transfer/channel-0/transfer/channel-1/uakt"),
        "multi-hop synthetic must not be audited"
    );
    assert!(synthetic.contains_key("transfer/channel-0/transfer/channel-1/uakt"));
    assert!(synthetic.contains_key("transfer/channel-0/factory/terp1abc/ta0"));

    for (path, expected) in &all {
        let got = compute_ibc_denom_hash(path);
        assert_eq!(&got, expected, "hash mismatch for path {path}");
        assert!(got.starts_with("ibc/"));
        assert_eq!(got.len(), 68);
        assert_eq!(
            hop_count_from_trace_path(path),
            path.matches("transfer/").count()
        );
    }
}

#[test]
fn golden_ibc_data_predict_invariants_clean() {
    let ibc_data = load_golden_ibc_data().expect("load golden ibc-data");
    let assets = golden_chain_assets();
    let world = PredictedWorld::from_inputs(ibc_data, &assets, 3);
    let report = check_invariants(&world);
    for e in report.errors() {
        eprintln!("ERROR [{}] {} ({:?})", e.code, e.message, e.path);
    }
    assert!(
        !report.has_errors(),
        "golden predicted world has hard invariant errors"
    );

    let akt = world
        .routes
        .lookup_ibc_denom("AKT", "osmosis")
        .expect("AKT preferred on osmosis");
    assert_eq!(akt.hop_count, 1);
    assert_eq!(akt.trace_path, "transfer/channel-1/uakt");
    assert_eq!(
        akt.dest_denom,
        "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4"
    );
    assert!(akt.preferred);

    let all_akt: Vec<_> = world
        .routes
        .lookup_by_dest_chain("osmosis")
        .into_iter()
        .filter(|r| r.symbol == "AKT")
        .collect();
    assert!(all_akt.iter().any(|r| r.hop_count > 1));
    assert!(
        all_akt
            .iter()
            .filter(|r| r.hop_count > 1)
            .all(|r| !r.preferred)
    );
}

#[test]
fn golden_expected_lookup_pins_preferred_routes() {
    let expected = load_expected_lookup().expect("expected_lookup");
    let world =
        PredictedWorld::from_inputs(load_golden_ibc_data().unwrap(), &golden_chain_assets(), 3);

    let osmo_akt = &expected["osmosis"]["AKT"];
    let path = osmo_akt["trace_path"].as_str().unwrap();
    let denom = osmo_akt["ibc_denom"].as_str().unwrap();
    let route = world
        .routes
        .lookup_ibc_denom("AKT", "osmosis")
        .expect("AKT on osmosis");
    assert_eq!(route.trace_path, path);
    assert_eq!(route.dest_denom, denom);
    assert_eq!(
        route.hop_count,
        osmo_akt["hop_count"].as_u64().unwrap() as usize
    );

    let terp_akt = &expected["terp"]["AKT"];
    let tpath = terp_akt["trace_path"].as_str().unwrap();
    let troute = world
        .routes
        .lookup_ibc_denom("AKT", "terp")
        .expect("AKT on terp");
    assert_eq!(troute.trace_path, tpath);
    assert_eq!(troute.hop_count, 1);
}

#[test]
fn golden_fixture_backend_uses_explicit_denom_traces() {
    let backend = FixtureBackend::load_from_golden_dir(golden_root()).expect("load golden backend");
    assert!(
        !backend.invert_known_hashes,
        "must not invert known_hashes by default"
    );

    let world =
        PredictedWorld::from_inputs(load_golden_ibc_data().unwrap(), &golden_chain_assets(), 3);
    let akt = world
        .routes
        .lookup_ibc_denom("AKT", "osmosis")
        .expect("akt");
    let obs = backend
        .observe_denom_trace("osmosis", &akt.dest_denom)
        .unwrap()
        .expect("explicit denom_traces.json observation for AKT");
    assert_eq!(obs.path, akt.trace_path);

    let report = compare_predict_observe(&world, &backend);
    assert!(
        !report.has_errors(),
        "compare should not error when explicit traces match: {:?}",
        report.items
    );
}
