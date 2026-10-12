//! Federation HTTP: transport only. Turns require ed25519 M-of-N QC.
//! Durable cell store. Not HashMerchant.

use dregg::cell::{Cell, Phase, Turn};
use dregg::commit::{ActionOpening, DepositOpening, ForeignWitness, Rail};
use dregg::hd::HdEngine;
use dregg::memo::parse_terp_recipient;
use dregg::pedersen::{self, Commit};
use dregg::store::PendingPay;
use dregg::checkout::{self, CheckoutStatus};
use dregg::prices;
use dregg::zip321::{payment_request, zat_from_zec_decimal};
use dregg::observer::{confirm_rail, wasm_supply_tag};
use dregg::payout::{RailBooks, RailPayout};
use dregg::qc::{count_valid, sign, Checkpoint, OracleSig};
use dregg::store::Durable;
use ed25519_dalek::{Keypair, PublicKey, SecretKey, Signature, Verifier};
use serde::Deserialize;
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod sessions;

struct Fed {
    cell: Cell,
    books: RailBooks,
    durable: Durable,
    allowed: Vec<[u8; 32]>,
    threshold: u32,
    operators: Vec<Keypair>,
    epoch: u64,
    /// Terp mint recipient (off-chain only) → deposit nullifier.
    by_recipient: HashMap<String, [u8; 32]>,
    pending: HashMap<Commit, PendingPay>,
    hd_index: u32,
    hd: HdEngine,
    bogo_seen: u64,
    mempool: HashMap<String, serde_json::Value>,
    /// Shielded txids currently in the house node mempool (getrawmempool).
    house_mempool_txids: Vec<String>,
    admin_challenges: HashMap<String, u64>,
    sessions: sessions::Log,
    recipients_path: String,
}

fn recipients_path(store: &str) -> String {
    format!("{store}.recipients.json")
}

fn load_recipients(path: &str) -> HashMap<String, [u8; 32]> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(map) = serde_json::from_slice::<HashMap<String, String>>(&bytes) else {
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(k, hexn)| {
            let raw = hex::decode(hexn).ok()?;
            if raw.len() != 32 {
                return None;
            }
            let mut n = [0u8; 32];
            n.copy_from_slice(&raw);
            Some((k, n))
        })
        .collect()
}

fn save_recipients(path: &str, map: &HashMap<String, [u8; 32]>) {
    let hexmap: HashMap<String, String> = map.iter().map(|(k, n)| (k.clone(), hex::encode(n))).collect();
    if let Ok(bytes) = serde_json::to_vec(&hexmap) {
        let _ = std::fs::write(path, bytes);
    }
}

fn load_keys() -> (Vec<Keypair>, Vec<[u8; 32]>, u32) {
    let thresh: u32 = std::env::var("SVG_MINT_FED_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    if let Ok(hexsk) = std::env::var("SVG_MINT_FED_SK_HEX") {
        let raw = hex::decode(hexsk.trim()).expect("sk hex");
        assert_eq!(raw.len(), 32, "ed25519 sk 32 bytes");
        let sk = SecretKey::from_bytes(&raw).expect("sk");
        let pk = PublicKey::from(&sk);
        let kp = Keypair { secret: sk, public: pk };
        let pk_bytes = kp.public.to_bytes();
        return (vec![kp], vec![pk_bytes], thresh);
    }
    let kp = dregg::qc::generate_operator();
    let pk_bytes = kp.public.to_bytes();
    eprintln!(
        "svg-mint-fed generated operator pk={}",
        hex::encode(pk_bytes)
    );
    (vec![kp], vec![pk_bytes], thresh)
}

fn main() {
    let bind = std::env::var("SVG_MINT_FED_BIND").unwrap_or_else(|_| "0.0.0.0:8421".into());
    let store_path = std::env::var("SVG_MINT_FED_STORE")
        .unwrap_or_else(|_| "/tmp/svg-mint-fed-cell.json".into());
    let (durable, mut cell, books, pending, hd_index, bogo_seen) = Durable::open(&store_path).expect("store");
    if !cell.supply("svg").0 {
        let max_supply = std::env::var("DREGG_SVG_MAX_SUPPLY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &u64| *n > 0)
            .unwrap_or(10_000);
        let _ = cell.apply(Turn::SetCollection {
            id: "svg".into(),
            max_supply,
        });
        eprintln!("svg-mint-fed bound collection svg max_supply={max_supply}");
    }
    let hd = HdEngine::from_env();
    if dregg::rpc_allowlist::require_self_hosted() {
        if let Err(e) = dregg::rpc_allowlist::check_watch_rpcs() {
            eprintln!("svg-mint-fed refuse: {e}");
            std::process::exit(2);
        }
    }
    let (operators, allowed, threshold) = load_keys();
    // Feegrant and the mint desk are one process. Two ports, one binary.
    std::thread::spawn(|| {
        let port = std::env::var("ONBOARD_PORT").unwrap_or_else(|_| "19090".into());
        let host = std::env::var("ONBOARD_BIND").unwrap_or_else(|_| "127.0.0.1".into());
        let obind = if host.contains(':') {
            host
        } else {
            format!("{host}:{port}")
        };
        let runtime = std::sync::Arc::new(hash_market::onboard_authz::OnboardRuntime::from_env());
        eprintln!("onboard listening {obind}");
        if let Err(e) = hash_market::onboard_authz::http::serve(&obind, runtime) {
            eprintln!("onboard serve {e}");
        }
    });
    let listener = TcpListener::bind(&bind).expect("bind");
    eprintln!("svg-mint-fed listening {bind} store={store_path} threshold={threshold}");
    let fed = Arc::new(Mutex::new(Fed {
        cell,
        books,
        durable,
        allowed,
        threshold,
        operators,
        epoch: 1,
        by_recipient: load_recipients(&recipients_path(&store_path)),
        pending,
        hd_index,
        hd,
        bogo_seen,
        mempool: HashMap::new(),
        house_mempool_txids: Vec::new(),
        admin_challenges: HashMap::new(),
        sessions: sessions::Log::open(format!("{store_path}.sessions.json")),
        recipients_path: recipients_path(&store_path),
    }));
    spawn_note_watchers(fed.clone());
    spawn_stale_sweeper(fed.clone());
    for stream in listener.incoming().flatten() {
        let fed = fed.clone();
        // /v1/watch holds the socket open. Handling it inline blocks every other request.
        std::thread::spawn(move || {
            let _ = handle(stream, fed);
        });
    }
}

fn paid_phase_name(ph: Phase) -> Option<&'static str> {
    match ph {
        Phase::Deposited => Some("Deposited"),
        Phase::Entitled => Some("Entitled"),
        Phase::Settled => Some("Settled"),
        Phase::Empty | Phase::Refunded => None,
    }
}

fn cancel_checkout(fed: &Arc<Mutex<Fed>>, rec: &str) -> (u16, serde_json::Value) {
    let mut f = fed.lock().unwrap();
    if let Some(n) = f.by_recipient.get(rec).copied() {
        if let Some(ph) = f.cell.phase(&n) {
            if let Some(name) = paid_phase_name(ph) {
                return dregg::store::cancel_http(Some(name), 0);
            }
        }
    }
    let dropped = dregg::store::cancel_unpaid(&mut f.pending, rec);
    for p in &dropped {
        btc_watch_drop(&p.btc_addr);
    }
    if !dropped.is_empty() {
        let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
    }
    dregg::store::cancel_http(None, dropped.len())
}

fn checkout_now(f: &Fed) -> CheckoutStatus {
    f.cell.checkout_now()
}

fn checkout_closed_http(st: &CheckoutStatus) -> Option<(u16, serde_json::Value)> {
    if st.checkout_ready {
        None
    } else {
        let mut p = json!({
            "ok": false,
            "pay": false,
            "error": format!("checkout closed: {}", st.reasons.join(",")),
        });
        if let serde_json::Value::Object(m) = &mut p {
            if let serde_json::Value::Object(s) = st.to_json() {
                for (k, v) in s {
                    m.insert(k, v);
                }
            }
        }
        Some((503, p))
    }
}

fn resolve_dest(fed: &Arc<Mutex<Fed>>, blob: &[u8]) -> Option<String> {
    let f = fed.lock().unwrap();
    if let Some(t) = dregg::store::dest_for_zec_watch(&f.pending, "", blob) {
        return Some(t);
    }
    drop(f);
    parse_terp_recipient(blob)
}

fn issue_pay_target(fed: &Arc<Mutex<Fed>>, terp: &str, qty: u32) -> Result<serde_json::Value, String> {
    let qty = qty.clamp(checkout::QTY_MIN, checkout::QTY_MAX);
    let mut rng = rand::rngs::OsRng;
    let mut session = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rng, &mut session);
    let (c, r) = pedersen::commit_session(terp, &session, &mut rng);
    let mut f = fed.lock().unwrap();
    let st = checkout_now(&f);
    let reserved: u64 = f
        .pending
        .values()
        .map(|p| p.qty.max(1) as u64)
        .sum();
    if !st.checkout_ready || st.remaining_supply < reserved.saturating_add(qty as u64) {
        return Err(format!(
            "checkout closed: {}",
            if st.reasons.is_empty() {
                "reserved_sold_out".into()
            } else {
                st.reasons.join(",")
            }
        ));
    }
    let i = f.hd_index;
    f.hd_index = f.hd_index.saturating_add(1);
    let main = std::env::var("DREGG_BTC_MAINNET").ok().as_deref() == Some("1");
    let btc = f.hd.btc_p2wpkh(i, main)?;
    let eth = f.hd.eth_addr(i)?;
    let div = f.hd.zec_diversifier(i);
    let zat_unit = prices::zec_zat();
    let zat = zat_unit.saturating_mul(qty as u64);
    let native = prices::NATIVE_AMOUNT.saturating_mul(qty as u128);
    let eth_wei = prices::eth_wei()
        .parse::<u128>()
        .unwrap_or(prices::ETH_WEI)
        .saturating_mul(qty as u128)
        .to_string();
    let base_wei = prices::base_wei()
        .parse::<u128>()
        .unwrap_or(prices::ETH_WEI)
        .saturating_mul(qty as u128)
        .to_string();
    let btc_sats = prices::btc_sats().saturating_mul(qty as u64);
    let bogo = prices::nft_count_for_rail(f.bogo_seen);
    let zec_ua = if st.zec_tip.ok {
        diversified_zec_ua(i).or_else(|| std::env::var("DREGG_ZEC_UA").ok())
    } else {
        None
    };
    let zec_uri = zec_ua
        .as_ref()
        .and_then(|u| payment_request(u, zat, &c).ok());
    let memo_hex = pedersen::memo_hex(&c);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    f.pending.insert(
        c,
        PendingPay {
            terp: terp.to_string(),
            blinding: r,
            hd_index: i,
            eth_addr: eth.clone(),
            btc_addr: btc.clone(),
            zec_div_hex: hex::encode(div),
            zec_ua: zec_ua.clone(),
            issued_at: now,
            qty,
            session: session.to_vec(),
        },
    );
    f.sessions.issue(terp, qty, zat);
    let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
    let ttl = checkout::pending_ttl_secs();
    let bogo_seen = f.bogo_seen;
    drop(f);
    btc_watch_import(&btc);
    Ok(json!({
        "ok": true,
        "eth_addr": eth,
        "eth_vault": std::env::var("DREGG_ETH_VAULT").ok(),
        "base_addr": eth,
        "base_vault": std::env::var("DREGG_BASE_VAULT").ok(),
        "base_chain_id": prices::base_chain_id(),
        "btc_addr": btc,
        "eth_wei": eth_wei,
        "base_wei": base_wei,
        "btc_sats": btc_sats,
        "native_amount": native,
        "native_denom": prices::native_denom(),
        "qty": qty,
        "bogo": bogo == 2,
        "bogo_remaining": prices::CROSS_CHAIN_BOGO_CAP.saturating_sub(bogo_seen),
        "zec_ua": zec_ua,
        "zec_diversifier": hex::encode(div),
        "zec_zat": zat,
        "zec_uri": zec_uri,
        "zip321": zec_uri.is_some(),
        "memo_commit": hex::encode(c),
        "memo_hex": memo_hex,
        "session_hex": hex::encode(session),
        "hd_index": i,
        "or": true,
        "pending_ttl_secs": ttl,
        "memo": "put memo_hex / ZIP-321 memo (Pedersen C) in the payment — never terp1 on ETH/Base/BTC/ZEC",
    }))
}

fn zrt_bin() -> String {
    if let Ok(b) = std::env::var("SVG_MINT_ZRT_BIN") {
        if !b.is_empty() {
            return b;
        }
    }
    let sibling = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/release/svg-mint-zrt");
    sibling.to_string_lossy().into_owned()
}

fn diversified_zec_ua(index: u32) -> Option<String> {
    let _ = index;
    let bin = zrt_bin();
    if !std::path::Path::new(&bin).is_file() {
        return None;
    }
    // Same UA + unique ZIP-321 memo (Pedersen C) is the stable checkout dest.
    let mut addr_cmd = std::process::Command::new(&bin);
    addr_cmd.arg("addresses");
    if let Ok(dir) = std::env::var("SVG_MINT_ZRT_DIR") {
        if !dir.is_empty() {
            addr_cmd.env("SVG_MINT_ZRT_DIR", dir);
        }
    }
    let out = addr_cmd
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let t = line.trim();
        for prefix in ["Default Address:", "Address:"] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let ua = rest.trim();
                if ua.starts_with('u') && ua.len() > 20 {
                    return Some(ua.to_string());
                }
            }
        }
        if let Some(rest) = t.strip_prefix("Unified Address:") {
            let ua = rest.trim();
            if !ua.is_empty() {
                return Some(ua.to_string());
            }
        }
        if t.starts_with("u1") && t.len() > 20 {
            return Some(t.to_string());
        }
    }
    None
}

fn spawn_stale_sweeper(fed: Arc<Mutex<Fed>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(15));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut f = match fed.lock() {
            Ok(g) => g,
            Err(_) => continue,
        };
        let _ = f.cell.apply(Turn::Tick { now });
        let _ = f.cell.apply(Turn::SweepStaleEntitlements {
            timeout_secs: checkout::mint_timeout_secs(),
        });
        let ns = f.cell.refunded_nullifiers();
        let pending = f.pending.clone();
        let wires: Vec<_> = ns
            .into_iter()
            .filter_map(|n| f.cell.refund_wire(&n).map(|w| (n, w)))
            .collect();
        drop(f);
        for (n, (rail, amt, raw)) in wires {
            let dest = btc_dest_for(&pending, &n);
            let txid = attempt_rail_refund(rail, amt, &raw, dest.as_deref());
            if let Ok(mut f) = fed.lock() {
                let bytes = txid
                    .as_deref()
                    .map(|t| {
                        hex::decode(t.trim_start_matches("0x")).unwrap_or_else(|_| t.as_bytes().to_vec())
                    })
                    .unwrap_or_else(|| blake3::hash(&n).as_bytes().to_vec());
                let _ = f.books.apply(n, RailPayout::Refund, bytes);
            }
        }
        let dropped = if let Ok(mut f) = fed.lock() {
            let keep: std::collections::HashSet<String> = f.mempool.keys().cloned().collect();
            let gone = dregg::store::expire_unpaid_keeping(
                &mut f.pending,
                now,
                checkout::pending_ttl_secs(),
                &keep,
            );
            let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
            gone
        } else {
            Vec::new()
        };
        for p in dropped {
            btc_watch_drop(&p.btc_addr);
        }
    });
}

fn apply_paid_qty(f: &mut Fed, n: &[u8; 32], amount: u128, rail: Rail) -> (u32, Vec<u32>) {
    let units = checkout::units_paid(amount, rail.price_units());
    let nfts = checkout::nfts_for_units(units, f.bogo_seen);
    f.bogo_seen = f.bogo_seen.saturating_add(units);
    let qty = nfts.min(u32::MAX as u64) as u32;
    f.cell.set_nft_qty(n, qty);
    let batches = checkout::mint_batches(qty as u64, checkout::mint_batch_size());
    (qty, batches)
}

fn spawn_note_watchers(fed: Arc<Mutex<Fed>>) {
    let f_eth = fed.clone();
    std::thread::spawn(move || loop {
        if let Err(e) = watch_eth_once(&f_eth) {
            eprintln!("eth watch: {e}");
        }
        std::thread::sleep(std::time::Duration::from_secs(12));
    });
    let f_base = fed.clone();
    std::thread::spawn(move || loop {
        if let Err(e) = watch_base_once(&f_base) {
            eprintln!("base watch: {e}");
        }
        std::thread::sleep(std::time::Duration::from_secs(12));
    });
    let f_btc = fed.clone();
    std::thread::spawn(move || loop {
        if let Err(e) = watch_btc_once(&f_btc) {
            eprintln!("btc watch: {e}");
        }
        std::thread::sleep(std::time::Duration::from_secs(12));
    });
    let f_zec = fed.clone();
    std::thread::spawn(move || loop {
        if let Err(e) = watch_zec_once(&f_zec) {
            eprintln!("zec watch: {e}");
        }
        std::thread::sleep(std::time::Duration::from_secs(12));
    });
}

fn watch_eth_once(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    let rpc = match std::env::var("ETH_RPC") {
        Ok(u) => u,
        Err(_) => return Ok(()),
    };
    if let Ok(vault) = std::env::var("DREGG_ETH_VAULT") {
    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"eth_getLogs",
        "params":[{"address": vault, "fromBlock": "earliest", "toBlock": "latest"}]
    });
    let v = http_json_local(&rpc, &body)?;
    let logs = v.pointer("/result").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    for log in logs {
        let txh = log.get("transactionHash").and_then(|x| x.as_str()).unwrap_or("");
        if txh.is_empty() {
            continue;
        }
        let tx = http_json_local(
            &rpc,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_getTransactionByHash","params":[txh]}),
        )?;
        let input = tx
            .pointer("/result/input")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let Some(rec) = resolve_dest(fed, input.as_bytes()) else {
            continue;
        };
        let raw = hex::decode(txh.trim_start_matches("0x")).unwrap_or_else(|_| txh.as_bytes().to_vec());
        let value = tx
            .pointer("/result/value")
            .and_then(|x| x.as_str())
            .unwrap_or("0x0");
        let amt = u128::from_str_radix(value.trim_start_matches("0x"), 16)
            .unwrap_or_else(|_| prices::eth_wei().parse().unwrap_or(Rail::Eth.price_units()));
        ingest_observed(fed, Rail::Eth, amt, raw, rec)?;
    }
    }
    watch_evm_hd(fed, &rpc, Rail::Eth)?;
    Ok(())
}

fn watch_base_once(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    let rpc = match std::env::var("BASE_RPC") {
        Ok(u) => u,
        Err(_) => return Ok(()),
    };
    if let Ok(vault) = std::env::var("DREGG_BASE_VAULT") {
    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"eth_getLogs",
        "params":[{"address": vault, "fromBlock": "earliest", "toBlock": "latest"}]
    });
    let v = http_json_local(&rpc, &body)?;
    let logs = v.pointer("/result").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    for log in logs {
        let txh = log.get("transactionHash").and_then(|x| x.as_str()).unwrap_or("");
        if txh.is_empty() {
            continue;
        }
        let tx = http_json_local(
            &rpc,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_getTransactionByHash","params":[txh]}),
        )?;
        let input = tx
            .pointer("/result/input")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let Some(rec) = resolve_dest(fed, input.as_bytes()) else {
            continue;
        };
        let raw = hex::decode(txh.trim_start_matches("0x")).unwrap_or_else(|_| txh.as_bytes().to_vec());
        let value = tx
            .pointer("/result/value")
            .and_then(|x| x.as_str())
            .unwrap_or("0x0");
        let amt = u128::from_str_radix(value.trim_start_matches("0x"), 16)
            .unwrap_or_else(|_| prices::base_wei().parse().unwrap_or(Rail::Base.price_units()));
        ingest_observed(fed, Rail::Base, amt, raw, rec)?;
    }
    }
    watch_evm_hd(fed, &rpc, Rail::Base)?;
    Ok(())
}

/// Native transfers to HD `eth_addr` from pay-targets (not only vault logs).
fn watch_evm_hd(fed: &Arc<Mutex<Fed>>, rpc: &str, rail: Rail) -> Result<(), String> {
    let pending: Vec<(String, String)> = {
        let f = fed.lock().unwrap();
        f.pending
            .values()
            .map(|p| (p.eth_addr.to_lowercase(), p.terp.clone()))
            .collect()
    };
    if pending.is_empty() {
        return Ok(());
    }
    let bn = http_json_local(
        rpc,
        &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}),
    )?;
    let tip = u64::from_str_radix(
        bn.pointer("/result")
            .and_then(|x| x.as_str())
            .unwrap_or("0x0")
            .trim_start_matches("0x"),
        16,
    )
    .unwrap_or(0);
    let from = tip.saturating_sub(48);
    for h in from..=tip {
        let blk = http_json_local(
            rpc,
            &serde_json::json!({
                "jsonrpc":"2.0","id":1,
                "method":"eth_getBlockByNumber",
                "params":[format!("0x{h:x}"), true]
            }),
        )?;
        let txs = blk
            .pointer("/result/transactions")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        for tx in txs {
            let to = tx
                .get("to")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_lowercase();
            let Some((_, rec)) = pending.iter().find(|(a, _)| *a == to) else {
                continue;
            };
            let hash = tx.get("hash").and_then(|x| x.as_str()).unwrap_or("");
            if hash.is_empty() {
                continue;
            }
            let value = tx.get("value").and_then(|x| x.as_str()).unwrap_or("0x0");
            let amt = u128::from_str_radix(value.trim_start_matches("0x"), 16).unwrap_or(0);
            if amt == 0 {
                continue;
            }
            let raw = hex::decode(hash.trim_start_matches("0x")).unwrap_or_else(|_| hash.as_bytes().to_vec());
            ingest_observed(fed, rail, amt, raw, rec.clone())?;
            if let Ok(mut f) = fed.lock() {
                f.pending.retain(|_, p| p.eth_addr.to_lowercase() != to);
                let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
            }
        }
    }
    Ok(())
}

fn watch_btc_once(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    let url = match std::env::var("BTC_RPC") {
        Ok(u) => u,
        Err(_) => return Ok(()),
    };
    let v = http_json_local(
        &url,
        &serde_json::json!({"jsonrpc":"1.0","id":"dregg","method":"listsinceblock","params":["", 1]}),
    )?;
    let txs = v
        .pointer("/result/transactions")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for tx in txs {
        let txid = tx.get("txid").and_then(|x| x.as_str()).unwrap_or("");
        if txid.is_empty() {
            continue;
        }
        let addr = tx.get("address").and_then(|x| x.as_str()).unwrap_or("");
        let rec = {
            let f = fed.lock().unwrap();
            f.pending
                .values()
                .find(|p| p.btc_addr == addr)
                .map(|p| p.terp.clone())
        };
        let rec = match rec {
            Some(r) => r,
            None => {
                let rawtx = http_json_local(
                    &url,
                    &serde_json::json!({"jsonrpc":"1.0","id":"dregg","method":"getrawtransaction","params":[txid, true]}),
                )?;
                match resolve_dest(fed, rawtx.to_string().as_bytes()) {
                    Some(r) => r,
                    None => continue,
                }
            }
        };
        let btc = tx.get("amount").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let sats = (btc * 100_000_000.0).round() as u128;
        let amt = if sats == 0 {
            prices::btc_sats() as u128
        } else {
            sats
        };
        let raw = hex::decode(txid).unwrap_or_else(|_| txid.as_bytes().to_vec());
        ingest_observed(fed, Rail::Btc, amt, raw, rec)?;
        if let Ok(mut f) = fed.lock() {
            f.pending.retain(|_, p| p.btc_addr != addr);
            let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
        }
        btc_watch_drop(addr);
    }
    Ok(())
}

/// ZEC: ZIP-321 memo is the terp1 dest. svg-mint-zrt is the watcher.
/// zcashd-compat `z_listreceivedbyaddress` is optional.
fn watch_zec_once(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let watched = (|| {
        ztreamer_require_vendor()?;
        if let Err(e) = watch_zec_zrt(fed) {
            eprintln!("zec watch: {e}");
        }
        Ok::<(), String>(())
    })();
    BUSY.store(false, Ordering::Release);
    if let Err(e) = watched {
        eprintln!("zec watch: {e}");
    }
    let _ = watch_zec_zcli(fed);
    let _ = watch_zec_zcashd(fed);
    let _ = watch_zec_house_mempool(fed);
    Ok(())
}

/// Compact streamer host: ZTREAMER_GRPC or ZCASH_DEVTOOL_SERVER (loopback).
fn ztreamer_grpc_host() -> Option<String> {
    for name in ["ZTREAMER_GRPC", "ZCASH_DEVTOOL_SERVER"] {
        if let Ok(s) = std::env::var(name) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                let lower = s.to_lowercase();
                if lower.contains("zec.rocks") || lower.contains("rotko.net") {
                    continue;
                }
                return Some(s);
            }
        }
    }
    None
}

/// Fail-closed: live GetLightdInfo.vendor must be Ztreamer (not ECC LWD / zidecar).
fn ztreamer_require_vendor() -> Result<(), String> {
    let Some(host) = ztreamer_grpc_host() else {
        return Ok(());
    };
    let bin = match std::env::var("ZTREAMER_BIN") {
        Ok(b) if !b.is_empty() => b,
        _ => return Ok(()),
    };
    let bind = host
        .trim_start_matches("grpc://")
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let out = std::process::Command::new(&bin)
        .args(["probe", "--bind", bind])
        .output()
        .map_err(|e| format!("ztreamer probe: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ztreamer probe failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains("\"Ztreamer\"") && !stdout.contains("Ztreamer") {
        return Err("ztreamer probe: vendor is not Ztreamer".into());
    }
    Ok(())
}


fn watch_zec_zrt(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    let bin = zrt_bin();
    if !std::path::Path::new(&bin).is_file() {
        return Ok(());
    }
    if std::env::var("SVG_MINT_SKIP_ZRT_WATCH").ok().as_deref() == Some("1") {
        return Ok(());
    }
    let mut notes_cmd = std::process::Command::new(&bin);
    notes_cmd.arg("notes");
    if let Ok(dir) = std::env::var("SVG_MINT_ZRT_DIR") {
        if !dir.is_empty() {
            notes_cmd.env("SVG_MINT_ZRT_DIR", dir);
        }
    }
    let out = notes_cmd
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "svg-mint-zrt notes: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let notes = dregg::zec_notes::parse_notes_json(&text)?;
    let pending = {
        let f = fed.lock().unwrap();
        f.pending.clone()
    };
    for note in notes {
        let Some(rec) = dregg::zec_notes::match_note(&pending, &note) else { continue };
        let txid = if note.txid.is_empty() {
            hex::encode(blake3::hash(note.address.as_bytes()).as_bytes())
        } else {
            note.txid.clone()
        };
        let raw = hex::decode(&txid).unwrap_or_else(|_| txid.as_bytes().to_vec());
        let zat = if note.zat == 0 { zec_paid_fallback(fed, &rec) } else { note.zat as u128 };
        match dregg::zec_notes::list_tx_credit(true, !note.mined) {
            dregg::zec_notes::ListTxCredit::Mint => {
                ingest_observed(fed, Rail::ZecIronwood, zat, raw, rec)?;
            }
            dregg::zec_notes::ListTxCredit::Mempool => {
                note_mempool(fed, &rec, &txid, zat);
            }
            dregg::zec_notes::ListTxCredit::Skip => {}
        }
    }
    Ok(())
}

fn watch_zec_zcli(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    let bin = match std::env::var("ZCLI_BIN") {
        Ok(b) if !b.is_empty() => b,
        _ => return Ok(()),
    };
    let ep = std::env::var("ZCLI_ENDPOINT").unwrap_or_default();
    if ep.is_empty() {
        return Err("ZCLI_BIN set but ZCLI_ENDPOINT missing (want http://127.0.0.1:9067, not zcash.rotko.net)".into());
    }
    let lower = ep.to_lowercase();
    if lower.contains("rotko.net") || lower.contains("zec.rocks") {
        return Err("ZCLI_ENDPOINT refuses public zidecar".into());
    }
    dregg::rpc_allowlist::check_env_rpc("ZCLI_ENDPOINT")?;
    // zcli --json address is offline UA (currently orchard-encoded until Ironwood receiver lands).
    // Scan/list still uses CompactTxStreamer (devtool) against ztreamer until zcli speaks native gRPC.
    let out = std::process::Command::new(&bin)
        .args(["--json", "address"])
        .env("ZCLI_ENDPOINT", &ep)
        .output()
        .map_err(|e| e.to_string())?;
    ingest_zec_blob(fed, &out.stdout)?;
    Ok(())
}

fn zcash_cookie_rpc(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let url = std::env::var("ZCASH_PRIVATE_RPC")
        .or_else(|_| std::env::var("ZCASHD_RPC"))
        .map_err(|_| "no house zcash rpc".to_string())?;
    if url.is_empty() {
        return Err("empty zcash rpc".into());
    }
    let cookie_path = std::env::var("ZCASH_PRIVATE_RPC_COOKIE")
        .unwrap_or_else(|_| format!("{}/.terp/zakura.rpc.cookie", std::env::var("HOME").unwrap_or_else(|_| ".".into())));
    let cookie = std::fs::read_to_string(&cookie_path).map_err(|e| format!("zcash cookie: {e}"))?;
    let cookie = cookie.trim();
    let body = serde_json::json!({"jsonrpc":"1.0","id":"dregg","method":method,"params":params});
    let out = std::process::Command::new("curl")
        .args([
            "-sS", "--max-time", "8",
            "-u", cookie,
            "-H", "content-type: application/json",
            "-d", &body.to_string(),
            &url,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

/// House `getrawmempool` + verbose tx. Shielded (Ironwood/Orchard) txs are the
/// mempool primitive a view-wallet sync does not see until a block.
fn watch_zec_house_mempool(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    let listed = zcash_cookie_rpc("getrawmempool", serde_json::json!([]))?;
    let ids = listed
        .get("result")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut shielded = Vec::new();
    for id in ids.iter().take(8) {
        let Some(txid) = id.as_str() else { continue };
        let tx = zcash_cookie_rpc("getrawtransaction", serde_json::json!([txid, 1]))?;
        let result = tx.get("result").cloned().unwrap_or(serde_json::Value::Null);
        let n_iw = result.pointer("/ironwood/actions").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
        let n_or = result.pointer("/orchard/actions").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
        if n_iw + n_or == 0 {
            continue;
        }
        shielded.push(txid.to_string());
    }
    let mut f = fed.lock().unwrap();
    f.house_mempool_txids = shielded;
    // Do not attribute a mempool tx to a checkout here. A note counts only after
    // the house view key trial-decrypts it and the memo is that checkout's C.
    Ok(())
}

fn watch_zec_zcashd(fed: &Arc<Mutex<Fed>>) -> Result<(), String> {
    let url = match std::env::var("ZCASHD_RPC") {
        Ok(u) if !u.is_empty() => u,
        _ => return Ok(()),
    };
    let mut uas = Vec::new();
    if let Ok(u) = std::env::var("DREGG_ZEC_UA") {
        if !u.is_empty() {
            uas.push(u);
        }
    }
    {
        let f = fed.lock().unwrap();
        for p in f.pending.values() {
            if let Some(u) = &p.zec_ua {
                if !uas.contains(u) {
                    uas.push(u.clone());
                }
            }
        }
    }
    for ua in uas {
        let v = http_json_local(
            &url,
            &serde_json::json!({
                "jsonrpc":"1.0","id":"dregg","method":"z_listreceivedbyaddress",
                "params":[ua, 0]
            }),
        )?;
        ingest_zec_json(fed, &v)?;
    }
    Ok(())
}

fn parse_zec_paid_zat(item: &serde_json::Value, line: Option<&str>) -> Option<u128> {
    if let Some(z) = item.get("amountZat").or_else(|| item.get("zatoshis")).and_then(|x| x.as_u64()) {
        if z > 0 {
            return Some(z as u128);
        }
    }
    if let Some(n) = item.get("amount").and_then(|x| x.as_u64()) {
        if n >= 1_000 {
            return Some(n as u128);
        }
        if n > 0 {
            return Some((n as u128).saturating_mul(100_000_000));
        }
    }
    if let Some(f) = item.get("amount").and_then(|x| x.as_f64()) {
        if f > 0.0 {
            return Some((f * 100_000_000.0).round() as u128);
        }
    }
    if let Some(s) = item.get("amount").and_then(|x| x.as_str()) {
        if let Some(z) = zat_from_zec_decimal(s) {
            if z > 0 {
                return Some(z as u128);
            }
        }
    }
    if let Some(line) = line {
        if let Some(a) = line.split("amount=").nth(1) {
            let a = a.split(['&', ' ', '"', ',']).next().unwrap_or("");
            if let Some(z) = zat_from_zec_decimal(a) {
                if z > 0 {
                    return Some(z as u128);
                }
            }
        }
    }
    None
}

fn zec_paid_fallback(fed: &Arc<Mutex<Fed>>, rec: &str) -> u128 {
    let unit = prices::zec_zat() as u128;
    let f = fed.lock().unwrap();
    let q = f
        .pending
        .values()
        .find(|p| p.terp == rec)
        .map(|p| p.qty.max(1) as u128)
        .unwrap_or(1);
    unit.saturating_mul(q)
}

fn ingest_zec_json(fed: &Arc<Mutex<Fed>>, v: &serde_json::Value) -> Result<(), String> {
    let arr = v
        .pointer("/result")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for item in arr {
        let memo = item
            .get("memo")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let blob = item.to_string();
        let Some(rec) = {
            let f = fed.lock().unwrap();
            dregg::store::dest_for_zec_watch(&f.pending, memo, blob.as_bytes())
        }
        .or_else(|| resolve_dest(fed, blob.as_bytes()))
        else {
            continue;
        };
        let txid = item
            .get("txid")
            .or_else(|| item.get("txId"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if txid.is_empty() {
            continue;
        }
        let raw = hex::decode(txid.trim_start_matches("0x")).unwrap_or_else(|_| txid.as_bytes().to_vec());
        let zat = parse_zec_paid_zat(&item, None).unwrap_or_else(|| zec_paid_fallback(fed, &rec));
        let conf = item.get("confirmations").and_then(|x| x.as_i64()).unwrap_or(1);
        if conf == 0 {
            note_mempool(fed, &rec, txid, zat);
            continue;
        }
        ingest_observed(fed, Rail::ZecIronwood, zat, raw, rec)?;
    }
    Ok(())
}

fn note_mempool(fed: &Arc<Mutex<Fed>>, rec: &str, txid: &str, zat: u128) {
    let mut f = fed.lock().unwrap();
    f.sessions.advance(rec, "mempool", txid, zat as u64, "");
    f.mempool.insert(
        rec.to_string(),
        serde_json::json!({
            "txid": txid,
            "zat": zat,
            "at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }),
    );
}


fn recipient_for_zec_ua(fed: &Arc<Mutex<Fed>>, ua: &str) -> Option<String> {
    let ua = ua.trim();
    if !ua.starts_with("u1") { return None; }
    let f = fed.lock().unwrap();
    f.pending.values().find(|p| p.zec_ua.as_deref() == Some(ua)).map(|p| p.terp.clone())
}

fn ingest_zec_blob(fed: &Arc<Mutex<Fed>>, blob: &[u8]) -> Result<(), String> {
    let s = String::from_utf8_lossy(blob);
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(blob) {
        ingest_zec_json(fed, &v)?;
        if v.is_array() {
            ingest_zec_json(fed, &serde_json::json!({"result": v}))?;
        }
    }
    // list-tx text: a 64-hex txid line, then "Unmined" or "Mined:", then "Memo:".
    let mut last_txid: Option<String> = None;
    let mut unmined = false;
    // ZIP-321 memo field in text: memo=<b64url> or hex memo then terp1
    for line in s.lines() {
        let trim = line.trim();
        if trim.len() == 64 && trim.chars().all(|c| c.is_ascii_hexdigit()) {
            last_txid = Some(trim.to_string());
            unmined = false;
        }
        if trim.contains("Unmined") {
            unmined = true;
        }
        if trim.starts_with("Mined:") {
            unmined = false;
        }
        if let Some(rest) = trim.strip_prefix("To:") {
            let ua = rest.trim();
            if let Some(rec) = recipient_for_zec_ua(fed, ua) {
                let txid = last_txid.clone().unwrap_or_else(|| hex::encode(blake3::hash(ua.as_bytes()).as_bytes()));
                let raw = hex::decode(&txid).unwrap_or_else(|_| txid.as_bytes().to_vec());
                let zat = zec_paid_fallback(fed, &rec);
                match dregg::zec_notes::list_tx_credit(true, unmined) {
                    dregg::zec_notes::ListTxCredit::Mempool => note_mempool(fed, &rec, &txid, zat),
                    dregg::zec_notes::ListTxCredit::Mint => {
                        ingest_observed(fed, Rail::ZecIronwood, zat, raw, rec.clone())?;
                    }
                    dregg::zec_notes::ListTxCredit::Skip => {}
                }
            }
        }
        let memo_text = if let Some(m) = line.split("memo=").nth(1) {
            Some(m.split(['&', ' ', '"', ',']).next().unwrap_or("").to_string())
        } else if let Some(rest) = trim.strip_prefix("Memo:") {
            Some(rest.trim().to_string())
        } else {
            None
        };
        if let Some(m) = memo_text {
            let m = m.split(['&', ' ', '"', ',']).next().unwrap_or("");
            if let Some(rec) = {
                let f = fed.lock().unwrap();
                dregg::store::dest_for_zec_watch(&f.pending, &m, m.as_bytes())
            }
            .or_else(|| resolve_dest(fed, m.as_bytes()))
            {
                let txid = last_txid.clone().or_else(|| extract_txid(line)).unwrap_or_else(|| {
                    hex::encode(blake3::hash(line.as_bytes()).as_bytes())
                });
                let raw = hex::decode(&txid).unwrap_or_else(|_| txid.as_bytes().to_vec());
                let zat = parse_zec_paid_zat(&serde_json::json!({}), Some(line))
                    .unwrap_or_else(|| zec_paid_fallback(fed, &rec));
                match dregg::zec_notes::list_tx_credit(true, unmined) {
                    dregg::zec_notes::ListTxCredit::Mempool => note_mempool(fed, &rec, &txid, zat),
                    dregg::zec_notes::ListTxCredit::Mint => {
                        ingest_observed(fed, Rail::ZecIronwood, zat, raw, rec)?;
                    }
                    dregg::zec_notes::ListTxCredit::Skip => {}
                }
            }
        }
        // To: and Memo: already applied list_tx_credit. This tail used to mint
        // any terp1 on the line and ignored Unmined.
        let handled = trim.starts_with("To:") || trim.starts_with("Memo:") || line.contains("memo=");
        if !handled {
            if let Some(rec) = resolve_dest(fed, line.as_bytes()) {
                let open = {
                    let f = fed.lock().unwrap();
                    f.pending.values().any(|p| p.terp == rec)
                };
                let txid = last_txid.clone().or_else(|| extract_txid(line));
                let zat = parse_zec_paid_zat(&serde_json::json!({}), Some(line))
                    .unwrap_or_else(|| zec_paid_fallback(fed, &rec));
                match dregg::zec_notes::list_tx_credit(open, unmined) {
                    dregg::zec_notes::ListTxCredit::Mempool => {
                        if let Some(txid) = txid {
                            note_mempool(fed, &rec, &txid, zat);
                        }
                    }
                    dregg::zec_notes::ListTxCredit::Mint => {
                        let Some(txid) = txid else { continue };
                        let raw = hex::decode(&txid).unwrap_or_else(|_| txid.as_bytes().to_vec());
                        ingest_observed(fed, Rail::ZecIronwood, zat, raw, rec)?;
                    }
                    dregg::zec_notes::ListTxCredit::Skip => {}
                }
            }
        }
    }
    Ok(())
}

fn urlencoding_decode(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(h, 16) {
                out.push(v as char);
                i += 3;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn extract_txid(s: &str) -> Option<String> {
    for tok in s.split(|c: char| !c.is_ascii_hexdigit()) {
        if tok.len() == 64 {
            return Some(tok.to_string());
        }
    }
    None
}

fn http_json_local(url: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    dregg::httpx::post_json(url, body)
}

/// Watch-only import of a pay-target dest. rescan=false (HD dest is new).
fn btc_watch_import(addr: &str) {
    if addr.is_empty() {
        return;
    }
    let Ok(url) = std::env::var("BTC_RPC") else {
        return;
    };
    let _ = http_json_local(
        &url,
        &serde_json::json!({
            "jsonrpc": "1.0",
            "id": "dregg",
            "method": "importaddress",
            "params": [addr, "dregg-pending", false]
        }),
    );
}

/// Core cannot delete imported addrs; relabel so operators can prune wallets.
fn btc_watch_drop(addr: &str) {
    if addr.is_empty() {
        return;
    }
    let Ok(url) = std::env::var("BTC_RPC") else {
        return;
    };
    let _ = http_json_local(
        &url,
        &serde_json::json!({
            "jsonrpc": "1.0",
            "id": "dregg",
            "method": "setlabel",
            "params": [addr, "dregg-expired"]
        }),
    );
}

fn txhash_hex(raw: &[u8]) -> String {
    if raw.len() == 32 {
        return format!("0x{}", hex::encode(raw));
    }
    let s = String::from_utf8_lossy(raw);
    if s.starts_with("0x") {
        s.to_string()
    } else {
        format!("0x{}", s)
    }
}

fn btc_dest_for(
    _pending: &HashMap<dregg::pedersen::Commit, PendingPay>,
    _n: &[u8; 32],
) -> Option<String> {
    std::env::var("DREGG_BTC_REFUND_TO")
        .ok()
        .filter(|s| !s.is_empty())
}

/// EVM: `eth_sendTransaction` from unlocked `DREGG_EVM_REFUND_FROM` to original `tx.from`.
/// BTC: `sendtoaddress` when dest is known.
fn attempt_rail_refund(rail: Rail, amount: u128, witness_raw: &[u8], btc_dest: Option<&str>) -> Option<String> {
    match rail {
        Rail::Eth | Rail::Base => {
            let rpc = if rail == Rail::Eth {
                std::env::var("ETH_RPC").ok()
            } else {
                std::env::var("BASE_RPC").ok()
            }?;
            let from = std::env::var("DREGG_EVM_REFUND_FROM").ok().filter(|s| s.starts_with("0x"))?;
            let hash = txhash_hex(witness_raw);
            let tx = http_json_local(
                &rpc,
                &serde_json::json!({
                    "jsonrpc":"2.0","id":1,
                    "method":"eth_getTransactionByHash",
                    "params":[hash]
                }),
            )
            .ok()?;
            let to = tx.pointer("/result/from").and_then(|x| x.as_str())?.to_string();
            let val = tx
                .pointer("/result/value")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("0x{amount:x}"));
            let sent = http_json_local(
                &rpc,
                &serde_json::json!({
                    "jsonrpc":"2.0","id":1,
                    "method":"eth_sendTransaction",
                    "params":[{
                        "from": from,
                        "to": to,
                        "value": val
                    }]
                }),
            )
            .ok()?;
            sent.pointer("/result").and_then(|x| x.as_str()).map(|s| s.to_string())
        }
        Rail::Btc => {
            let url = std::env::var("BTC_RPC").ok()?;
            let dest = btc_dest.filter(|s| s.starts_with("bcrt1") || s.starts_with("bc1") || s.starts_with("tb1"))?;
            let btc = (amount as f64) / 100_000_000.0;
            let v = http_json_local(
                &url,
                &serde_json::json!({
                    "jsonrpc":"1.0","id":"dregg",
                    "method":"sendtoaddress",
                    "params":[dest, format!("{btc:.8}")]
                }),
            )
            .ok()?;
            v.pointer("/result").and_then(|x| x.as_str()).map(|s| s.to_string())
        }
        _ => None,
    }
}

fn ingest_observed(
    fed: &Arc<Mutex<Fed>>,
    rail: Rail,
    amount: u128,
    raw: Vec<u8>,
    recipient: String,
) -> Result<(), String> {
    if confirm_rail(rail, &raw).is_err() {
        return Ok(());
    }
    let mut salt = [11u8; 32];
    salt.copy_from_slice(blake3::hash(&raw).as_bytes());
    let opening = DepositOpening {
        witness: ForeignWitness {
            rail,
            amount,
            raw,
            salt,
        },
    };
    let n = dregg::commit::nullifier(&opening);
    let mut f = fed.lock().unwrap();
    if !checkout_now(&f).checkout_ready {
        return Ok(());
    }
    let ck = Checkpoint::new(f.epoch, f.cell.cell_root(), [0u8; 32], "deposit");
    let _sigs: Vec<_> = f.operators.iter().map(|kp| sign(kp, &ck.digest)).collect();
    match f.cell.apply(Turn::DepositSeen { opening }) {
        Ok(_) => {}
        Err(dregg::cell::CellError::DoubleSpend)
        | Err(dregg::cell::CellError::CheckoutClosed) => {
            return Ok(());
        }
        Err(dregg::cell::CellError::Underpay) => {
            eprintln!(
                "zec ingest underpay: have {amount} need {}",
                rail.price_units()
            );
            return Ok(());
        }
        Err(e) => return Err(e.to_string()),
    }
    if f.cell.phase(&n) != Some(Phase::Deposited) {
        let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
        return Ok(());
    }
    let action = ActionOpening {
        collection: "svg".into(),
        svg_hash: [0u8; 32],
        recipient: recipient.clone(),
        session: n,
        salt: [2u8; 32],
    };
    let _ = apply_paid_qty(&mut f, &n, amount, rail);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = f.cell.apply(Turn::Tick { now });
    let _ = f.cell.apply(Turn::MintEntitled {
        n_deposit: n,
        action,
    });
    let _ = f.cell.apply(Turn::SweepStaleEntitlements {
        timeout_secs: checkout::mint_timeout_secs(),
    });
    f.by_recipient.insert(recipient.clone(), n);
    save_recipients(&f.recipients_path, &f.by_recipient);
    f.mempool.remove(&recipient);
    f.sessions.advance(&recipient, "entitled", "", amount as u64, "");
    let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
    Ok(())
}

fn parse_sigs(v: &serde_json::Value) -> Vec<OracleSig> {
    let mut out = Vec::new();
    if let Some(arr) = v.get("signatures").and_then(|x| x.as_array()) {
        for s in arr {
            if let (Some(p), Some(g)) = (s.get("pubkey"), s.get("signature")) {
                let mut pk = [0u8; 32];
                let mut sg = Vec::new();
                if let Ok(pb) = serde_json::from_value::<[u8; 32]>(p.clone()) {
                    pk = pb;
                } else if let Some(h) = p.as_str() {
                    if let Ok(b) = hex::decode(h) {
                        if b.len() == 32 {
                            pk.copy_from_slice(&b);
                        }
                    }
                }
                if let Ok(sb) = serde_json::from_value::<Vec<u8>>(g.clone()) {
                    sg = sb;
                } else if let Some(h) = g.as_str() {
                    if let Ok(b) = hex::decode(h) {
                        sg = b;
                    }
                }
                out.push(OracleSig {
                    pubkey: pk,
                    signature: sg,
                });
            }
        }
    }
    out
}

fn json_ckpt(ck: &Checkpoint) -> serde_json::Value {
    json!({
        "ok": true,
        "epoch": ck.epoch,
        "cell_root": hex::encode(ck.cell_root),
        "n_deposit": ck.n_deposit,
        "digest": hex::encode(ck.digest),
        "digest_bytes": ck.digest,
        "turn_tag": ck.turn_tag,
    })
}

fn parse_digest_body(body: &str) -> Result<[u8; 32], String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    parse_digest_value(v.get("digest").unwrap_or(&serde_json::Value::Null))
}

fn parse_digest_value(v: &serde_json::Value) -> Result<[u8; 32], String> {
    parse_n32(v).ok_or_else(|| "digest missing".into())
}

fn parse_n32(v: &serde_json::Value) -> Option<[u8; 32]> {
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        let b = hex::decode(s.trim_start_matches("0x")).ok()?;
        if b.len() != 32 {
            return None;
        }
        let mut a = [0u8; 32];
        a.copy_from_slice(&b);
        return Some(a);
    }
    serde_json::from_value::<[u8; 32]>(v.clone()).ok()
}

fn n_from_body(v: &serde_json::Value) -> [u8; 32] {
    v.get("n_deposit")
        .and_then(parse_n32)
        .unwrap_or([0u8; 32])
}

fn deposit_opening_from(v: &serde_json::Value) -> Option<dregg::commit::DepositOpening> {
    let rail: Rail = serde_json::from_value(v.get("rail")?.clone()).ok()?;
    let amount = v.get("amount").and_then(|x| x.as_u64()).map(|u| u as u128)?;
    let raw_hex = v.get("raw_hex")?.as_str()?;
    if raw_hex.trim() == "aa" || raw_hex.trim() == "0xaa" {
        return None;
    }
    let raw = hex::decode(raw_hex.trim_start_matches("0x")).unwrap_or_else(|_| raw_hex.as_bytes().to_vec());
    let mut salt = [11u8; 32];
    salt.copy_from_slice(blake3::hash(&raw).as_bytes());
    Some(dregg::commit::DepositOpening {
        witness: ForeignWitness {
            rail,
            amount,
            raw,
            salt,
        },
    })
}

fn tag_from_body(v: &serde_json::Value) -> Result<String, String> {
    if let Some(wq) = v.get("wasm_query").filter(|x| !x.is_null()) {
        return Ok(wasm_supply_tag(
            v.get("collection").and_then(|x| x.as_str()).unwrap_or("svg"),
            v.get("minted").and_then(|x| x.as_u64()).unwrap_or(0),
            v.get("max_supply").and_then(|x| x.as_u64()).unwrap_or(0),
            v.get("height").and_then(|x| x.as_u64()).unwrap_or(0),
            &serde_json::to_vec(wq).unwrap_or_default(),
        ));
    }
    v.get("turn_tag")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "turn_tag required".into())
}

fn n_for_body(v: &serde_json::Value) -> [u8; 32] {
    // Preview and apply must hash the same (epoch, cell_root, n, tag).
    // Observe-supply binds n to the wasm supply tag, never a silent n=0
    // while apply uses a deposit nullifier.
    if v.get("wasm_query").filter(|x| !x.is_null()).is_some() {
        if let Ok(tag) = tag_from_body(v) {
            return *blake3::hash(tag.as_bytes()).as_bytes();
        }
    }
    if let Some(op) = deposit_opening_from(v) {
        return dregg::commit::nullifier(&op);
    }
    n_from_body(v)
}

fn preview_ckpt(fed: &Fed, v: &serde_json::Value) -> Result<Checkpoint, String> {
    let tag = tag_from_body(v)?;
    let n = n_for_body(v);
    Ok(Checkpoint::new(fed.epoch, fed.cell.cell_root(), n, &tag))
}

fn require_qc_body(fed: &Fed, v: &serde_json::Value, sigs: &[OracleSig]) -> Result<Checkpoint, String> {
    let ck = preview_ckpt(fed, v)?;
    count_valid(&ck.digest, sigs, &fed.allowed, fed.threshold).map_err(|e| e.to_string())?;
    Ok(ck)
}

fn withdraw_allowlist() -> Vec<String> {
    let raw = std::env::var("DREGG_WITHDRAW_ZEC_UA").unwrap_or_default();
    let mut v: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("u1"))
        .collect();
    if v.is_empty() {
        v.push("u1uml44spc4nn732t6605wjcwu3wn9dla4aehpyrvnhssq2ew3dk8mqk5e9jhnndzfny7fkrkx5ape05nzf8gwruqykkvc27fm0v9ln4hfrwkmtfrs27r06xu0uc9eneh777le3m8362v7xaurgtkgxtspy5eqhgpzw6axjwgwdy5wjdjt".into());
    }
    v
}

fn admin_adr36_session(fed: &Arc<Mutex<Fed>>, body: &str) -> Result<serde_json::Value, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let challenge = v.get("challenge").and_then(|x| x.as_str()).unwrap_or("");
    let signer = v.get("signer").and_then(|x| x.as_str()).unwrap_or("");
    let chain_id = v
        .get("chain_id")
        .and_then(|x| x.as_str())
        .unwrap_or("morocco-1");
    let pk = v
        .pointer("/pub_key/value")
        .or_else(|| v.get("pub_key"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let sig = v
        .get("signature")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if challenge.is_empty() || signer.is_empty() || pk.is_empty() || sig.is_empty() {
        return Err("ADR-036 requires challenge, signer, pub_key, signature".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    {
        let mut f = fed.lock().unwrap();
        match f.admin_challenges.remove(challenge) {
            Some(exp) if exp >= now => {}
            _ => return Err("challenge expired or unknown".into()),
        }
    }
    // Sign-in plane: Keplr signs the raw challenge bytes (hex-decoded), not the withdraw sentence.
    let data = hex::decode(challenge).map_err(|_| "challenge hex".to_string())?;
    let terp = dregg::adr36::verify_keplr_arbitrary(chain_id, signer, &data, pk, sig)?;
    let path = std::env::var("DREGG_ADMIN_TERP1_PATH").unwrap_or_else(|_| {
        format!(
            "{}/.terp/dregg-admin-terp1",
            std::env::var("HOME").unwrap_or_else(|_| ".".into())
        )
    });
    let designated = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("terp1"));
    match designated {
        None => {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&path, terp.as_bytes()).map_err(|e| e.to_string())?;
            let _ = std::fs::set_permissions(
                &path,
                std::os::unix::fs::PermissionsExt::from_mode(0o600),
            );
        }
        Some(want) if want != terp => {
            return Err("Keplr address is not the designated admin".into());
        }
        Some(_) => {}
    }
    let token = match std::env::var("DREGG_ADMIN_TOKEN") {
        Ok(s) if !s.is_empty() => s,
        _ => terp.clone(),
    };
    Ok(json!({"ok": true, "token": token, "signer": terp, "mode": "adr36"}))
}


fn withdraw_history(limit: usize, before: Option<u64>) -> Result<serde_json::Value, String> {
    let ua = withdraw_allowlist().into_iter().next().unwrap_or_default();
    let path = std::env::temp_dir().join("svg-mint-withdraw-hist.py");
    std::fs::write(&path, include_str!("withdraw_hist.py")).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new("python3");
    cmd.arg(&path).arg(&ua).arg(limit.to_string());
    if let Some(b) = before {
        cmd.arg(b.to_string());
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

fn zcli_withdraw_zec(to: &str, zat: u64, lab_txid: Option<&str>) -> Result<String, String> {
    if !withdraw_allowlist().iter().any(|u| u == to) {
        return Err("withdraw dest is not the donation / allowlisted UA".into());
    }
    if zat == 0 {
        return Err("zat required".into());
    }
    let bin = zrt_bin();
    if !std::path::Path::new(&bin).is_file() {
        return Err("svg-mint-zrt missing".into());
    }
    let zat_s = zat.to_string();
    let lab = std::env::var("SVG_MINT_ZRT_LAB").ok().as_deref() == Some("1");
    let mut cmd = std::process::Command::new(&bin);
    if let Ok(dir) = std::env::var("SVG_MINT_ZRT_DIR") {
        if !dir.is_empty() {
            cmd.env("SVG_MINT_ZRT_DIR", dir);
        }
    }
    // Lab commit is not a broadcast. Production leaves this unset and uses `send`,
    // which refuses to mark notes spent while the prover is unlinked.
    if lab {
        let txid = lab_txid.unwrap_or("");
        if txid.len() != 64 || !txid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("lab withdraw needs a 64-hex txid; notes were not marked spent".into());
        }
        cmd.args(["lab-send", "--zat", &zat_s, "--address", to, "--txid", txid]);
    } else {
        cmd.args(["send", "--zat", &zat_s, "--address", to]);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!("svg-mint-zrt send failed: {} {}", stderr.trim(), stdout.trim()));
    }
    Ok(stdout.trim().to_string())
}

fn admin_zid_session(fed: &Arc<Mutex<Fed>>, body: &str) -> Result<serde_json::Value, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let challenge = v.get("challenge").and_then(|x| x.as_str()).unwrap_or("");
    let pk_hex = v.get("publicKey").or_else(|| v.get("public_key")).and_then(|x| x.as_str()).unwrap_or("");
    let sig_hex = v.get("signature").and_then(|x| x.as_str()).unwrap_or("");
    if challenge.is_empty() || pk_hex.is_empty() || sig_hex.is_empty() {
        return Err("challenge, publicKey, signature required".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    {
        let mut f = fed.lock().unwrap();
        match f.admin_challenges.remove(challenge) {
            Some(exp) if exp >= now => {}
            _ => return Err("challenge expired or unknown".into()),
        }
    }
    let pk_raw = hex::decode(pk_hex.trim_start_matches("0x")).map_err(|_| "publicKey hex".to_string())?;
    let sig_raw = hex::decode(sig_hex.trim_start_matches("0x")).map_err(|_| "signature hex".to_string())?;
    if pk_raw.len() != 32 || sig_raw.len() != 64 {
        return Err("ed25519 publicKey 32 bytes, signature 64 bytes".into());
    }
    let mut pk32 = [0u8; 32];
    pk32.copy_from_slice(&pk_raw);
    let mut sig64 = [0u8; 64];
    sig64.copy_from_slice(&sig_raw);
    let pk = PublicKey::from_bytes(&pk32).map_err(|_| "publicKey".to_string())?;
    let sig = Signature::from_bytes(&sig64).map_err(|_| "signature".to_string())?;
    let msg = hex::decode(challenge).map_err(|_| "challenge hex".to_string())?;
    pk.verify(&msg, &sig).map_err(|_| "zid signature".to_string())?;

    let path = std::env::var("DREGG_ADMIN_ZID_PK_PATH")
        .unwrap_or_else(|_| format!("{}/.terp/dregg-admin-zid.pk", std::env::var("HOME").unwrap_or_else(|_| ".".into())));
    let designated = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let pk_norm = hex::encode(pk32);
    match designated {
        None => {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&path, pk_norm.as_bytes()).map_err(|e| e.to_string())?;
            let _ = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
        }
        Some(want) if want.trim_start_matches("0x") != pk_norm => {
            return Err("zid public key is not the designated admin".into());
        }
        Some(_) => {}
    }
    let token = match std::env::var("DREGG_ADMIN_TOKEN") {
        Ok(s) if !s.is_empty() => s,
        _ => pk_norm.clone(),
    };
    Ok(json!({"ok": true, "token": token, "pubkey": pk_norm}))
}

fn designated_admin_terp1() -> Option<String> {
    let path = std::env::var("DREGG_ADMIN_TERP1_PATH").unwrap_or_else(|_| {
        format!(
            "{}/.terp/dregg-admin-terp1",
            std::env::var("HOME").unwrap_or_else(|_| ".".into())
        )
    });
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("terp1"))
}

fn admin_token_ok(got: Option<&str>) -> bool {
    let Some(g) = got else {
        return false;
    };
    let g = g.strip_prefix("Bearer ").unwrap_or(g).trim();
    if g.is_empty() {
        return false;
    }
    if let Ok(s) = std::env::var("DREGG_ADMIN_TOKEN") {
        if !s.is_empty() && g == s {
            return true;
        }
    }
    // ADR-036 session returns the designated terp1 when no env token is set.
    if designated_admin_terp1().as_deref() == Some(g) {
        return true;
    }
    false
}

fn adr36_fields(v: &serde_json::Value) -> Result<(&str, &str, &str, &str, &str), String> {
    let challenge = v.get("challenge").and_then(|x| x.as_str()).unwrap_or("");
    let signer = v.get("signer").and_then(|x| x.as_str()).unwrap_or("");
    let chain_id = v
        .get("chain_id")
        .and_then(|x| x.as_str())
        .unwrap_or("morocco-1");
    let pk = v
        .pointer("/pub_key/value")
        .or_else(|| v.get("pub_key"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let sig = v.get("signature").and_then(|x| x.as_str()).unwrap_or("");
    if challenge.is_empty() || signer.is_empty() || pk.is_empty() || sig.is_empty() {
        return Err("ADR-036 required: challenge, signer, pub_key, signature".into());
    }
    Ok((challenge, signer, chain_id, pk, sig))
}

fn consume_challenge(fed: &Arc<Mutex<Fed>>, challenge: &str) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut f = fed.lock().unwrap();
    match f.admin_challenges.remove(challenge) {
        Some(exp) if exp >= now => Ok(()),
        _ => Err("challenge expired or unknown".into()),
    }
}

fn require_designated(terp: &str) -> Result<String, String> {
    match designated_admin_terp1() {
        None => Err("no designated admin; ADR-036 session first".into()),
        Some(want) if want != terp => Err("Keplr address is not the designated admin".into()),
        Some(_) => Ok(terp.to_string()),
    }
}

/// Fresh ADR-036 over `statement`. Consumes the nonce (no replay).
fn admin_adr36_over(
    fed: &Arc<Mutex<Fed>>,
    v: &serde_json::Value,
    statement: &[u8],
) -> Result<String, String> {
    let (challenge, signer, chain_id, pk, sig) = adr36_fields(v)?;
    consume_challenge(fed, challenge)?;
    let terp = dregg::adr36::verify_keplr_arbitrary(chain_id, signer, statement, pk, sig)?;
    require_designated(&terp)
}

/// Fresh ADR-036 over a live challenge. Consumes the nonce (no replay).
fn admin_adr36_authorize(fed: &Arc<Mutex<Fed>>, v: &serde_json::Value) -> Result<String, String> {
    let challenge = v.get("challenge").and_then(|x| x.as_str()).unwrap_or("");
    let zat = v.get("zat").and_then(|x| x.as_u64()).unwrap_or(0);
    let statement = dregg::adr36::withdraw_statement(
        zat,
        dregg::adr36::DONATE_ZEC_UA,
        challenge,
    );
    admin_adr36_over(fed, v, statement.as_bytes())
}

fn checkout_action(
    fed: &Arc<Mutex<Fed>>,
    v: &serde_json::Value,
    terp: &str,
    action: &str,
    signer: String,
) -> (u16, serde_json::Value) {
    if action == "refund" {
        let mut g = fed.lock().unwrap();
        if g.sessions.refund(terp) {
            (
                200,
                json!({
                    "ok": true,
                    "authorized": true,
                    "minted": false,
                    "phase": "refunded",
                    "signer": signer,
                }),
            )
        } else {
            (
                404,
                json!({
                    "ok": false,
                    "authorized": true,
                    "minted": false,
                    "error": "no open unminted checkout for this address",
                    "signer": signer,
                }),
            )
        }
    } else {
        let tx = v.get("terp_tx").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
        let mut g = fed.lock().unwrap();
        let phase = g.sessions.open_row(terp).map(|r| r.phase.clone());
        if phase.as_deref() != Some("entitled") {
            (
                409,
                json!({
                    "ok": false,
                    "authorized": true,
                    "minted": false,
                    "phase": phase,
                    "error": "mint-to requires an entitled checkout that has not minted",
                    "signer": signer,
                }),
            )
        } else if tx.is_empty() {
            (
                200,
                json!({
                    "ok": true,
                    "authorized": true,
                    "minted": false,
                    "phase": "entitled",
                    "signer": signer,
                    "error": "The mint transaction has not been sent",
                }),
            )
        } else if g.sessions.record_chain_mint(terp, &tx) {
            (
                200,
                json!({
                    "ok": true,
                    "authorized": true,
                    "minted": true,
                    "phase": "settled",
                    "terp_tx": tx,
                    "signer": signer,
                }),
            )
        } else {
            (
                400,
                json!({
                    "ok": false,
                    "authorized": true,
                    "minted": false,
                    "phase": "entitled",
                    "error": "terp_tx must be the 64-hex mint hash. Status stays entitled.",
                    "signer": signer,
                }),
            )
        }
    }
}


fn status_for(fed: &Fed, rec: &str) -> serde_json::Value {
    let st = checkout_now(fed);
    let n = fed.by_recipient.get(rec);
    let mut phase = n.and_then(|n| fed.cell.phase(n)).map(|ph| format!("{ph:?}"));
    let mut mempool = serde_json::Value::Null;
    if phase.is_none() {
        if let Some(m) = fed.mempool.get(rec) {
            phase = Some("Mempool".into());
            mempool = m.clone();
        }
    }
    json!({
        "ok": true,
        "recipient": rec,
        "phase": phase,
        "mempool": mempool,
        "house_mempool_shielded": fed.house_mempool_txids.len(),
        "matched": phase.is_some(),
    })
}

fn write_phase_stream(mut stream: TcpStream, fed: Arc<Mutex<Fed>>, path: &str) -> std::io::Result<()> {
    let rec = urlencoding_decode(
        path.split("recipient=").nth(1).unwrap_or("").split('&').next().unwrap_or(""),
    );
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
    stream.write_all(header.as_bytes())?;
    stream.set_nonblocking(false)?;
    let mut last = String::new();
    for _ in 0..150 {
        let body = {
            let f = fed.lock().unwrap();
            status_for(&f, &rec).to_string()
        };
        if body != last {
            last = body.clone();
            stream.write_all(format!("data: {body}\n\n").as_bytes())?;
            stream.flush()?;
        } else {
            stream.write_all(b": ping\n\n")?;
            stream.flush()?;
        }
        if last.contains("\"phase\":\"Settled\"") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    Ok(())
}

fn handle(mut stream: TcpStream, fed: Arc<Mutex<Fed>>) -> std::io::Result<()> {
    let mut buf = [0u8; 65536];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let (head, body) = req.split_once("\r\n\r\n").unwrap_or((&req, ""));
    let first = head.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let admin_got = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        let k = k.trim().to_ascii_lowercase();
        if k == "authorization" || k == "x-dregg-admin" {
            Some(v.trim().to_string())
        } else {
            None
        }
    });

    if method == "GET" && path.starts_with("/v1/watch") {
        return write_phase_stream(stream, fed, path);
    }

    let (code, payload) = match (method, path) {
        ("OPTIONS", _) => (204, json!("")),
        ("GET", "/health") | ("GET", "/") => {
            let f = fed.lock().unwrap();
            let st = checkout_now(&f);
            let mut p = json!({
                "ok": st.checkout_ready,
                "service": "dregg-svg",
                "pay": st.checkout_ready,
            });
            if let serde_json::Value::Object(m) = &mut p {
                if let serde_json::Value::Object(s) = st.to_json() {
                    for (k, v) in s {
                        m.insert(k, v);
                    }
                }
            }
            // Liveness is always 200 so the sidecar probe can succeed before LCD funds.
            // Readiness is `ok` / `checkout_ready` / `pay` (all false until the minter is funded).
            (200, p)
        }
        ("GET", "/v1/prices") => {
            let f = fed.lock().unwrap();
            let st = checkout_now(&f);
            let mut p = prices::catalog();
            if let serde_json::Value::Object(m) = &mut p {
                m.insert("checkout_ready".into(), json!(st.checkout_ready));
            }
            (200, p)
        }
        (m, p) if m == "GET" && p.starts_with("/v1/query/paid") => {
            let q = p.splitn(2, '?').nth(1).unwrap_or("");
            let mut rec = String::new();
            let mut c_hex = String::new();
            for pair in q.split('&') {
                let mut it = pair.splitn(2, '=');
                let k = it.next().unwrap_or("");
                let v = it.next().unwrap_or("");
                if k == "recipient" {
                    rec = v.to_string();
                }
                if k == "c" {
                    c_hex = v.to_string();
                }
            }
            let f = fed.lock().unwrap();
            let mut paid = false;
            let mut phase = serde_json::Value::Null;
            let mut dest = rec.clone();
            if !c_hex.is_empty() {
                if let Some(c) = dregg::pedersen::parse_commit(c_hex.as_bytes()) {

                    if let Some(t) = dregg::store::dest_for_commit(&f.pending, &c) {
                        dest = t.clone();
                        if rec.is_empty() || rec == t {
                            if let Some(n) = f.by_recipient.get(&t) {
                                if let Some(ph) = f.cell.phase(n) {
                                    paid = paid_phase_name(ph).is_some();
                                    if let Some(name) = paid_phase_name(ph) {
                                        phase = json!(name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !paid && !rec.is_empty() {
                let (rows, _) = f.sessions.page(200, None);
                if let Some(row) = rows.iter().find(|r| {
                    r.terp == rec
                        && matches!(r.phase.as_str(), "deposited" | "entitled" | "settled")
                }) {
                    paid = true;
                    phase = json!(row.phase);
                }
            }
            if !paid && !rec.is_empty() {
                if let Some(n) = f.by_recipient.get(&rec) {
                    if let Some(ph) = f.cell.phase(n) {
                        paid = paid_phase_name(ph).is_some();
                        if let Some(name) = paid_phase_name(ph) {
                            phase = json!(name);
                        }
                    }
                }
            }
            (
                200,
                json!({
                    "ok": true,
                    "paid": paid,
                    "recipient": dest,
                    "phase": phase,
                }),
            )
        }
        ("GET", "/v1/admin/challenge") => {
            let mut nonce = [0u8; 32];
            use rand::RngCore;
            rand::rngs::OsRng.fill_bytes(&mut nonce);
            let challenge = hex::encode(nonce);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            {
                let mut f = fed.lock().unwrap();
                f.admin_challenges.retain(|_, exp| *exp > now);
                f.admin_challenges.insert(challenge.clone(), now + 120);
            }
            (
                200,
                json!({
                    "ok": true,
                    "challenge": challenge,
                    "statement": "Sign in to permissionless.money desk",
                    "exp": now + 120,
                }),
            )
        }
        ("POST", "/v1/admin/session") => {
            let kind = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("kind").and_then(|x| x.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            let r = if kind == "adr36" || body.contains("sign/MsgSignData") || body.contains("signer") {
                admin_adr36_session(&fed, body)
            } else {
                admin_zid_session(&fed, body)
            };
            match r {
                Ok(v) => (200, v),
                Err(e) => (401, json!({"ok": false, "error": e})),
            }
        }
        ("POST", "/v1/admin/checkout") => {
            let v = serde_json::from_str::<serde_json::Value>(body).unwrap_or(json!({}));
            let terp = v.get("terp").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            let action = v.get("action").and_then(|x| x.as_str()).unwrap_or("");
            if terp.is_empty() {
                (400, json!({"ok": false, "error": "terp required"}))
            } else if action != "refund" && action != "mint-to" {
                (400, json!({"ok": false, "error": "action is refund or mint-to"}))
            } else if adr36_fields(&v).is_err() {
                (401, json!({"ok": false, "authorized": false, "error": "ADR-036 required"}))
            } else {
                let challenge = v.get("challenge").and_then(|x| x.as_str()).unwrap_or("");
                match dregg::adr36::checkout_statement(action, &terp, challenge) {
                    Err(e) => (400, json!({"ok": false, "error": e})),
                    Ok(statement) => match admin_adr36_over(&fed, &v, statement.as_bytes()) {
                        Err(e) => (401, json!({"ok": false, "authorized": false, "error": e})),
                        Ok(signer) => checkout_action(&fed, &v, &terp, action, signer),
                    },
                }
            }
        }
        ("POST", "/v1/admin/mints/flush") => {
            if !admin_token_ok(admin_got.as_deref()) {
                (401, json!({"ok": false, "error": "admin"}))
            } else {
                let mut g = fed.lock().unwrap();
                let rows = g.sessions.export_once();
                (200, json!({"ok": true, "flushed": rows.len(), "sessions": rows}))
            }
        }
        ("POST", "/v1/admin/withdraw") => {
            match serde_json::from_str::<serde_json::Value>(body) {
                Ok(v) => match admin_adr36_authorize(&fed, &v) {
                    Err(e) => (401, json!({"ok": false, "error": e})),
                    Ok(signer) => {
                        let dest = dregg::adr36::DONATE_ZEC_UA.to_string();
                        let to = v.get("to").and_then(|x| x.as_str()).unwrap_or("");
                        let zat = v.get("zat").and_then(|x| x.as_u64()).unwrap_or(0);
                        let rail = v.get("rail").and_then(|x| x.as_str()).unwrap_or("zec");
                        if rail != "zec" {
                            (400, json!({"ok": false, "error": "only zec withdraw in this cut"}))
                        } else if !to.is_empty() && to != dest {
                            (
                                400,
                                json!({
                                    "ok": false,
                                    "error": "withdraw dest is not the donation / allowlisted UA",
                                    "withdraw_to": dest,
                                    "signer": signer,
                                }),
                            )
                        } else if zat == 0 {
                            (400, json!({"ok": false, "error": "zat required", "withdraw_to": dest}))
                        } else {
                            let lab_txid = v.get("lab_txid").and_then(|x| x.as_str());
                            match zcli_withdraw_zec(&dest, zat, lab_txid) {
                                Ok(out) => (
                                    200,
                                    json!({
                                        "ok": true,
                                        "authorized": true,
                                        "rail": "zec",
                                        "to": dest,
                                        "zat": zat,
                                        "signer": signer,
                                        "zcli": out,
                                    }),
                                ),
                                Err(e) => (
                                    400,
                                    json!({
                                        "ok": false,
                                        "authorized": true,
                                        "error": e,
                                        "to": dest,
                                        "zat": zat,
                                        "signer": signer,
                                    }),
                                ),
                            }
                        }
                    }
                },
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("GET", p) if p.starts_with("/v1/admin/stats") => {
            if !admin_token_ok(admin_got.as_deref()) {
                (401, json!({"ok": false, "error": "admin token required"}))
            } else {
                let (pending, mempool_n, recipients, minted, remaining, ready, reasons, hd) = {
                    let f = fed.lock().unwrap();
                    let st = checkout_now(&f);
                    (
                        f.pending.len(),
                        f.mempool.len(),
                        f.by_recipient.len(),
                        st.minted,
                        st.remaining_supply,
                        st.checkout_ready,
                        st.reasons.clone(),
                        f.hd_index,
                    )
                };
                let wallet = dregg::zec_wallet::query().to_json();
                let incoming: Vec<_> = {
                    let f = fed.lock().unwrap();
                    f.mempool.values().cloned().collect()
                };
                (
                    200,
                    json!({
                        "ok": true,
                        "pending": pending,
                        "mempool": mempool_n,
                        "incoming": incoming,
                        "recipients": recipients,
                        "minted": minted,
                        "remaining_supply": remaining,
                        "checkout_ready": ready,
                        "reasons": reasons,
                        "hd_index": hd,
                        "rails": {
                            "zec": {
                                "mempool": mempool_n,
                                "pending_checkouts": pending,
                            }
                        },
                        "withdraw_to": withdraw_allowlist(),
                        "zec_wallet": wallet,
                    }),
                )
            }
        }
        (m, p) if m == "GET" && p.starts_with("/v1/admin/withdraws") => {
            if !admin_token_ok(admin_got.as_deref()) {
                (401, json!({"ok": false, "error": "admin"}))
            } else {
                let limit = p.split("limit=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok()).unwrap_or(20usize);
                let before = p.split("before=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok());
                match withdraw_history(limit, before) {
                    Ok(v) => (200, v),
                    Err(e) => (400, json!({"ok": false, "error": e})),
                }
            }
        }
        (m, p) if m == "GET" && p.starts_with("/v1/admin/sessions") => {
            if !admin_token_ok(admin_got.as_deref()) {
                (401, json!({"ok": false, "error": "admin"}))
            } else {
                let limit = p.split("limit=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok()).unwrap_or(20usize);
                let before = p.split("before=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok());
                let f = fed.lock().unwrap();
                let (rows, next) = f.sessions.page(limit, before);
                (200, json!({"ok": true, "sessions": rows, "next": next}))
            }
        }
        ("GET", "/v1/admin/info") => {
            if !admin_token_ok(admin_got.as_deref()) {
                (401, json!({"ok": false, "error": "admin token required"}))
            } else {
                let (pending, mempool_n, minted, remaining, ready, reasons) = {
                    let f = fed.lock().unwrap();
                    let st = checkout_now(&f);
                    (f.pending.len(), f.mempool.len(), st.minted, st.remaining_supply, st.checkout_ready, st.reasons.clone())
                };
                let wallet = dregg::zec_wallet::query().to_json();
                let incoming: Vec<_> = {
                    let f = fed.lock().unwrap();
                    f.mempool.values().cloned().collect()
                };
                (
                    200,
                    json!({
                        "ok": true,
                        "pending": pending,
                        "mempool": mempool_n,
                        "incoming": incoming,
                        "minted": minted,
                        "remaining_supply": remaining,
                        "checkout_ready": ready,
                        "reasons": reasons,
                        "withdraw_to": withdraw_allowlist(),
                        "zec_wallet": wallet,
                    }),
                )
            }
        }
        (m, p) if m == "GET" && p.starts_with("/v1/status") => {
            let rec = urlencoding_decode(
                p.split("recipient=")
                    .nth(1)
                    .unwrap_or("")
                    .split('&')
                    .next()
                    .unwrap_or(""),
            );
            let f = fed.lock().unwrap();
            let st = checkout_now(&f);
            let n = f.by_recipient.get(&rec);
            let mut phase = n.and_then(|n| f.cell.phase(n)).map(|ph| format!("{ph:?}"));
            let qty = n.and_then(|n| f.cell.qty(n));
            let batches = qty
                .map(|q| checkout::mint_batches(q as u64, st.mint_batch_size))
                .unwrap_or_default();
            let mut mempool = serde_json::Value::Null;
            if phase.is_none() {
                if let Some(m) = f.mempool.get(&rec) {
                    phase = Some("Mempool".into());
                    mempool = m.clone();
                }
            }
            let mut p = json!({
                "ok": true,
                "recipient": rec,
                "n_deposit": n,
                "phase": phase,
                "qty": qty,
                "mint_batches": batches,
                "mempool": mempool,
                "house_mempool_shielded": f.house_mempool_txids.len(),
            });
            if let serde_json::Value::Object(m) = &mut p {
                if let serde_json::Value::Object(s) = st.to_json() {
                    for (k, v) in s {
                        m.insert(k, v);
                    }
                }
            }
            (200, p)
        }
        (m, p) if m == "GET" && p.starts_with("/v1/cancel") => {
            let rec = p
                .split("recipient=")
                .nth(1)
                .unwrap_or("")
                .split('&')
                .next()
                .unwrap_or("");
            let rec = urlencoding_decode(rec);
            if !rec.starts_with("terp1") {
                (
                    400,
                    json!({"ok": false, "error": "cancel requires ?recipient=terp1…"}),
                )
            } else {
                cancel_checkout(&fed, &rec)
            }
        }
        (m, p) if m == "GET" && p.starts_with("/v1/pay-targets") => {
            let rec = p
                .split("recipient=")
                .nth(1)
                .unwrap_or("")
                .split('&')
                .next()
                .unwrap_or("");
            let rec = urlencoding_decode(rec);
            if !rec.starts_with("terp1") {
                (
                    400,
                    json!({"ok": false, "error": "pay-targets requires ?recipient=terp1… over TLS; never put terp1 in the chain memo"}),
                )
            } else {
                let qty_raw = p.split("qty=").nth(1).and_then(|s| s.split('&').next());
                let qty = checkout::parse_qty(qty_raw);
                match issue_pay_target(&fed, &rec, qty) {
                    Ok(v) => (200, v),
                    Err(e) if e.starts_with("checkout closed") => (
                        503,
                        json!({"ok": false, "checkout_ready": false, "error": e}),
                    ),
                    Err(e) => (400, json!({"ok": false, "error": e})),
                }
            }
        }
        ("GET", p) if p.starts_with("/v1/checkpoint-preview") => {
            let tag = p.split("tag=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
            let nhex = p.split("n_deposit=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
            if tag.is_empty() || nhex.is_empty() {
                (400, json!({"ok": false, "error": "checkpoint-preview requires tag and n_deposit (same as apply)"}))
            } else {
                let n = parse_n32(&json!(nhex)).ok_or_else(|| "n_deposit".to_string());
                match n {
                    Ok(n) => {
                        let f = fed.lock().unwrap();
                        let ck = Checkpoint::new(f.epoch, f.cell.cell_root(), n, tag);
                        (200, json_ckpt(&ck))
                    }
                    Err(e) => (400, json!({"ok": false, "error": e})),
                }
            }
        }
        ("POST", "/v1/checkpoint-preview") => {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let f = fed.lock().unwrap();
            match preview_ckpt(&f, &v) {
                Ok(ck) => (200, json_ckpt(&ck)),
                Err(e) => (400, json!({"ok": false, "error": e})),
            }
        }
        ("GET", "/v1/operators") => {
            let f = fed.lock().unwrap();
            (
                200,
                json!({
                    "ok": true,
                    "threshold": f.threshold,
                    "pubkeys": f.allowed,
                }),
            )
        }
        ("POST", "/v1/collection") => {
            #[derive(Deserialize)]
            struct SetCol {
                id: String,
                max_supply: u64,
            }
            let mut v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            if v.get("turn_tag").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                v["turn_tag"] = json!("set-collection");
            }
            match serde_json::from_value::<SetCol>(v.clone()) {
                Ok(s) => {
                    let mut f = fed.lock().unwrap();
                    match require_qc_body(&f, &v, &sigs) {
                        Ok(_) => match f.cell.apply(Turn::SetCollection {
                            id: s.id,
                            max_supply: s.max_supply,
                        }) {
                            Ok(_) => {
                                let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                (200, json!({"ok": true}))
                            }
                            Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
                        },
                        Err(e) => (401, json!({"ok": false, "error": e})),
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        (m, p) if m == "GET" && p.starts_with("/v1/query/sold-out-refunds") => {
            let col = p
                .split("collection=")
                .nth(1)
                .unwrap_or("svg")
                .split('&')
                .next()
                .unwrap_or("svg");
            let q = fed.lock().unwrap().cell.query_sold_out_refunds(col);
            (
                200,
                json!({
                    "ok": true,
                    "sold_out": q.sold_out,
                    "minted": q.minted,
                    "max_supply": q.max_supply,
                    "refund_nullifiers": q.refund_nullifiers,
                    "action": "sold_out_refund",
                }),
            )
        }
        ("POST", "/v1/observe-supply") => {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            #[derive(Deserialize)]
            struct Ob {
                collection: String,
                minted: u64,
                max_supply: u64,
                height: u64,
                wasm_query: serde_json::Value,
            }
            match serde_json::from_value::<Ob>(v.clone()) {
                Ok(o) => {
                    let qj = serde_json::to_vec(&o.wasm_query).unwrap_or_default();
                    if qj.is_empty() || o.wasm_query.is_null() {
                        (400, json!({"ok": false, "error": "wasm query required"}))
                    } else {
                    let mut f = fed.lock().unwrap();
                    match require_qc_body(&f, &v, &sigs) {
                        Ok(ck) => match f.cell.apply(Turn::ObserveSupply {
                            collection: o.collection,
                            minted: o.minted,
                            max_supply: o.max_supply,
                            height: o.height,
                            receipt: ck.digest,
                        }) {
                            Ok(_) => {
                                let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                (200, json!({"ok": true, "receipt": ck.digest, "height": o.height, "wasm": true}))
                            }
                            Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
                        },
                        Err(e) => (401, json!({"ok": false, "error": e})),
                    }
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("POST", "/v1/query/sold-out-refund") => {
            let mut v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            if v.get("turn_tag").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                v["turn_tag"] = json!("sold-out-refund");
            }
            #[derive(Deserialize)]
            struct Q {
                collection: String,
            }
            match serde_json::from_value::<Q>(v.clone()) {
                Ok(q) => {
                    let mut f = fed.lock().unwrap();
                    match require_qc_body(&f, &v, &sigs) {
                        Ok(_) => match f.cell.query_and_refund_sold_out(&q.collection) {
                            Ok(a) => {
                                let pending = f.pending.clone();
                                for n in &a.refund_nullifiers {
                                    if let Some((rail, amt, raw)) = f.cell.refund_wire(n) {
                                        let dest = btc_dest_for(&pending, n);
                                        if let Some(txid) =
                                            attempt_rail_refund(rail, amt, &raw, dest.as_deref())
                                        {
                                            let bytes = hex::decode(txid.trim_start_matches("0x"))
                                                .unwrap_or_else(|_| txid.as_bytes().to_vec());
                                            let _ = f.books.apply(*n, RailPayout::Refund, bytes);
                                        } else {
                                            let rawh = blake3::hash(n).as_bytes().to_vec();
                                            let _ = f.books.apply(*n, RailPayout::Refund, rawh);
                                        }
                                    } else {
                                        let raw = blake3::hash(n).as_bytes().to_vec();
                                        let _ = f.books.apply(*n, RailPayout::Refund, raw);
                                    }
                                }
                                let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                (
                                    200,
                                    json!({
                                        "ok": true,
                                        "action": "sold_out_refund",
                                        "sold_out": a.sold_out,
                                        "minted": a.minted,
                                        "max_supply": a.max_supply,
                                        "refunded": a.refund_nullifiers.len(),
                                        "refund_nullifiers": a.refund_nullifiers,
                                    }),
                                )
                            }
                            Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
                        },
                        Err(e) => (401, json!({"ok": false, "error": e})),
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("POST", "/v1/refund") => {
            let mut v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            #[derive(Deserialize)]
            struct Rf {
                n_deposit: [u8; 32],
            }
            if v.get("turn_tag").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                v["turn_tag"] = json!("refund");
            }
            match serde_json::from_value::<Rf>(v.clone()) {
                Ok(r) => {
                    let mut f = fed.lock().unwrap();
                    match require_qc_body(&f, &v, &sigs) {
                        Ok(_) => match f.cell.apply(Turn::DepositRefunded {
                            n_deposit: r.n_deposit,
                        }) {
                            Ok(_) => {
                                let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                (200, json!({"ok": true, "refunded": true}))
                            }
                            Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
                        },
                        Err(e) => (401, json!({"ok": false, "error": e})),
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("POST", "/v1/deposit") => {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            #[derive(Deserialize)]
            struct DepositReq {
                rail: Rail,
                amount: u128,
                raw_hex: String,
            }
            let mut v = v;
            if v.get("turn_tag").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                v["turn_tag"] = json!("deposit");
            }
            match serde_json::from_value::<DepositReq>(v.clone()) {
                Ok(d) => {
                    if d.raw_hex.trim() == "aa" || d.raw_hex.trim() == "0xaa" {
                        (400, json!({"ok": false, "error": "dummy witness"}))
                    } else {
                        let opening = deposit_opening_from(&v).ok_or_else(|| "opening".to_string());
                        match opening {
                            Err(e) => (400, json!({"ok": false, "error": e})),
                            Ok(opening) => {
                        let raw = opening.witness.raw.clone();
                        if confirm_rail(d.rail, &raw).is_err() {
                            (400, json!({"ok": false, "error": "observer unconfirmed"}))
                        } else {
                        let n = dregg::commit::nullifier(&opening);
                        let c = dregg::commit::commit_deposit(&opening);
                        let mut f = fed.lock().unwrap();
                        if let Some(resp) = checkout_closed_http(&checkout_now(&f)) {
                            resp
                        } else {
                        match require_qc_body(&f, &v, &sigs) {
                            Ok(_) => match f.cell.apply(Turn::DepositSeen { opening }) {
                                Ok(_) => {
                                    let (qty, batches) =
                                        apply_paid_qty(&mut f, &n, d.amount, d.rail);
                                    let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                    (200, json!({
                                        "ok": true,
                                        "n_deposit": n,
                                        "c_deposit": c,
                                        "qty": qty,
                                        "mint_batches": batches,
                                    }))
                                }
                                Err(dregg::cell::CellError::CheckoutClosed) => {
                                    checkout_closed_http(&checkout_now(&f)).unwrap_or((
                                        503,
                                        json!({"ok": false, "checkout_ready": false}),
                                    ))
                                }
                                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
                            },
                            Err(e) => (401, json!({"ok": false, "error": e})),
                        }
                        }
                        }
                            }
                        }
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("POST", "/v1/payout") => {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            #[derive(Deserialize)]
            struct Pay {
                n_deposit: [u8; 32],
                kind: String,
                rail_hex: String,
            }
            let mut v = v;
            if v.get("turn_tag").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                v["turn_tag"] = json!("payout");
            }
            match serde_json::from_value::<Pay>(v.clone()) {
                Ok(p) => {
                    if p.rail_hex.trim() == "aa" || p.rail_hex.trim() == "0xaa" {
                        (400, json!({"ok": false, "error": "dummy rail payout"}))
                    } else {
                        let raw = hex::decode(p.rail_hex.trim_start_matches("0x"))
                            .unwrap_or_else(|_| p.rail_hex.as_bytes().to_vec());
                        let kind = if p.kind == "release" {
                            RailPayout::Release
                        } else {
                            RailPayout::Refund
                        };
                        let mut f = fed.lock().unwrap();
                        match require_qc_body(&f, &v, &sigs) {
                            Ok(_) => match f.books.apply(p.n_deposit, kind, raw) {
                                Ok(()) => {
                                    let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                    (200, json!({"ok": true, "payout": p.kind, "xor": true}))
                                }
                                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
                            },
                            Err(e) => (401, json!({"ok": false, "error": e})),
                        }
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("POST", "/v1/entitle") => {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            #[derive(Deserialize)]
            struct EntitleReq {
                n_deposit: [u8; 32],
                collection: String,
                svg_hash: [u8; 32],
                recipient: String,
                session: [u8; 32],
            }
            let mut v = v;
            if v.get("turn_tag").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                v["turn_tag"] = json!("entitle");
            }
            match serde_json::from_value::<EntitleReq>(v.clone()) {
                Ok(e) => {
                    let recipient = e.recipient.clone();
                    let n_ent = e.n_deposit;
                    let action = ActionOpening {
                        collection: e.collection,
                        svg_hash: e.svg_hash,
                        recipient: e.recipient,
                        session: e.session,
                        salt: [2u8; 32],
                    };
                    let mut f = fed.lock().unwrap();
                    if let Some(resp) = checkout_closed_http(&checkout_now(&f)) {
                        resp
                    } else {
                    match require_qc_body(&f, &v, &sigs) {
                        Ok(_) => {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let _ = f.cell.apply(Turn::Tick { now });
                            match f.cell.apply(Turn::MintEntitled {
                            n_deposit: e.n_deposit,
                            action,
                        }) {
                            Ok(_) => {
                                f.by_recipient.insert(recipient.clone(), n_ent);
                                save_recipients(&f.recipients_path, &f.by_recipient);
                                f.sessions.advance(&recipient, "entitled", "", 0, "");
                                let qty = f.cell.qty(&n_ent).unwrap_or(1);
                                let batches = checkout::mint_batches(
                                    qty as u64,
                                    checkout::mint_batch_size(),
                                );
                                let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                (200, json!({
                                    "ok": true,
                                    "qty": qty,
                                    "mint_batches": batches,
                                }))
                            }
                            Err(dregg::cell::CellError::CheckoutClosed) => {
                                checkout_closed_http(&checkout_now(&f)).unwrap_or((
                                    503,
                                    json!({"ok": false, "checkout_ready": false}),
                                ))
                            }
                            Err(err) => (400, json!({"ok": false, "error": err.to_string()})),
                        }
                        }
                        Err(e) => (401, json!({"ok": false, "error": e})),
                    }
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("POST", "/v1/settle") => {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
            let sigs = parse_sigs(&v);
            #[derive(Deserialize)]
            struct SettleReq {
                n_deposit: [u8; 32],
                token_id: String,
                terp_tx: String,
            }
            let mut v = v;
            if v.get("turn_tag").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                v["turn_tag"] = json!("settle");
            }
            match serde_json::from_value::<SettleReq>(v.clone()) {
                Ok(s) => {
                    let mut f = fed.lock().unwrap();
                    match require_qc_body(&f, &v, &sigs) {
                        Ok(ck) => match f.cell.apply(Turn::MintSettled {
                            n_deposit: s.n_deposit,
                            token_id: s.token_id,
                            terp_tx: s.terp_tx,
                            receipt: ck.digest,
                        }) {
                            Ok(pkt) => {
                                let _ = f.durable.save(&f.cell, &f.books, &f.pending, f.hd_index, f.bogo_seen);
                                (200, json!({"ok": true, "packet": pkt, "receipt": ck.digest}))
                            }
                            Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
                        },
                        Err(e) => (401, json!({"ok": false, "error": e})),
                    }
                }
                Err(e) => (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        ("POST", "/v1/sign") => {
            match parse_digest_body(body) {
                Ok(d) => {
                    let f = fed.lock().unwrap();
                    let sigs: Vec<_> = f.operators.iter().map(|kp| sign(kp, &d)).collect();
                    (200, json!({"ok": true, "signatures": sigs}))
                }
                Err(e) => (400, json!({"ok": false, "error": e})),
            }
        }
        _ => (404, json!({"ok": false, "error": "not found"})),
    };

    let mut body = payload.to_string();
    // 204 must not carry a body. Cloudflare turns "204 + Content-Length: 2" into HTTP 520,
    // which drops CORS and blocks the ops preflight.
    if code == 204 {
        body.clear();
    }
    let status = match code {
        200 => "200 OK".into(),
        204 => "204 No Content".into(),
        400 => "400 Bad Request".into(),
        401 => "401 Unauthorized".into(),
        404 => "404 Not Found".into(),
        409 => "409 Conflict".into(),
        n => format!("{n} Error"),
    };
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization, X-Dregg-Admin\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(resp.as_bytes())?;
    Ok(())
}
