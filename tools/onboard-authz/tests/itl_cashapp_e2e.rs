//! Cash App zkEmail dimension — same crate as combo / itl_auth_e2e.
//! Local: no chain, no Docker, no Square. Live ingest is ignored.
//!
//!   cargo test --manifest-path tools/onboard-authz/Cargo.toml --test itl_cashapp_e2e

use terp_onboard_authz::itl_cashapp::{
    parse_received_text, receipt_json, synthetic_received_html, Engine, CHECK_IN_GUEST,
    LIST_CENTS, SKU,
};

#[test]
fn itl_cashapp_gap0_fields_and_payment_sent_reject() {
    let html = synthetic_received_html("cmt_7F3K2", 2500, "D-VR992KMXL");
    let p = parse_received_text(&html, "Payment received").expect("parse");
    assert_eq!(p.pay_ref, "cmt_7F3K2");
    assert_eq!(p.amount_cents, 2500);
    assert_eq!(p.tx_id, "D-VR992KMXL");
    assert!(parse_received_text(&html, "Payment sent").is_err());
}

#[test]
fn itl_cashapp_payref_bind_nullifier_replay_unknown_note() {
    let mut e = Engine::default();
    let o = e.create_order("cred-guest");
    let html = synthetic_received_html(&o.pay_ref, LIST_CENTS, "D-AAAAAAAAA");
    let paid = e.ingest_text(&html, "Payment received").expect("paid");
    assert_eq!(paid.status, "paid");
    assert_eq!(paid.buyer_binding, "cred-guest");
    let replay = e.ingest_text(&html, "Payment received");
    assert_eq!(replay.unwrap_err(), "nullifier_used");
    let stray = synthetic_received_html("cmt_NOTOURS", LIST_CENTS, "D-BBBBBBBBB");
    assert_eq!(
        e.ingest_text(&stray, "Payment received").unwrap_err(),
        "unknown_payRef"
    );
    let o2 = e.create_order("g2");
    let low = synthetic_received_html(&o2.pay_ref, 100, "D-CCCCCCCCC");
    assert_eq!(
        e.ingest_text(&low, "Payment received").unwrap_err(),
        "amount_too_low"
    );
}

#[test]
fn itl_cashapp_rush_64_unique_nullifiers() {
    let mut e = Engine::default();
    for i in 0..64 {
        let o = e.create_order(&format!("g{i}"));
        let tx = format!("D-{i:09}");
        let html = synthetic_received_html(&o.pay_ref, LIST_CENTS, &tx);
        e.ingest_text(&html, "Payment received")
            .unwrap_or_else(|err| panic!("{i} {tx} {err}"));
    }
    assert_eq!(e.paid_count(), 64);
}

#[test]
fn itl_cashapp_reservation_one_winner() {
    let mut e = Engine::default();
    let mut ok = 0;
    let mut taken = 0;
    for i in 0..40 {
        match e.book_slot("hydro", "2026-10-18T18:00:00Z", &format!("g{i}")) {
            Ok(()) => ok += 1,
            Err("slot_taken") => taken += 1,
            Err(x) => panic!("{x}"),
        }
    }
    assert_eq!(ok, 1);
    assert_eq!(taken, 39);
    assert!(e.book_slot("needle", "2026-10-18T18:00:00Z", "other").is_ok());
}

#[test]
fn itl_cashapp_receipt_json_is_tickets_plane_shape() {
    let j = receipt_json("cmt_ABCDE", 2500, "D-VR992KMXL");
    assert_eq!(j["payRef"], "cmt_ABCDE");
    assert_eq!(j["amountCents"], 2500);
    assert_eq!(j["txId"], "D-VR992KMXL");
    assert_eq!(j["proven"], false);
    assert_eq!(j["dkimDomain"], "square.com");
    assert!(j.get("cashtag").is_none());
    assert_eq!(SKU, "itl-speakeasy-oct18");
    assert_eq!(CHECK_IN_GUEST, "check_in_guest");
}

#[test]
#[ignore = "needs ICT_LCD or ITL_MOCK_CASHAPP (harness mock :3051)"]
fn live_mock_cashapp_receipt_http() {
    let base = std::env::var("ITL_MOCK_CASHAPP")
        .unwrap_or_else(|_| "http://127.0.0.1:3051".into());
    let pay = "cmt_HARNESS";
    let inj = curl(
        &format!("{base}/inject"),
        Some(&format!(r#"{{"payRef":"{pay}","amountCents":2500}}"#)),
    );
    assert!(inj.contains(&pay) || inj.contains("txId"), "{inj}");
    let rec = curl(&format!("{base}/cashapp/receipts/{pay}.json"), None);
    assert!(rec.contains("\"payRef\""), "{rec}");
    assert!(rec.contains("D-"), "{rec}");
    assert!(!rec.to_lowercase().contains("cashtag of guest"));
}

fn curl(url: &str, body: Option<&str>) -> String {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sS", "--max-time", "8"]);
    if let Some(b) = body {
        cmd.args(["-X", "POST", "-H", "content-type: application/json", "-d", b]);
    }
    cmd.arg(url);
    let out = cmd.output().expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
