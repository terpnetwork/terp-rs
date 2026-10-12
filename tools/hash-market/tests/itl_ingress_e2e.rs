//! Ingress plane — same crate as combo / itl_cashapp_e2e.
//! Local: no chain, no Docker, no Square, no second ETH/BTC/ZEC stack.
//!
//!   cargo test --manifest-path tools/onboard-authz/Cargo.toml --test itl_ingress_e2e

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Mutex;
use hash_market::onboard_authz::itl_cashapp::{
    nullifier_hex, parse_eml_wasix, synthetic_received_eml, synthetic_sent_eml, wasix_parse_json,
    LIST_CENTS, SKU,
};
use hash_market::onboard_authz::itl_ingress::{
    browser_paid_refused, cashapp_chain_id, cashapp_local_prod_cmd, dest_ok, dregg_endpoint_ok,
    dregg_instruct_commits_paid, dregg_spawn_cmd, evidence_may_commit, ict_mock_refused,
    is_mock_bech32, is_second_stack_spawn, parse_inject_eml, public_json_is_clean, DreggWatch,
    EvidenceKind, IngressEngine, Rail, CASHAPP_CHAIN_ID, EVENT_ID, SVG_MINT_FEATURES,
    SVG_MINT_LOCAL_PROD,
};
use hash_market::onboard_authz::poseidon::{door_nullifier, ticket_leaf, PoseidonTree};

#[test]
fn itl_ingress_gap0_wasix_parse_payment_sent_reject() {
    let eml = synthetic_received_eml("cmt_7F3K2", 2500, "D-VR992KMXL");
    let p = parse_eml_wasix(&eml).expect("wasix parse");
    assert_eq!(p.pay_ref, "cmt_7F3K2");
    assert_eq!(p.amount_cents, 2500);
    assert_eq!(p.tx_id, "D-VR992KMXL");
    let j = wasix_parse_json(&eml);
    assert_eq!(j["ok"], true);
    assert_eq!(j["proven"], false);
    assert_eq!(j["dkimDomain"], "square.com");
    assert_eq!(j["nullifier"], nullifier_hex("D-VR992KMXL"));
    let sent = synthetic_sent_eml("cmt_7F3K2", 2500, "D-VR992KMXL");
    assert_eq!(parse_eml_wasix(&sent).unwrap_err(), "not_payment_received");
    assert_eq!(wasix_parse_json(&sent)["error"], "not_payment_received");
}

#[test]
fn itl_ingress_cashapp_inject_to_poseidon_guest_root() {
    let mut e = IngressEngine::default();
    let empty = e.guest_root_hex();
    let o = e.create_cash_order("cred-guest");
    let eml = synthetic_received_eml(&o.pay_ref, LIST_CENTS, "D-AAAAAAAAA");
    let inj = parse_inject_eml(&eml);
    assert_eq!(inj["ok"], true);
    let issued = e.ingest_cashapp_eml(&eml).expect("paid");
    assert_eq!(issued.rail, Rail::CashApp);
    assert_eq!(issued.paid_n, 1);
    assert_eq!(issued.sku, SKU);
    assert_eq!(issued.event_id, EVENT_ID);
    assert_eq!(issued.payment_nullifier, nullifier_hex("D-AAAAAAAAA"));
    assert_eq!(
        issued.leaf,
        ticket_leaf(&issued.ticket_secret, SKU, EVENT_ID)
    );
    assert_eq!(
        issued.door_nullifier(),
        door_nullifier(&issued.ticket_secret, EVENT_ID)
    );
    assert_ne!(e.guest_root_hex(), empty);
    assert_eq!(e.guest_root_hex(), issued.guest_root_hex());
    assert!(public_json_is_clean(&e.public_sale_json()));
    let msg = e.update_guest_root_msg();
    assert!(public_json_is_clean(&msg));
    assert!(msg.get("txId").is_none());
}

#[test]
fn itl_ingress_replay_txid_and_payment_sent_refused() {
    let mut e = IngressEngine::default();
    let o = e.create_cash_order("g1");
    let eml = synthetic_received_eml(&o.pay_ref, LIST_CENTS, "D-BBBBBBBBB");
    e.ingest_cashapp_eml(&eml).expect("first");
    assert_eq!(e.ingest_cashapp_eml(&eml).unwrap_err(), "nullifier_used");
    let sent = synthetic_sent_eml(&o.pay_ref, LIST_CENTS, "D-CCCCCCCCC");
    assert_eq!(
        e.ingest_cashapp_eml(&sent).unwrap_err(),
        "not_payment_received"
    );
    let o2 = e.create_cash_order("g2");
    let stray = synthetic_received_eml("cmt_NOTOURS", LIST_CENTS, "D-DDDDDDDDD");
    assert_eq!(e.ingest_cashapp_eml(&stray).unwrap_err(), "unknown_payRef");
    let low = synthetic_received_eml(&o2.pay_ref, 100, "D-EEEEEEEEE");
    assert_eq!(e.ingest_cashapp_eml(&low).unwrap_err(), "amount_too_low");
    assert_eq!(e.paid_count(), 1);
}

#[test]
fn itl_ingress_rush_64_unique_commitments_and_nullifiers() {
    let mut e = IngressEngine::default();
    let mut roots = HashSet::new();
    roots.insert(e.guest_root_hex());
    let mut nfs = HashSet::new();
    let mut leaves = HashSet::new();
    for i in 0..64 {
        let o = e.create_cash_order(&format!("g{i}"));
        let tx = format!("D-{i:09}");
        let eml = synthetic_received_eml(&o.pay_ref, LIST_CENTS, &tx);
        let issued = e
            .ingest_cashapp_eml(&eml)
            .unwrap_or_else(|err| panic!("{i} {tx} {err}"));
        assert!(nfs.insert(issued.payment_nullifier.clone()), "dup nf {i}");
        assert!(leaves.insert(issued.leaf_hex()), "dup leaf {i}");
        assert!(
            roots.insert(issued.guest_root_hex()),
            "root did not bump {i}"
        );
    }
    assert_eq!(e.paid_count(), 64);
    assert_eq!(e.unique_payment_nullifiers(), 64);
    assert_eq!(nfs.len(), 64);
    assert_eq!(leaves.len(), 64);
    assert_eq!(roots.len(), 65);
    assert!(public_json_is_clean(&e.public_sale_json()));
    assert_eq!(e.public_sale_json()["paid_n"], 64);
}

#[test]
fn itl_ingress_concurrent_pays_unique_nullifiers() {
    let e = Mutex::new(IngressEngine::default());
    {
        let mut g = e.lock().unwrap();
        for i in 0..16 {
            g.create_cash_order(&format!("c{i}"));
        }
    }
    std::thread::scope(|s| {
        for i in 0..16 {
            let e = &e;
            s.spawn(move || {
                let pay = format!("cmt_{:05}", i + 1);
                let tx = format!("D-{:09}", 1000 + i);
                let eml = synthetic_received_eml(&pay, LIST_CENTS, &tx);
                e.lock()
                    .unwrap()
                    .ingest_cashapp_eml(&eml)
                    .unwrap_or_else(|err| panic!("{i} {err}"));
            });
        }
    });
    let g = e.lock().unwrap();
    assert_eq!(g.paid_count(), 16);
    assert_eq!(g.unique_payment_nullifiers(), 16);
}

#[test]
fn itl_ingress_poseidon_leaf_is_not_sha256() {
    let secret = [7u8; 32];
    let leaf = ticket_leaf(&secret, SKU, EVENT_ID);
    let mut h = Sha256::new();
    h.update(secret);
    h.update(SKU.as_bytes());
    h.update(EVENT_ID.as_bytes());
    assert_ne!(leaf.as_slice(), h.finalize().as_slice());
    let nf = door_nullifier(&secret, EVENT_ID);
    assert_ne!(nf, leaf);
    let mut t = PoseidonTree::default();
    let before = t.root_hex();
    t.append_bytes(leaf).unwrap();
    assert_ne!(before, t.root_hex());
}

#[test]
fn itl_ingress_dregg_watch_commits_instruct_does_not() {
    assert!(!dregg_instruct_commits_paid());
    assert!(evidence_may_commit(EvidenceKind::DreggWatch));
    assert!(evidence_may_commit(EvidenceKind::CashappReceipt));
    assert_eq!(browser_paid_refused(), "browser_paid");

    dest_ok(Rail::Zec, "zcash:u1lab?amount=0.068&memo=YWJj").unwrap();
    assert_eq!(
        dest_ok(Rail::Zec, "zcash://u1lab").unwrap_err(),
        "bad_zec_uri"
    );
    dest_ok(Rail::Btc, "bcrt1qittestlocalprod00000000000000000000").unwrap();
    dest_ok(Rail::Eth, "0x0000000000000000000000000000000000000001").unwrap();
    dest_ok(Rail::Base, "0xabcdefffffffffffffffffffffffffffffff0001").unwrap();
    assert_eq!(
        dest_ok(Rail::Penumbra, "penumbra1xyz").unwrap_err(),
        "penumbra_skipped"
    );

    let mut e = IngressEngine::default();
    let empty = e.guest_root_hex();
    let watch = DreggWatch {
        rail: Rail::Zec,
        record_id: "memo_commit_lab_01".into(),
        dest: "zcash:u1lab?amount=0.068&memo=YWJj".into(),
    };
    let issued = e.ingest_dregg(&watch).expect("dregg watch");
    assert_eq!(issued.rail, Rail::Zec);
    assert_eq!(issued.paid_n, 1);
    assert_ne!(e.guest_root_hex(), empty);
    assert!(public_json_is_clean(&e.public_sale_json()));
    assert_eq!(e.ingest_dregg(&watch).unwrap_err(), "nullifier_used");

    let mock = DreggWatch {
        rail: Rail::Zec,
        record_id: "x".into(),
        dest: "mockbech32:paid".into(),
    };
    assert_eq!(e.ingest_dregg(&mock).unwrap_err(), "mock_bech32");
    assert!(is_mock_bech32("MockBech32"));

    let fake = DreggWatch {
        rail: Rail::Eth,
        record_id: "invented-rpc-tx".into(),
        dest: "0x0000000000000000000000000000000000000001".into(),
    };
    assert_eq!(e.ingest_dregg(&fake).unwrap_err(), "invented_rpc");

    let pen = DreggWatch {
        rail: Rail::Penumbra,
        record_id: "n".into(),
        dest: "penumbra1".into(),
    };
    assert_eq!(e.ingest_dregg(&pen).unwrap_err(), "penumbra_skipped");
}

#[test]
fn itl_ingress_dregg_reuses_svg_mint_local_prod_no_second_stack() {
    assert_eq!(dregg_spawn_cmd(), SVG_MINT_LOCAL_PROD);
    assert!(SVG_MINT_LOCAL_PROD.contains("svg_mint_local_prod"));
    assert!(SVG_MINT_LOCAL_PROD.contains(SVG_MINT_FEATURES));
    assert!(SVG_MINT_FEATURES.contains("zakura"));
    assert!(SVG_MINT_FEATURES.contains("ethereum"));
    assert!(SVG_MINT_FEATURES.contains("bitcoin"));
    assert!(!is_second_stack_spawn(SVG_MINT_LOCAL_PROD));
    assert!(is_second_stack_spawn(
        "docker compose up bitcoind zcashd geth"
    ));
    assert!(is_second_stack_spawn("anvil --fork-url http://fake-rpc"));
    assert!(dregg_endpoint_ok("http://127.0.0.1:8421"));
    assert!(!dregg_endpoint_ok("http://fake-rpc.example"));
    assert!(cashapp_local_prod_cmd().contains("itl_cashapp_local_prod"));
    assert!(ict_mock_refused(Some("1")));
    assert!(!ict_mock_refused(None));
}

#[test]
fn itl_ingress_amazonses_dkim_alone_rejected() {
    let eml = synthetic_received_eml("cmt_7F3K2", 2500, "D-VR992KMXL")
        .replace("d=square.com", "d=amazonses.com");
    assert_eq!(parse_eml_wasix(&eml).unwrap_err(), "dkim_domain");
}

/// Live: reuse existing svg_mint_local_prod. Do not spawn a second stack here.
#[test]
#[ignore = "needs ict spawn; reuse svg_mint_local_prod — do not duplicate ETH/BTC/ZEC"]
fn live_dregg_svg_mint_local_prod() {
    assert_eq!(dregg_spawn_cmd(), SVG_MINT_LOCAL_PROD);
    assert!(
        std::env::var("ICT_MOCK").ok().as_deref() != Some("1"),
        "ICT_MOCK=1 refused"
    );
}

/// Live: ict-rs Cash App Docker Terp. Mail inject is the only mock.
#[test]
#[ignore = "needs ICT_LCD or ict spawn (itl_cashapp_local_prod)"]
fn live_cashapp_local_prod_refuses_ict_mock() {
    if ict_mock_refused(std::env::var("ICT_MOCK").ok().as_deref()) {
        panic!("ICT_MOCK=1 refused");
    }
    assert!(cashapp_local_prod_cmd().contains("itl_cashapp_local_prod"));
}

#[test]
fn itl_ingress_wasix_json_issues_poseidon_commitment() {
    let mut e = IngressEngine::default();
    let o = e.create_cash_order("cred-wasix");
    let eml = synthetic_received_eml(&o.pay_ref, LIST_CENTS, "D-WASIX0001");
    let v = parse_inject_eml(&eml);
    assert_eq!(v["ok"], true);
    assert_eq!(v["proven"], false);
    let issued = e.ingest_wasix_json(&v).expect("wasix json");
    assert_eq!(issued.payment_nullifier, nullifier_hex("D-WASIX0001"));
    assert_eq!(issued.paid_n, 1);
    assert!(public_json_is_clean(&e.public_sale_json()));
    let sale = e.public_sale_json().to_string();
    assert!(!sale.contains("D-WASIX0001"));
    assert!(!sale.contains("txId"));
    let groth = serde_json::json!({
        "ok": true,
        "payRef": o.pay_ref,
        "amountCents": LIST_CENTS,
        "txId": "D-WASIX0002",
        "nullifier": nullifier_hex("D-WASIX0002"),
        "proven": true
    });
    assert_eq!(e.ingest_wasix_json(&groth).unwrap_err(), "zkemail_groth16_out");
}

#[test]
fn itl_ingress_dregg_btc_eth_base_reuse_svg_mint_not_second_stack() {
    assert!(!is_second_stack_spawn(SVG_MINT_LOCAL_PROD));
    let mut e = IngressEngine::default();
    let watches = [
        DreggWatch {
            rail: Rail::Btc,
            record_id: "btc_watch_01".into(),
            dest: "bcrt1qittestlocalprod00000000000000000000".into(),
        },
        DreggWatch {
            rail: Rail::Eth,
            record_id: "eth_watch_01".into(),
            dest: "0x0000000000000000000000000000000000000001".into(),
        },
        DreggWatch {
            rail: Rail::Base,
            record_id: "base_watch_01".into(),
            dest: "0xabcdefffffffffffffffffffffffffffffff0001".into(),
        },
    ];
    for w in &watches {
        e.ingest_dregg(w).unwrap_or_else(|err| panic!("{:?} {err}", w.rail));
    }
    assert_eq!(e.paid_count(), 3);
    assert_eq!(e.unique_payment_nullifiers(), 3);
    assert!(public_json_is_clean(&e.public_sale_json()));
    assert_eq!(cashapp_chain_id(), CASHAPP_CHAIN_ID);
    assert_eq!(CASHAPP_CHAIN_ID, "itl-cashapp-1");
}

