//! Durable FROST inbox: two holder POSTs, kill, restart still sees threshold, one broadcast.
//! Local always (tempdir). No Docker.

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use terp_onboard_authz::frost_http::FrostInbox;
use terp_onboard_authz::itl_frost::persist_required;

fn tmp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("itl-frost-persist-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn http_json(bind: &str, method: &str, path: &str, body: &str) -> (u16, Value) {
    let mut s = TcpStream::connect(bind).expect("connect frost inbox");
    s.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    let code = buf
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    let v: Value = serde_json::from_str(body.trim()).unwrap_or(Value::Null);
    (code, v)
}

fn wait_up(bind: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if TcpStream::connect(bind).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("frost-inbox did not bind {bind}");
}

fn free_loopback() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    format!("127.0.0.1:{port}")
}

fn spawn_inbox(bind: &str, key: &str, dir: &std::path::Path) -> Child {
    let bin = env!("CARGO_BIN_EXE_frost-inbox");
    Command::new(bin)
        .env("ITL_FROST_PERSIST_KEY", key)
        .env("ITL_FROST_PERSIST_DIR", dir)
        .env("HASHMERCHANT_BIND", bind)
        .env("ITL_FROST_BIND", bind)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn frost-inbox")
}

fn kill_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn missing_key_stays_persistence_off_ram_only() {
    std::env::remove_var("ITL_FROST_PERSIST_KEY");
    assert_eq!(persist_required(), Err("persistence_off"));
    let dir = tmp_dir("off");
    std::env::set_var("ITL_FROST_PERSIST_DIR", &dir);
    let mut a = FrostInbox::from_env().unwrap();
    assert!(!a.persist_on());
    let rc = terp_onboard_authz::itl_frost_rotate::FrostCommittee::new(
        "vk-old",
        &["desk", "steward", "spare"],
    )
    .roster_commit();
    let r = a
        .try_handle(
            "POST",
            "/itl/frost/jobs",
            &format!(r#"{{"pay_code":"pay-20-ink","roster_commit":"{rc}"}}"#),
        )
        .unwrap();
    assert_eq!(r.status, 200);
    drop(a);
    let mut b = FrostInbox::from_env().unwrap();
    let g = b.try_handle("GET", "/itl/frost/jobs/job-1", "").unwrap();
    assert_eq!(
        g.status, 404,
        "RAM-only must not restore without key: {}",
        g.body
    );
    assert!(!dir.join("inbox.seal").exists());
    let _ = std::fs::remove_dir_all(&dir);
    std::env::remove_var("ITL_FROST_PERSIST_DIR");
}

#[test]
fn two_shares_kill_restart_get_job_one_broadcast() {
    let dir = tmp_dir("kill");
    let key = "sixteen-chars-ok";
    let bind = free_loopback();
    assert!(bind.starts_with("127.0.0.1:"), "{bind}");

    let mut child = spawn_inbox(&bind, key, &dir);
    wait_up(&bind);

    let (c, v) = http_json(
        &bind,
        "POST",
        "/itl/frost/jobs",
        &{
            let rc = terp_onboard_authz::itl_frost_rotate::FrostCommittee::new(
                "vk-old",
                &["desk", "steward", "spare"],
            )
            .roster_commit();
            format!(r#"{{"pay_code":"pay-20-ink","scheme":"RedPallas","target":"Zcash","roster_commit":"{rc}"}}"#)
        },
    );
    assert_eq!(c, 200, "{v}");
    let id = v["id"].as_str().expect("id").to_string();

    let (c1, v1) = http_json(
        &bind,
        "POST",
        &format!("/itl/frost/jobs/{id}/share"),
        r#"{"admin_id":"desk","blob":"S:0:a"}"#,
    );
    assert_eq!(c1, 200, "{v1}");
    assert_eq!(v1["status"], "pending");

    let (c2, v2) = http_json(
        &bind,
        "POST",
        &format!("/itl/frost/jobs/{id}/share"),
        r#"{"admin_id":"steward","blob":"S:0:b"}"#,
    );
    assert_eq!(c2, 200, "{v2}");
    assert_eq!(v2["status"], "threshold");

    kill_wait(&mut child);

    let mut child2 = spawn_inbox(&bind, key, &dir);
    wait_up(&bind);

    let (cg, vg) = http_json(&bind, "GET", &format!("/itl/frost/jobs/{id}"), "");
    assert_eq!(cg, 200, "{vg}");
    assert_eq!(vg["ok"], true);
    let st = vg["status"].as_str().unwrap_or("");
    assert!(
        st == "threshold" || st == "pending" || st == "ready",
        "GET job still pending/ready after restart, got {st} {vg}"
    );
    assert_eq!(vg["shares"], 2, "two shares restored {vg}");

    let (cb, vb) = http_json(
        &bind,
        "POST",
        &format!("/itl/frost/jobs/{id}/broadcast"),
        r#"{"txid":"txid-20"}"#,
    );
    assert_eq!(cb, 200, "{vb}");
    assert_eq!(vb["status"], "broadcast");
    assert_eq!(vb["txid"], "txid-20");

    kill_wait(&mut child2);
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!("proof: two shares; kill; restart; GET job still pending/ready; one broadcast. Bind is {bind}");
}
