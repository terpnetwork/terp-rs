//! Tickets pay HTTP — local, no chain, no mainnet.

use hash_market::onboard_authz::itl_ingress::cashapp_on_tickets;
use hash_market::onboard_authz::itl_pay::{health_json, instruct_json, quote_json, try_handle};
use serde_json::Value;

#[test]
fn itl_pay_quote_is_tickets_catalog() {
    assert!(!cashapp_on_tickets());
    let q = quote_json();
    assert_eq!(q["ok"], true);
    assert_eq!(q["cashapp_on_tickets"], false);
    assert_eq!(q["instruct_commits_paid"], false);
    assert_eq!(q["browser_writes_paid"], false);
    let cat: Vec<&str> = q["catalog"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert_eq!(cat, vec!["zec", "eth", "base", "penumbra"]);
    assert_eq!(q["rails"][0]["id"], "zec");
    assert_eq!(q["rails"][1]["chain_id"], 11155111);
    assert_eq!(q["rails"][2]["chain_id"], 84532);
    assert_eq!(q["rails"][3]["live"], false);
    assert_eq!(q["rails"][3]["coming"], true);
}

#[test]
fn itl_pay_cashapp_refused_and_penumbra_coming() {
    let c = instruct_json("cashapp", "cred");
    assert_eq!(c["error"], "cashapp_door_only");
    let p = instruct_json("penumbra", "");
    assert_eq!(p["ok"], false);
    assert_eq!(p["error"], "penumbra_coming");
    assert_eq!(p["paid"], false);
}

#[test]
fn itl_pay_http_surface() {
    let h = try_handle("GET", "/itl/pay/health", "").expect("health");
    assert_eq!(h.status, 200);
    assert!(h.body.contains("cashapp_on_tickets"));
    assert!(h.body.contains("penumbra_coming"));
    assert!(h.body.contains("\"anvil_refused\":true"));
    let q = try_handle("GET", "/itl/pay/quote", "").expect("quote");
    assert_eq!(q.status, 200);
    let i = try_handle("GET", "/itl/pay/instruct?rail=eth&recipient=terp1q", "").unwrap();
    assert!(i.body.contains("dregg_not_ready") || i.body.contains("need_binding"));
    assert!(try_handle("GET", "/nope", "").is_none());
    let _ = health_json();
}

#[test]
fn browser_forged_paid_flag_does_not_unlock_ticket() {
    let forged = try_handle(
        "POST",
        "/itl/pay/watch",
        r#"{"paid":true,"binding":"e2e-forge","rail":"zec"}"#,
    )
    .unwrap();
    assert_eq!(forged.status, 400);
    let v: Value = serde_json::from_str(&forged.body).unwrap();
    assert_eq!(v["error"], "browser_paid");
    assert_eq!(v["paid"], false);

    let post = try_handle(
        "POST",
        "/itl/pay/ticket?binding=e2e-forge",
        r#"{"paid":true}"#,
    )
    .unwrap();
    assert_eq!(post.status, 400);
    let p: Value = serde_json::from_str(&post.body).unwrap();
    assert_eq!(p["error"], "browser_paid");

    let read = try_handle("GET", "/itl/pay/ticket?binding=e2e-forge", "").unwrap();
    let r: Value = serde_json::from_str(&read.body).unwrap();
    assert_eq!(r["paid"], false, "forged paid must not unlock");
}

#[test]
fn server_watch_fact_is_the_only_paid_writer() {
    let body = r#"{
        "rail":"eth",
        "record_id":"eth_watch_r2_e2e_01",
        "dest":"0x00000000000000000000000000000000000000aa",
        "binding":"terp1r2e2eeth0000000000000000000000000000"
    }"#;
    let w = try_handle("POST", "/itl/pay/watch", body).unwrap();
    assert_eq!(w.status, 200);
    let v: Value = serde_json::from_str(&w.body).unwrap();
    assert_eq!(v["paid"], true);

    let read = try_handle(
        "GET",
        "/itl/pay/ticket?binding=terp1r2e2eeth0000000000000000000000000000",
        "",
    )
    .unwrap();
    let r: Value = serde_json::from_str(&read.body).unwrap();
    assert_eq!(r["paid"], true);
    assert_eq!(r["rail"], "eth");
    assert!(r.get("ticket_secret").is_none());

    let poll = try_handle(
        "GET",
        "/itl/pay/poll?rail=eth&dest=0x00000000000000000000000000000000000000aa&record=eth_watch_r2_e2e_01",
        "",
    )
    .unwrap();
    let p: Value = serde_json::from_str(&poll.body).unwrap();
    assert_eq!(p["paid"], true);
}
