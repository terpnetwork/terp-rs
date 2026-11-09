//! Latent Benders day record -> SvgCollectionSuite on cw-orch Mock.
//! `cargo test -p cw-infuser-scripts --test benders_e2e`
use cw_infuser_scripts::benders::{self, DayMintParams};
use cw_infuser_scripts::collection::SvgCollectionSuite;
use cw_orch::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};

const TEMPLATE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><text x="4" y="32">{{glyph}}</text><circle cx="32" cy="32" r="{{r}}"/></svg>"#;

/// Write a finalized-looking day record + packed.svp, hashed exactly like daily.py finalize.
fn fixture(dir: &std::path::Path) -> (String, String) {
    let vars = vec![
        cw_svg::VariableDef {
            name: "glyph".into(),
            kind: cw_svg::VariableKind::Options(vec!["Kara".into(), "Gippy".into()]),
        },
        cw_svg::VariableDef {
            name: "r".into(),
            kind: cw_svg::VariableKind::Range {
                min: "4".into(),
                max: "28".into(),
                precision: 0,
            },
        },
    ];
    let packed = cw721_svg::pack_svg(TEMPLATE, &vars);
    std::fs::write(dir.join("packed.svp"), packed.as_slice()).unwrap();
    let packed_sha = hex::encode(Sha256::digest(packed.as_slice()));
    let mut rec = json!({
        "schema": "symbient-day-record/v2", "persona": "latent-benders", "day": "2026-10-22",
        "day_index": 3, "prev_record_sha256": "0".repeat(64), "day_seed": "ab".repeat(32),
        "picks": [{"slug": "x", "name": "X — ünïcode", "url": "https://example.org/x", "classes": [], "acute_toxicity": "unknown", "effects": []}],
        "memory": [], "interactions": [], "model": {"name": "fixture", "temperature": 0.8},
        "frames": [{"seed": 1, "raw_sha256": "cd".repeat(32)}], "disclaimer": "",
        "packed_sha256": packed_sha, "packed_bytes": packed.len(),
    });
    let rs = benders::record_sha256(&rec).unwrap();
    rec["record_sha256"] = json!(rs);
    std::fs::write(
        dir.join("day-record.json"),
        serde_json::to_string_pretty(&rec).unwrap(),
    )
    .unwrap();
    (rs, packed_sha)
}

fn tmp() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "benders-e2e-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn day_record_to_collection_on_mock() {
    let dir = tmp();
    let (record_sha, packed_sha) = fixture(&dir);
    let v = benders::load(&dir.join("day-record.json"), None).unwrap();
    assert_eq!(v.record_sha256, record_sha);

    let chain = MockBech32::new("terp");
    let creator = chain.addr_make("kara-and-gippy");
    let chain = chain.call_as(&creator);
    let cfg = benders::day_config(&v, &DayMintParams::defaults(creator.clone())).unwrap();
    assert_eq!(
        (cfg.total, cfg.price_tiers.len(), cfg.mint_end_after_secs),
        (10_000, 0, Some(86_400))
    );
    // The contract stores mint_end_time at whole-second precision.
    let end = cosmwasm_std::Timestamp::from_seconds(
        chain
            .block_info()
            .unwrap()
            .time
            .plus_seconds(86_400)
            .seconds(),
    );

    let mut suite = SvgCollectionSuite::new(chain.clone());
    let addr = suite.instantiate(&cfg).unwrap();
    let st = suite.query(&addr).unwrap();
    assert_eq!(st.seed_hex, record_sha);
    assert_eq!(st.packed_sha256, packed_sha);
    assert_eq!(st.mint_end_time, Some(end));
    assert!(st.name.starts_with("Latent Benders: A Study"));

    // open mint: anyone mints for free inside the window
    assert!(suite.open_mint(&addr, false).unwrap());
    let minter = chain.addr_make("collector");
    SvgCollectionSuite::new(chain.call_as(&minter))
        .mint(&addr, 1, &[])
        .unwrap();
    assert_eq!(suite.query(&addr).unwrap().minted, 1);

    // past mint_end_time the contract refuses
    let now = chain.block_info().unwrap().time.seconds();
    chain.wait_seconds(end.seconds() - now + 1).unwrap();
    assert!(!suite.open_mint(&addr, false).unwrap());
    let err = SvgCollectionSuite::new(chain.call_as(&minter))
        .mint(&addr, 1, &[])
        .unwrap_err();
    assert!(format!("{err:#}").to_lowercase().contains("end"), "{err:#}");
}

#[test]
fn tampered_record_or_packed_is_rejected() {
    let dir = tmp();
    fixture(&dir);
    let p = dir.join("day-record.json");
    let mut rec: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    rec["day_index"] = json!(4);
    std::fs::write(&p, rec.to_string()).unwrap();
    assert!(format!("{:#}", benders::load(&p, None).unwrap_err()).contains("record_sha256"));

    let dir = tmp();
    fixture(&dir);
    std::fs::write(dir.join("packed.svp"), b"SVP1junk").unwrap();
    assert!(
        format!(
            "{:#}",
            benders::load(&dir.join("day-record.json"), None).unwrap_err()
        )
        .contains("packed_sha256")
    );
}
