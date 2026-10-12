//! Ecosystem combination tests — one grant plane, all presets/modes.
//! Product crates keep their own focused tests; this file is the *composition*.
//!
//! Offline (always): curate + in-process HTTP runtime.
//! Live LCD (ICT_LCD): confirm DAO / authenticator / norick codes from one chain.
//!
//!   cargo test -p terp-onboard-authz --test combo_e2e
//!   ICT_LCD=https://testnet-api.terp.network ICT_CHAIN=120u-1 \
//!     cargo test -p terp-onboard-authz --test combo_e2e -- --ignored --nocapture

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use hash_market::onboard_authz::curate::{curate, load_infra, preset, CurateMode, PRESETS};
use hash_market::onboard_authz::http::serve;
use hash_market::onboard_authz::{OnboardRoles, OnboardRuntime};

fn roles() -> OnboardRoles {
    OnboardRoles {
        runtime: std::env::var("ONBOARD_RUNTIME")
            .or_else(|_| std::env::var("ONBOARD_DAO"))
            .unwrap_or_else(|_| "terp1dao".into()),
        hashmerchant: std::env::var("ONBOARD_HASHMERCHANT")
            .unwrap_or_else(|_| "terp1runtime".into()),
        chain_id: std::env::var("ICT_CHAIN")
            .or_else(|_| std::env::var("ONBOARD_CHAIN"))
            .unwrap_or_else(|_| "120u-1".into()),
    }
}

#[test]
fn combo_curate_all_presets_three_modes() {
    let infra = load_infra(None);
    let r = roles();
    for p in PRESETS {
        for mode in [
            CurateMode::DaoProposal,
            CurateMode::WalletProxy,
            CurateMode::WalletAsRuntime,
        ] {
            let b = curate(mode, &r, p, &infra, None, "terp1aliceqqqqqqqqqqqqqqqqqqqqqqqqqqqq");
            assert!(
                !b.preset.allowance_msg_types.is_empty(),
                "{} {:?}",
                p.id,
                mode
            );
            match mode {
                CurateMode::DaoProposal => assert!(b.dao_proposal.is_some(), "{}", p.id),
                _ => assert!(b.dao_proposal.is_none(), "{}", p.id),
            }
        }
    }
    if r.chain_id == "120u-1" {
        assert_eq!(
            preset("norick")
                .unwrap()
                .resolve("120u-1", &infra)
                .code_ids
                .get("headstash:cw_norick"),
            Some(&72)
        );
        assert_eq!(
            preset("register")
                .unwrap()
                .resolve("120u-1", &infra)
                .code_ids
                .get("terp-passkey"),
            Some(&4)
        );
    }
}

#[test]
fn combo_runtime_register_and_mint_grants() {
    std::env::set_var("ONBOARD_SKIP_PAID", "1");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let bind = addr.to_string();
    let bind_c = bind.clone();
    thread::spawn(move || {
        let mut rt = OnboardRuntime::from_env();
        rt.roles = roles();
        rt.service_name = "combo-onboard".into();
        rt.broadcast = false;
        rt.broadcaster = None;
        let _ = serve(&bind_c, std::sync::Arc::new(rt));
    });
    thread::sleep(std::time::Duration::from_millis(80));

    let status = http_get(&format!("http://{bind}/status"));
    assert!(status.contains("combo-onboard"), "{status}");
    assert!(
        status.contains("/terp.smartaccount.v1beta1.MsgAddAuthenticator"),
        "{status}"
    );
    assert!(
        !status.contains("add_authenticator"),
        "must not advertise CosmWasm execute shape: {status}"
    );

    let user = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0";
    let reg = http_post(
        &format!("http://{bind}/v1/register-grant"),
        &format!(r#"{{"address":"{user}"}}"#),
    );
    assert!(reg.contains("AllowedMsgAllowance"), "{reg}");
    let reg_v: serde_json::Value = serde_json::from_str(&reg).expect("reg json");
    let allowed = reg_v["msg"]["allowance"]["allowed_messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        allowed
            .iter()
            .all(|x| x.as_str() != Some("/cosmos.bank.v1beta1.MsgSend")),
        "{reg}"
    );
    assert_eq!(reg_v["runway"]["amount"], "1000000", "{reg}");

    let mint = http_post(
        &format!("http://{bind}/v1/mint-grant"),
        &format!(r#"{{"address":"{user}"}}"#),
    );
    assert!(mint.contains("does not grant mint") || mint.contains("\"ok\":false"), "{mint}");
    let health = http_get(&format!("http://{bind}/health"));
    assert!(health.contains("\"ok\":true") || health.contains("\"ready\":true"), "{health}");
    assert!(!status.contains("MsgExecuteContract"), "{status}");
}

#[test]
#[ignore = "needs ICT_LCD (one spawned or public 120u-1 with DAO codes)"]
fn combo_lcd_dao_infra_matches_presets() {
    let lcd = std::env::var("ICT_LCD").expect("ICT_LCD");
    let chain = std::env::var("ICT_CHAIN").unwrap_or_else(|_| "120u-1".into());
    let infra = load_infra(None);
    let Some(expected) = infra.chains.get(&chain) else {
        panic!("no infra snapshot for {chain}");
    };
    // Parallel-ish: hit every known code id we care about for this combo.
    let keys = [
        "dao:dao_dao_core",
        "dao:dao_proposal_single",
        "terp-passkey",
        "terp-zkjwt",
        "headstash:cw_norick",
        "abstract:abstract_account",
    ];
    let handles: Vec<_> = keys
        .iter()
        .filter_map(|k| expected.code_ids.get(*k).map(|id| (*k, *id)))
        .map(|(k, id)| {
            let lcd = lcd.clone();
            thread::spawn(move || {
                let url = format!(
                    "{}/cosmwasm/wasm/v1/code/{}",
                    lcd.trim_end_matches('/'),
                    id
                );
                let body = http_get(&url);
                (k, id, body)
            })
        })
        .collect();
    assert!(!handles.is_empty(), "no code ids to check on {chain}");
    for h in handles {
        let (k, id, body) = h.join().expect("thread");
        assert!(
            body.contains(&format!("\"code_id\":\"{id}\""))
                || body.contains(&format!("\"code_id\":{id}"))
                || body.contains("code_info"),
            "{k} code {id} missing on {lcd}: {}",
            body.chars().take(240).collect::<String>()
        );
    }
}

fn http_get(url: &str) -> String {
    if let Ok(s) = tiny_http(url, "GET", None) {
        return s;
    }
    let out = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "8", url])
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn http_post(url: &str, body: &str) -> String {
    if let Ok(s) = tiny_http(url, "POST", Some(body)) {
        return s;
    }
    let out = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "8",
            "-H",
            "content-type: application/json",
            "-d",
            body,
            url,
        ])
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Avoid depending on curl for the in-process runtime (CI images have it anyway).
fn tiny_http(url: &str, method: &str, body: Option<&str>) -> Result<String, ()> {
    let u = url.strip_prefix("http://").ok_or(())?;
    let (hostport, path) = u.split_once('/').ok_or(())?;
    let path = format!("/{path}");
    let mut stream = std::net::TcpStream::connect(hostport).map_err(|_| ())?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(4)))
        .ok();
    let payload = body.unwrap_or("");
    let req = if method == "POST" {
        format!(
            "POST {path} HTTP/1.0\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        )
    } else {
        format!("GET {path} HTTP/1.0\r\nHost: {hostport}\r\nConnection: close\r\n\r\n")
    };
    stream.write_all(req.as_bytes()).map_err(|_| ())?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).map_err(|_| ())?;
    Ok(buf.split("\r\n\r\n").nth(1).unwrap_or(&buf).to_string())
}
