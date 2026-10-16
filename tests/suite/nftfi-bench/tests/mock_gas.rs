use std::path::PathBuf;

use nftfi_bench::{DaemonGate, daemon_gate};

#[test]
fn mock_gas_table() {
    if daemon_gate() == DaemonGate::NeedsFeature {
        panic!("NFTFI_BENCH_LOCAL=1 requires cargo test -p nftfi-bench --features local");
    }
    let table = nftfi_bench::run_mock().expect("mock flows");
    assert!(table.all_ok(), "gas rows:\n{}", table.markdown());
    let path = table.write_gas_md().expect("write GAS.md");
    let text = std::fs::read_to_string(&path).unwrap();
    for action in [
        "list",
        "bid",
        "buy",
        "accept",
        "cancel",
        "create",
        "buy_ticket",
        "draw",
        "claim",
        "offer",
        "repay",
        "default",
    ] {
        assert!(text.contains(action), "GAS.md missing {action}");
    }
    assert!(text.contains("read 1000"));
    assert!(text.contains("does not broadcast") || text.contains("No transaction sent"));
    for row in &table.rows {
        let gas = row.kv_gas.expect("kv gas");
        assert!(gas > 0, "{} {} kv gas", row.protocol, row.action);
    }
}

#[test]
fn wiring_files_name_the_four_projects() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("tests/suite/nftfi-bench"));

    let yaml = std::fs::read_to_string(root.join("tools/gen-tools/gen-tools.yaml")).unwrap();
    for name in ["marketplace", "reserve-auction", "raffle", "nft-loan"] {
        assert!(
            yaml.contains(&format!("name: {name}")),
            "gen-tools.yaml missing {name}"
        );
    }

    let justfile = std::fs::read_to_string(root.join("justfile")).unwrap();
    let gen_just = std::fs::read_to_string(root.join("scripts/just/gen.just")).unwrap();
    assert!(justfile.contains("nftfi-schema project:"));
    assert!(gen_just.contains("nftfi-schema project:"));
    assert!(justfile.contains("nftfi-schema-all:"));
    assert!(gen_just.contains("nftfi-schema-all:"));
}
