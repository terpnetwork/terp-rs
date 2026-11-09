//! Loopback ITL tickets pay surface. Shares the frost inbox listener
//! (`127.0.0.1:19091`). Not a second proxy. Not Cash App (door only).
//! Mainnet origins and chain ids are refused. Dest on screen is not paid.
//! Paid is a server watcher fact. The browser may only read it.

use super::http::HttpResponse;
use super::itl_ingress::{
    DreggWatch, IngressEngine, Rail, dregg_endpoint_ok, dregg_instruct_commits_paid,
    lab_evm_chain_id, mainnet_chain_id_refused, tickets_catalog,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn json_resp(status: u16, v: Value) -> HttpResponse {
    HttpResponse {
        status,
        body: v.to_string(),
    }
}

fn split_path(path_and_query: &str) -> (&str, &str) {
    match path_and_query.split_once('?') {
        Some((p, q)) => (p, q),
        None => (path_and_query, ""),
    }
}

fn qparam(query: &str, key: &str) -> Option<String> {
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        let k = kv.next().unwrap_or("");
        if k == key {
            let v = kv.next().unwrap_or("");
            return Some(url_decode(v));
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v as char);
            i += 3;
            continue;
        }
        if b[i] == b'+' {
            out.push(' ');
        } else {
            out.push(b[i] as char);
        }
        i += 1;
    }
    out
}

fn rail_of(s: &str) -> Option<Rail> {
    match s {
        "zec" => Some(Rail::Zec),
        "eth" => Some(Rail::Eth),
        "base" => Some(Rail::Base),
        "penumbra" => Some(Rail::Penumbra),
        "btc" => Some(Rail::Btc),
        "cashapp" => Some(Rail::CashApp),
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct TicketView {
    rail: String,
    record_id: String,
    dest: String,
    binding: String,
    guest_root: String,
    paid_n: usize,
}

struct PayStore {
    engine: IngressEngine,
    facts: HashMap<String, TicketView>,
}

fn store() -> &'static Mutex<PayStore> {
    static S: OnceLock<Mutex<PayStore>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(PayStore {
            engine: IngressEngine::default(),
            facts: HashMap::new(),
        })
    })
}

fn index_keys(watch: &DreggWatch, binding: Option<&str>) -> Vec<String> {
    let mut keys = vec![
        watch.dest.trim().to_string(),
        watch.record_id.trim().to_string(),
        format!("{}:{}", watch.rail.as_str(), watch.record_id.trim()),
    ];
    if let Some(b) = binding {
        let b = b.trim();
        if !b.is_empty() {
            keys.push(b.to_string());
        }
    }
    keys.into_iter().filter(|k| !k.is_empty()).collect()
}

fn remember(
    store: &mut PayStore,
    watch: &DreggWatch,
    binding: Option<&str>,
    guest_root: String,
    paid_n: usize,
) {
    let view = TicketView {
        rail: watch.rail.as_str().to_string(),
        record_id: watch.record_id.clone(),
        dest: watch.dest.clone(),
        binding: binding.unwrap_or("").to_string(),
        guest_root,
        paid_n,
    };
    for k in index_keys(watch, binding) {
        store.facts.insert(k, view.clone());
    }
}

fn lookup(store: &PayStore, keys: &[String]) -> Option<TicketView> {
    for k in keys {
        let t = k.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(v) = store.facts.get(t) {
            return Some(v.clone());
        }
    }
    None
}

fn fact_json(f: &TicketView) -> Value {
    json!({
        "ok": true,
        "paid": true,
        "status": "paid",
        "rail": f.rail,
        "record_id": f.record_id,
        "dest": f.dest,
        "binding": f.binding,
        "guest_root": f.guest_root,
        "paid_n": f.paid_n,
        "label": "Watcher fact on the server. Ticket paid."
    })
}

fn unpaid_json(label: &str) -> Value {
    json!({
        "ok": true,
        "paid": false,
        "status": "waiting",
        "label": label
    })
}

fn browser_paid_json() -> Value {
    json!({
        "ok": false,
        "error": "browser_paid",
        "paid": false,
        "status": "waiting",
        "label": "A browser paid flag does not unlock a ticket. Paid is a server watcher fact."
    })
}

fn claims_browser_paid(body: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        let l = body.to_ascii_lowercase();
        return l.contains("\"paid\":true") || l.contains("paid=true");
    };
    match v.get("paid") {
        Some(Value::Bool(true)) => true,
        Some(Value::String(s)) if s.eq_ignore_ascii_case("true") => true,
        Some(Value::Number(n)) if n.as_u64() == Some(1) => true,
        _ => false,
    }
}

fn watch_from_value(v: &Value) -> Result<(DreggWatch, Option<String>), &'static str> {
    let rail_s = v.get("rail").and_then(|x| x.as_str()).unwrap_or("");
    let record_id = v
        .get("record_id")
        .or_else(|| v.get("recordId"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let dest = v
        .get("dest")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let binding = v
        .get("binding")
        .or_else(|| v.get("recipient"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let rail = rail_of(rail_s).ok_or("unknown_rail")?;
    if dest.is_empty() || record_id.is_empty() {
        return Err("need_watch");
    }
    Ok((
        DreggWatch {
            rail,
            record_id,
            dest,
        },
        binding,
    ))
}

fn ingest_watch(watch: DreggWatch, binding: Option<String>) -> HttpResponse {
    let mut g = match store().lock() {
        Ok(g) => g,
        Err(_) => return json_resp(500, json!({"ok": false, "error": "pay_lock"})),
    };
    match g.engine.ingest_dregg(&watch) {
        Ok(issued) => {
            remember(
                &mut g,
                &watch,
                binding.as_deref(),
                issued.guest_root_hex(),
                issued.paid_n,
            );
            json_resp(
                200,
                json!({
                    "ok": true,
                    "paid": true,
                    "status": "paid",
                    "rail": watch.rail.as_str(),
                    "record_id": watch.record_id,
                    "guest_root": issued.guest_root_hex(),
                    "paid_n": issued.paid_n,
                    "label": "Watcher fact ingested. Ticket paid."
                }),
            )
        }
        Err("nullifier_used") => {
            if let Some(f) = lookup(&g, &index_keys(&watch, binding.as_deref())) {
                json_resp(200, fact_json(&f))
            } else {
                json_resp(
                    200,
                    json!({
                        "ok": true,
                        "paid": true,
                        "status": "paid",
                        "rail": watch.rail.as_str(),
                        "record_id": watch.record_id,
                        "label": "Watcher fact already on the server. Ticket paid."
                    }),
                )
            }
        }
        Err(e) => json_resp(
            400,
            json!({
                "ok": false,
                "error": e,
                "paid": false,
                "status": "waiting",
                "label": "Watcher fact refused. Ticket unpaid."
            }),
        ),
    }
}

fn handle_watch(body: &str) -> HttpResponse {
    let claimed = claims_browser_paid(body);
    let parsed = serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!({}));
    match watch_from_value(&parsed) {
        Err(_) if claimed => json_resp(400, browser_paid_json()),
        Err(e) => json_resp(
            400,
            json!({"ok": false, "error": e, "paid": false, "status": "waiting"}),
        ),
        Ok((watch, binding)) => ingest_watch(watch, binding),
    }
}

fn ticket_of(query: &str) -> HttpResponse {
    let keys = [
        qparam(query, "binding"),
        qparam(query, "recipient"),
        qparam(query, "dest"),
        qparam(query, "record"),
        qparam(query, "record_id"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let g = match store().lock() {
        Ok(g) => g,
        Err(_) => return json_resp(500, json!({"ok": false, "error": "pay_lock"})),
    };
    if let Some(f) = lookup(&g, &keys) {
        return json_resp(200, fact_json(&f));
    }
    json_resp(
        200,
        unpaid_json("No server watcher fact for this binding. Ticket unpaid."),
    )
}

fn dregg_phase_paid(phase: &str) -> bool {
    phase.contains("Deposited") || phase.contains("Entitled") || phase.contains("Settled")
}

/// Loopback DREGG only. Never follows a non-127.0.0.1 hop. No secrets.
fn dregg_status_phase(recipient: &str) -> Option<String> {
    if recipient.trim().is_empty() {
        return None;
    }
    if !dregg_endpoint_ok("http://127.0.0.1:8421") {
        return None;
    }
    let mut s =
        TcpStream::connect_timeout(&"127.0.0.1:8421".parse().ok()?, Duration::from_millis(400))
            .ok()?;
    s.set_read_timeout(Some(Duration::from_millis(400))).ok()?;
    s.set_write_timeout(Some(Duration::from_millis(400))).ok()?;
    let path = format!(
        "/v1/status?recipient={}",
        encoding_minimal(recipient.trim())
    );
    let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).ok()?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).ok()?;
    let body = buf.split("\r\n\r\n").nth(1)?;
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("phase").and_then(|x| x.as_str()).map(str::to_string)
}

fn encoding_minimal(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

fn poll_json(query: &str) -> Value {
    let rail_s = qparam(query, "rail").unwrap_or_default();
    let dest = qparam(query, "dest").unwrap_or_default();
    let recipient = qparam(query, "recipient")
        .or_else(|| qparam(query, "binding"))
        .unwrap_or_default();
    let record = qparam(query, "record")
        .or_else(|| qparam(query, "record_id"))
        .unwrap_or_default();

    if rail_of(&rail_s) == Some(Rail::Penumbra) || rail_s == "penumbra" {
        return json!({
            "ok": true,
            "paid": false,
            "status": "waiting",
            "rail": "penumbra",
            "coming": true,
            "label": "Shielded Penumbra is named and coming. narsil is a ZEC syndicate scanner, not an ITL Penumbra watcher. Not paid."
        });
    }
    if rail_of(&rail_s) == Some(Rail::CashApp) {
        return json!({
            "ok": false,
            "error": "cashapp_door_only",
            "paid": false,
            "status": "waiting"
        });
    }

    {
        let g = match store().lock() {
            Ok(g) => g,
            Err(_) => return json!({"ok": false, "error": "pay_lock", "paid": false}),
        };
        let keys = vec![
            dest.clone(),
            record.clone(),
            recipient.clone(),
            if !record.is_empty() {
                format!("{rail_s}:{record}")
            } else {
                String::new()
            },
        ];
        if let Some(f) = lookup(&g, &keys) {
            return fact_json(&f);
        }
    }

    if let Some(rail) = rail_of(&rail_s)
        && !dest.trim().is_empty()
        && !record.trim().is_empty()
        && let Some(phase) = dregg_status_phase(&recipient)
        && dregg_phase_paid(&phase)
    {
        let watch = DreggWatch {
            rail,
            record_id: record.clone(),
            dest: dest.clone(),
        };
        let resp = ingest_watch(watch, Some(recipient.clone()).filter(|s| !s.is_empty()));
        if let Ok(v) = serde_json::from_str::<Value>(&resp.body)
            && v.get("paid").and_then(|x| x.as_bool()) == Some(true)
        {
            return v;
        }
    }

    unpaid_json("Watcher has not marked paid. Dest on screen is not paid.")
}

pub fn quote_json() -> Value {
    json!({
        "ok": true,
        "catalog": tickets_catalog(),
        "cashapp_on_tickets": false,
        "instruct_commits_paid": dregg_instruct_commits_paid(),
        "network": "testnet_or_regtest_only",
        "browser_writes_paid": false,
        "rails": [
            {"id": "zec", "label": "Shielded ZEC", "discount": "20%", "live": true},
            {"id": "eth", "label": "Ether (obfuscated)", "discount": "20%", "live": true, "chain_id": 11155111},
            {"id": "base", "label": "Base (obfuscated)", "discount": "none", "live": true, "chain_id": 84532},
            {"id": "penumbra", "label": "Shielded Penumbra", "discount": "20%", "live": false, "coming": true, "watcher": false},
        ]
    })
}

pub fn health_json() -> Value {
    json!({
        "ok": true,
        "bind": "127.0.0.1",
        "cashapp_on_tickets": false,
        "catalog": tickets_catalog(),
        "dregg_loopback_ok": dregg_endpoint_ok("http://127.0.0.1:8421"),
        "penumbra_watcher": false,
        "penumbra_coming": true,
        "browser_writes_paid": false,
        "mainnet_eth_refused": mainnet_chain_id_refused(1),
        "mainnet_base_refused": mainnet_chain_id_refused(8453),
        "sepolia_ok": lab_evm_chain_id(11155111),
        "base_sepolia_ok": lab_evm_chain_id(84532),
        "anvil_refused": !lab_evm_chain_id(31337),
    })
}

/// Named, not selectable. narsil is not an ITL Penumbra note watcher.
pub fn penumbra_instruct() -> Value {
    json!({
        "ok": false,
        "rail": "penumbra",
        "error": "penumbra_coming",
        "paid": false,
        "coming": true,
        "label": "Shielded Penumbra is coming. narsil scans ZEC syndicate notes, not ITL Penumbra dests. No transfer from this page."
    })
}

pub fn instruct_json(rail: &str, recipient: &str) -> Value {
    let Some(r) = rail_of(rail) else {
        return json!({"ok": false, "error": "unknown_rail"});
    };
    if r == Rail::CashApp {
        return json!({
            "ok": false,
            "error": "cashapp_door_only",
            "label": "Cash App is door only. Not a tickets checkout."
        });
    }
    if r == Rail::Penumbra {
        return penumbra_instruct();
    }
    if r == Rail::Btc {
        return json!({
            "ok": false,
            "error": "not_on_tickets",
            "label": "Bitcoin is not on pre-arrival checkout this round."
        });
    }
    if recipient.trim().is_empty() {
        return json!({
            "ok": false,
            "error": "need_binding",
            "label": "Need a Terp address before DREGG can quote. No dest invented."
        });
    }
    json!({
        "ok": false,
        "rail": rail,
        "error": "dregg_not_ready",
        "label": "Loopback DREGG is not up. No dest invented. No transfer from this page.",
        "recipient_bound": true
    })
}

pub fn try_handle(method: &str, path_and_query: &str, body: &str) -> Option<HttpResponse> {
    let (path, query) = split_path(path_and_query);
    if !path.starts_with("/itl/pay") {
        return None;
    }
    if method == "OPTIONS" {
        return Some(json_resp(200, json!({})));
    }
    if method == "GET" && (path == "/itl/pay/health" || path == "/itl/pay") {
        return Some(json_resp(200, health_json()));
    }
    if method == "GET" && path == "/itl/pay/quote" {
        return Some(json_resp(200, quote_json()));
    }
    if method == "GET" && path == "/itl/pay/instruct" {
        let rail = qparam(query, "rail").unwrap_or_default();
        let recipient = qparam(query, "recipient").unwrap_or_default();
        return Some(json_resp(200, instruct_json(&rail, &recipient)));
    }
    if method == "GET" && path == "/itl/pay/poll" {
        return Some(json_resp(200, poll_json(query)));
    }
    if method == "GET" && path == "/itl/pay/ticket" {
        return Some(ticket_of(query));
    }
    if method == "POST" && path == "/itl/pay/ticket" {
        return Some(json_resp(400, browser_paid_json()));
    }
    if method == "POST" && path == "/itl/pay/watch" {
        return Some(handle_watch(body));
    }
    Some(json_resp(404, json!({"ok": false, "error": "not found"})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_excludes_cashapp() {
        let q = quote_json();
        assert_eq!(q["cashapp_on_tickets"], false);
        let cat = q["catalog"].as_array().unwrap();
        let ids: Vec<&str> = cat.iter().filter_map(|x| x.as_str()).collect();
        assert_eq!(ids, vec!["zec", "eth", "base", "penumbra"]);
        assert!(!ids.contains(&"cashapp"));
        assert_eq!(q["rails"][3]["live"], false);
        assert_eq!(q["rails"][3]["coming"], true);
        assert_eq!(q["browser_writes_paid"], false);
    }

    #[test]
    fn cashapp_instruct_refused() {
        let v = instruct_json("cashapp", "terp1qqqq");
        assert_eq!(v["error"], "cashapp_door_only");
    }

    #[test]
    fn penumbra_is_coming_not_selectable() {
        let v = penumbra_instruct();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "penumbra_coming");
        assert_eq!(v["paid"], false);
        assert_eq!(v["coming"], true);
    }

    #[test]
    fn routes_exist() {
        assert!(try_handle("GET", "/itl/pay/health", "").is_some());
        assert!(try_handle("GET", "/itl/pay/quote", "").is_some());
        assert!(try_handle("GET", "/itl/pay/instruct?rail=zec", "").is_some());
        assert!(try_handle("GET", "/itl/pay/ticket?binding=x", "").is_some());
        assert!(try_handle("POST", "/itl/pay/watch", "{}").is_some());
        assert!(try_handle("GET", "/frost", "").is_none());
    }

    #[test]
    fn browser_forged_paid_does_not_unlock() {
        let forged = try_handle("POST", "/itl/pay/watch", r#"{"paid":true}"#).unwrap();
        assert_eq!(forged.status, 400);
        let v: Value = serde_json::from_str(&forged.body).unwrap();
        assert_eq!(v["error"], "browser_paid");
        assert_eq!(v["paid"], false);

        let post_ticket = try_handle(
            "POST",
            "/itl/pay/ticket",
            r#"{"paid":true,"rail":"zec","binding":"forge-binding"}"#,
        )
        .unwrap();
        assert_eq!(post_ticket.status, 400);
        let t: Value = serde_json::from_str(&post_ticket.body).unwrap();
        assert_eq!(t["error"], "browser_paid");
        assert_eq!(t["paid"], false);

        let read = try_handle("GET", "/itl/pay/ticket?binding=forge-binding", "").unwrap();
        assert_eq!(read.status, 200);
        let r: Value = serde_json::from_str(&read.body).unwrap();
        assert_eq!(r["paid"], false);
    }

    #[test]
    fn watcher_fact_unlocks_and_browser_paid_flag_is_ignored() {
        let body = r#"{
            "rail":"zec",
            "record_id":"memo_commit_r2_watch_01",
            "dest":"zcash:u1lab?amount=0.068&memo=YWJj",
            "binding":"terp1r2watchunlock000000000000000000000000",
            "paid":true
        }"#;
        let w = try_handle("POST", "/itl/pay/watch", body).unwrap();
        assert_eq!(w.status, 200);
        let v: Value = serde_json::from_str(&w.body).unwrap();
        assert_eq!(v["paid"], true);
        assert_eq!(v["record_id"], "memo_commit_r2_watch_01");

        let read = try_handle(
            "GET",
            "/itl/pay/ticket?binding=terp1r2watchunlock000000000000000000000000",
            "",
        )
        .unwrap();
        let r: Value = serde_json::from_str(&read.body).unwrap();
        assert_eq!(r["paid"], true);
        assert_eq!(r["record_id"], "memo_commit_r2_watch_01");
        assert!(r.get("ticket_secret").is_none());
    }
}
