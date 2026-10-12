//! Node-confirmed 20% then 80% ZEC payouts to checksummed t-addrs via real PCZT
//! (not dummy [0xaa;32] sighash). Spawns zakura + ztreamer, then the proven
//! frost_ironwood_send_v6 live path.
//!
//!   ICT_SPAWN=1 cargo test --manifest-path tools/onboard-authz/Cargo.toml \
//!     --test itl_frost_notes_live -- --ignored --nocapture

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hash_market::onboard_authz::itl_frost::{
    confirm_txid_on_node, lab_taddr, require_node_txid, require_transparent_taddr, JobStatus,
    Ledger, PayRecord, PayeeKind, PendingJob, PayoutLeg, TicketInflow, FROST_N, FROST_T,
};

const RPC: &str = "http://127.0.0.1:18232";
const ZTREAMER_BIND: &str = "127.0.0.1:19067";

fn refuse_mock() {
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        panic!("itl_frost_notes_live refuses ICT_MOCK=1");
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../")
}

fn docker_bin() -> &'static str {
    if PathBuf::from("/usr/local/bin/docker").is_file() {
        "/usr/local/bin/docker"
    } else if PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker").is_file() {
        "/Applications/Docker.app/Contents/Resources/bin/docker"
    } else {
        "docker"
    }
}

fn rpc_ready() -> bool {
    Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "3",
            "-X",
            "POST",
            &format!("{RPC}/"),
            "-H",
            "Content-Type: application/json",
            "-d",
            r#"{"jsonrpc":"2.0","id":1,"method":"getblockchaininfo","params":[]}"#,
        ])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("\"result\""))
        .unwrap_or(false)
}

fn rpc_is_nu63() -> bool {
    let out = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "5",
            "-X",
            "POST",
            &format!("{RPC}/"),
            "-H",
            "Content-Type: application/json",
            "-d",
            r#"{"jsonrpc":"2.0","id":1,"method":"getblockchaininfo","params":[]}"#,
        ])
        .output()
        .ok();
    let Some(o) = out else { return false };
    let s = String::from_utf8_lossy(&o.stdout);
    s.contains("NU6.3") || s.contains("37a5165b") || s.contains("ironwood")
}

fn spawn_zakura() {
    if std::env::var("SVG_MINT_ZAKURA_REUSE").ok().as_deref() == Some("1") && rpc_ready() && rpc_is_nu63()
    {
        eprintln!("reusing zakura RPC at {RPC}");
        return;
    }
    if rpc_ready() && rpc_is_nu63() {
        eprintln!("zakura RPC already NU6.3 — reuse");
        return;
    }
    let bin = root().join("crates/zakura/target/zakura-regtest-e2e-linux/zakurad");
    assert!(bin.is_file(), "ZAKURAD_BIN missing at {}", bin.display());
    let tmp = std::env::temp_dir().join("itl-frost-notes-corridor");
    let _ = std::fs::create_dir_all(&tmp);
    let cfg = tmp.join("node-corridor.toml");
    std::fs::write(
        &cfg,
        r#"# ITL frost notes live
[network]
network = "Regtest"
listen_addr = "0.0.0.0:18233"
p2p_stack = "legacy"
cache_dir = false
initial_testnet_peers = []
max_connections_per_ip = 4

[network.testnet_parameters.activation_heights]
NU5 = 1
NU6 = 1
"NU6.1" = 2
"NU6.2" = 2
"NU6.3" = 2

[[network.testnet_parameters.funding_streams]]
height_range = { start = 1, end = 2 }
recipients = [{ receiver = "Deferred", numerator = 1 }]

[[network.testnet_parameters.lockbox_disbursements]]
address = "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8"
amount = 6250000

[state]
ephemeral = true

[rpc]
listen_addr = "0.0.0.0:18232"
enable_cookie_auth = false

[metrics]
endpoint_addr = "0.0.0.0:19901"

[mining]
internal_miner = false
miner_address = "tmLPctKo9j49rtCSKpwEBpLBeykiTGomGQs"

[tracing]
filter = "info"
"#,
    )
    .unwrap();
    let _ = Command::new(docker_bin())
        .args(["rm", "-f", "ict-itl-frost-notes-zakura-18232"])
        .status();
    let status = Command::new(docker_bin())
        .args([
            "run",
            "-d",
            "--name",
            "ict-itl-frost-notes-zakura-18232",
            "-p",
            "18232:18232",
            "-v",
            &format!("{}:/usr/local/bin/zakurad:ro", bin.display()),
            "-v",
            &format!("{}:/etc/zakura/node-corridor.toml:ro", cfg.display()),
            "debian:trixie-slim",
            "/usr/local/bin/zakurad",
            "--config",
            "/etc/zakura/node-corridor.toml",
            "start",
        ])
        .status()
        .expect("docker run zakura");
    assert!(status.success(), "docker run zakura failed");
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if rpc_ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("zakura RPC did not come up on {RPC}");
}

struct Ztreamer(Option<std::process::Child>);
impl Drop for Ztreamer {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn spawn_ztreamer() -> Ztreamer {
    if std::net::TcpStream::connect(ZTREAMER_BIND).is_ok() {
        eprintln!("reusing ztreamer {ZTREAMER_BIND}");
        return Ztreamer(None);
    }
    let bin = root().join("crates/ztreamer/target/release/ztreamer");
    assert!(bin.is_file(), "ZTREAMER_BIN missing at {}", bin.display());
    let child = Command::new(&bin)
        .args(["--bind", ZTREAMER_BIND])
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("ztreamer spawn");
    for _ in 0..50 {
        if std::net::TcpStream::connect(ZTREAMER_BIND).is_ok() {
            std::env::set_var("ZCLI_ENDPOINT", format!("http://{ZTREAMER_BIND}"));
            std::env::set_var("ZTREAMER_GRPC", ZTREAMER_BIND);
            return Ztreamer(Some(child));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("ztreamer did not listen on {ZTREAMER_BIND}");
}

fn grab(stdout: &str, key: &str) -> Option<String> {
    stdout.lines().find_map(|l| {
        l.trim()
            .strip_prefix(key)
            .map(|s| s.trim_start_matches('=').trim().to_string())
    })
}

#[ignore = "needs ICT_SPAWN"]
#[test]
fn itl_frost_notes_20_then_80_node_confirmed() {
    refuse_mock();
    spawn_zakura();
    let _zt = spawn_ztreamer();
    if !rpc_is_nu63() {
        panic!("zakura RPC up but not NU6.3 — residual: no ironwood notes");
    }
    require_transparent_taddr(&lab_taddr(1)).unwrap();

    let zcli = root().join("crates/zcli");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&zcli)
        .args([
            "test",
            "--release",
            "-p",
            "zafu-wasm",
            "--test",
            "frost_ironwood_send_v6",
            "frost_2_of_3_pays_taddr_20_then_80_live",
            "--",
            "--ignored",
            "--nocapture",
            "--exact",
        ])
        .env("ZAKURA_RPC", RPC)
        .env("ZEBRAD_RPC", RPC)
        .env("ICT_SPAWN", "1")
        .env(
            "PATH",
            format!(
                "/usr/local/bin:/Applications/Docker.app/Contents/Resources/bin:{}:/usr/bin:/bin",
                std::env::var("HOME")
                    .map(|h| format!("{h}/.cargo/bin"))
                    .unwrap_or_default()
            ),
        );
    let out = cmd.output().expect("cargo test zafu live");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let blob = format!("{stdout}\n{stderr}");
    println!("{blob}");
    if !out.status.success() {
        panic!("zafu live PCZT path failed (no node-confirmed notes/txids)");
    }
    let txid_20 = grab(&blob, "ITL_TXID_20").expect("ITL_TXID_20");
    let txid_80 = grab(&blob, "ITL_TXID_80").expect("ITL_TXID_80");
    let t20 = grab(&blob, "ITL_TADDR_20").unwrap_or_else(|| lab_taddr(1));
    let t80 = grab(&blob, "ITL_TADDR_80").unwrap_or_else(|| lab_taddr(1));
    require_node_txid(&txid_20).unwrap();
    require_node_txid(&txid_80).unwrap();
    assert_ne!(txid_20, txid_80);
    confirm_txid_on_node(RPC, &txid_20).unwrap();
    confirm_txid_on_node(RPC, &txid_80).unwrap();
    require_transparent_taddr(&t20).unwrap();
    require_transparent_taddr(&t80).unwrap();

    let mut l = Ledger::default();
    l.confirm_inflow(TicketInflow {
        record_id: "ticket-cashapp-lab".into(),
        rail: "cashapp".into(),
        amount_label: "list".into(),
    })
    .unwrap();
    l.enqueue(PayRecord {
        code: "pay-20-ink".into(),
        kind: PayeeKind::Artist,
        name: "ink".into(),
        taddr: lab_taddr(1),
        leg: PayoutLeg::Twenty,
        amount_label: "20% upfront".into(),
        attest_url: Some("https://example.invalid/attest/ink".into()),
        txid: None,
    })
    .unwrap();
    l.mark_broadcast("pay-20-ink", &txid_20).unwrap();
    l.enqueue(PayRecord {
        code: "pay-80-ink".into(),
        kind: PayeeKind::Artist,
        name: "ink".into(),
        taddr: lab_taddr(1),
        leg: PayoutLeg::Eighty,
        amount_label: "80% after event".into(),
        attest_url: Some("https://example.invalid/attest/ink".into()),
        txid: None,
    })
    .unwrap();
    l.mark_broadcast("pay-80-ink", &txid_80).unwrap();

    let rc = hash_market::onboard_authz::itl_frost_rotate::FrostCommittee::new(
        "vk-old",
        &["desk", "steward", "spare"],
    )
    .roster_commit();
    let mut job20 = PendingJob::open_bound("job-20", "pay-20-ink", FROST_T, FROST_N, &rc).unwrap();
    job20.submit_share("desk", "S:0:a").unwrap();
    job20.submit_share("steward", "S:0:b").unwrap();
    assert_eq!(job20.status, JobStatus::Threshold);
    job20.broadcast_node_confirmed(&txid_20, RPC).unwrap();
    let mut job80 = PendingJob::open_bound("job-80", "pay-80-ink", FROST_T, FROST_N, &rc).unwrap();
    job80.submit_share("desk", "S:0:a").unwrap();
    job80.submit_share("steward", "S:0:b").unwrap();
    job80.broadcast_node_confirmed(&txid_80, RPC).unwrap();

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "txid_20": txid_20,
            "txid_80": txid_80,
            "taddrs": [t20, t80, lab_taddr(1)],
        })
    );
}
