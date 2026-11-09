//! Product ZEC wallet CLI for the SVG mint desk.
//!
//! Talks to the house compact server through zync-core. No zcash-devtool exec.
//! Compact server is required, loopback only, never zec.rocks / rotko / zidecar.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod wallet;

use zync_core::{LightwalletdClient, OrchardFvk, ScanAction, Scanner};

const DONATION_UA: &str = "u1uml44spc4nn732t6605wjcwu3wn9dla4aehpyrvnhssq2ew3dk8mqk5e9jhnndzfny7fkrkx5ape05nzf8gwruqykkvc27fm0v9ln4hfrwkmtfrs27r06xu0uc9eneh777le3m8362v7xaurgtkgxtspy5eqhgpzw6axjwgwdy5wjdjt";

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}

fn wallet_dir() -> PathBuf {
    if let Ok(s) = env::var("ZCASH_DEVTOOL_WALLET") {
        let s = s.trim();
        if !s.is_empty() {
            return PathBuf::from(s);
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".terp/svg-mint-zrt")
}

fn normalize_compact(raw: &str) -> Result<String, String> {
    let lower = raw.to_lowercase();
    if lower.contains("zec.rocks") || lower.contains("rotko.net") || lower.contains("zidecar") {
        return Err("compact server refuses public zidecar".into());
    }
    let mut s = raw.trim().to_string();
    let sl = s.to_lowercase();
    for p in ["grpc://", "https://", "http://"] {
        if let Some(rest) = sl.strip_prefix(p) {
            s = s[s.len() - rest.len()..].to_string();
            break;
        }
    }
    s = s.trim_end_matches('/').to_string();
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (h, p) = rest
            .split_once("]:")
            .ok_or("compact server host:port required")?;
        (h.to_string(), p.to_string())
    } else {
        let (h, p) = s
            .rsplit_once(':')
            .ok_or("compact server host:port required")?;
        (h.to_string(), p.to_string())
    };
    if port.parse::<u16>().is_err() {
        return Err("compact server port required".into());
    }
    let host_l = host.to_lowercase();
    if host_l != "127.0.0.1" && host_l != "localhost" && host_l != "::1" {
        return Err("compact server must be loopback".into());
    }
    if host_l == "::1" {
        Ok(format!("http://[::1]:{port}"))
    } else {
        Ok(format!("http://{host_l}:{port}"))
    }
}

fn compact_server() -> Result<String, String> {
    for name in ["ZTREAMER_GRPC", "ZCASH_DEVTOOL_SERVER"] {
        if let Ok(s) = env::var(name) {
            let s = s.trim().to_string();
            if s.is_empty() {
                continue;
            }
            return normalize_compact(&s);
        }
    }
    Err("ZTREAMER_GRPC or ZCASH_DEVTOOL_SERVER required".into())
}

fn donation_only(addr: &str) -> Result<(), String> {
    if addr != DONATION_UA {
        return Err("send refuses dest that is not the donation UA".into());
    }
    Ok(())
}

fn notes_path() -> PathBuf {
    wallet_dir().join("zrt-notes.json")
}

fn tip_path() -> PathBuf {
    wallet_dir().join("zrt-ironwood-height")
}

fn read_notes() -> serde_json::Value {
    fs::read_to_string(notes_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(|v: &serde_json::Value| v.get("notes").and_then(|n| n.as_array()).is_some())
        .unwrap_or_else(|| serde_json::json!({"notes": []}))
}

fn write_notes(v: &serde_json::Value) -> Result<(), String> {
    let dir = wallet_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(
        notes_path(),
        serde_json::to_string(v).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn read_tip() -> Option<u64> {
    fs::read_to_string(tip_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn write_tip(h: u64) -> Result<(), String> {
    let dir = wallet_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(tip_path(), h.to_string()).map_err(|e| e.to_string())
}

fn fvk_from_ufvk(s: &str) -> Result<OrchardFvk, String> {
    use zcash_address::unified::{Container, Encoding, Fvk, Ufvk};
    let (_, parsed) = Ufvk::decode(s.trim()).map_err(|e| format!("ufvk decode: {e}"))?;
    let bytes = parsed
        .items_as_parsed()
        .iter()
        .find_map(|item| match item {
            Fvk::Orchard(b) => Some(b),
            _ => None,
        })
        .ok_or("ufvk has no orchard component")?;
    Option::from(OrchardFvk::from_bytes(&bytes)).ok_or_else(|| "orchard fvk rejected".into())
}

fn ufvk_from_wallet() -> Result<Option<String>, String> {
    if let Ok(path) = env::var("SVG_MINT_UFVK_FILE") {
        if !path.is_empty() {
            let s = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Ok(Some(s));
            }
        }
    }
    let db = wallet_dir().join("data.sqlite");
    if !db.is_file() {
        return Ok(None);
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("select ufvk from accounts where ufvk is not null and length(ufvk) > 0 limit 1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    if let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let s: String = row.get(0).map_err(|e| e.to_string())?;
        return Ok(Some(s));
    }
    Ok(None)
}

fn load_fvk() -> Result<Option<OrchardFvk>, String> {
    if let Some(s) = ufvk_from_wallet()? {
        return Ok(Some(fvk_from_ufvk(&s)?));
    }
    let from_env = if let Ok(s) = env::var("SVG_MINT_ORCHARD_FVK") {
        let s = s.trim();
        if s.is_empty() {
            None
        } else if s.len() != 192 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("SVG_MINT_ORCHARD_FVK must be 96-byte hex".into());
        } else {
            Some(hex::decode(s).map_err(|_| "SVG_MINT_ORCHARD_FVK hex".to_string())?)
        }
    } else {
        None
    };
    let raw = match from_env {
        Some(b) => b,
        None => {
            let path = wallet_dir().join("orchard-fvk");
            match fs::read(&path) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(format!("orchard-fvk: {e}")),
            }
        }
    };
    if raw.len() != 96 {
        return Err("orchard fvk must be 96 bytes".into());
    }
    let mut bytes = [0u8; 96];
    bytes.copy_from_slice(&raw);
    let fvk = Option::from(OrchardFvk::from_bytes(&bytes)).ok_or("orchard fvk rejected")?;
    Ok(Some(fvk))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| die(&format!("runtime: {e}")))
}

const IRONWOOD_ACTIVATION: u64 = 3_428_143;

fn scan_budget() -> u64 {
    env::var("SVG_MINT_SCAN_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000)
}

fn push_ironwood_note(
    notes: &mut Vec<serde_json::Value>,
    txid: &str,
    receiver: &str,
    zat: u64,
    mined: bool,
) {
    // One tx can carry change and the checkout output. Keep each receiver.
    if let Some(existing) = notes.iter_mut().find(|n| {
        let same = n.get("txid").and_then(|t| t.as_str()) == Some(txid);
        let rec = n.get("receiver").and_then(|t| t.as_str()).unwrap_or("");
        same && (rec == receiver || rec.is_empty() || receiver.is_empty())
    }) {
        if mined {
            existing["mined"] = serde_json::json!(true);
        }
        if receiver.len() == 86 {
            existing["receiver"] = serde_json::json!(receiver);
        }
        let prev = existing.get("zat").and_then(|z| z.as_u64()).unwrap_or(0);
        if zat > prev {
            existing["zat"] = serde_json::json!(zat);
        }
        existing["pool"] = serde_json::json!("ironwood");
        return;
    }
    notes.push(serde_json::json!({
        "txid": txid,
        "address": "",
        "receiver": receiver,
        "memo": "",
        "zat": zat,
        "mined": mined,
        "pool": "ironwood",
    }));
}

fn decrypt_ironwood_tx(
    scanner: &Scanner,
    tx_height: u32,
    txid: &[u8; 32],
    actions: &[zync_core::client::CompactAction],
    notes: &mut Vec<serde_json::Value>,
) {
    let mined = tx_height > 0;
    let txid_hex = if txid.iter().any(|b| *b != 0) {
        hex::encode(txid)
    } else {
        String::new()
    };
    for (i, action) in actions.iter().enumerate() {
        if action.nullifier.iter().all(|b| *b == 0) && action.cmx.iter().all(|b| *b == 0) {
            continue;
        }
        let scan = ScanAction {
            nullifier: action.nullifier,
            cmx: action.cmx,
            ephemeral_key: action.ephemeral_key,
            enc_ciphertext: action.ciphertext.clone(),
            block_height: tx_height,
            position_in_block: i as u32,
        };
        let Some(note) = scanner.try_decrypt_ironwood(&scan) else {
            continue;
        };
        let raw = note.recipient.to_raw_address_bytes();
        let id = if txid_hex.is_empty() {
            hex::encode(note.nullifier)
        } else {
            txid_hex.clone()
        };
        push_ironwood_note(notes, &id, &hex::encode(raw), note.value, mined);
    }
}


fn house_rpc(method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = env::var("ZCASH_PRIVATE_RPC")
        .or_else(|_| env::var("ZCASHD_RPC"))
        .unwrap_or_else(|_| "http://127.0.0.1:8232".into());
    let cookie_path = env::var("ZCASH_PRIVATE_RPC_COOKIE").unwrap_or_else(|_| {
        format!(
            "{}/.terp/zakura.rpc.cookie",
            env::var("HOME").unwrap_or_else(|_| ".".into())
        )
    });
    let cookie = fs::read_to_string(&cookie_path)
        .map_err(|_| "house mempool: cookie unreadable".to_string())?;
    let cookie = cookie.trim();
    if cookie.is_empty() || !cookie.contains(':') {
        return Err("house mempool: cookie unreadable".into());
    }
    let body = serde_json::json!({"jsonrpc":"1.0","id":"zrt","method":method,"params":params});
    let out = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "8",
            "-u",
            cookie,
            "-H",
            "content-type: application/json",
            "-d",
            &body.to_string(),
            &url,
        ])
        .output()
        .map_err(|e| format!("house mempool: {e}"))?;
    if !out.status.success() {
        return Err("house mempool: rpc failed".into());
    }
    serde_json::from_slice(&out.stdout).map_err(|_| "house mempool: rpc json".into())
}

fn compact_from_verbose(action: &serde_json::Value) -> Option<zync_core::client::CompactAction> {
    let hex32 = |v: &serde_json::Value| -> Option<[u8; 32]> {
        let s = v.as_str()?;
        let raw = hex::decode(s).ok()?;
        if raw.len() != 32 {
            return None;
        }
        let mut b = [0u8; 32];
        b.copy_from_slice(&raw);
        Some(b)
    };
    let enc = hex::decode(action.get("encCiphertext")?.as_str()?).ok()?;
    if enc.len() < 52 {
        return None;
    }
    Some(zync_core::client::CompactAction {
        cmx: hex32(action.get("cmx")?)?,
        ephemeral_key: hex32(action.get("ephemeralKey")?)?,
        ciphertext: enc[..52].to_vec(),
        nullifier: hex32(action.get("nullifier")?)?,
    })
}

/// Verbose house mempool txs. Compact `GetMempoolTx` often omits Ironwood actions.
/// Trial-decrypt only. Orchard actions are ignored.
fn merge_house_rpc_mempool(scanner: &Scanner, notes: &mut Vec<serde_json::Value>) {
    let listed = match house_rpc("getrawmempool", &serde_json::json!([])) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let ids = listed
        .get("result")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut txs = 0u32;
    let mut owned = 0u32;
    for id in ids.iter().take(16) {
        let Some(txid) = id.as_str() else { continue };
        let Ok(tx) = house_rpc("getrawtransaction", &serde_json::json!([txid, 1])) else {
            continue;
        };
        let Some(actions) = tx
            .pointer("/result/ironwood/actions")
            .and_then(|x| x.as_array())
        else {
            continue;
        };
        let compact: Vec<_> = actions.iter().filter_map(compact_from_verbose).collect();
        let orchard_actions = tx
            .pointer("/result/orchard/actions")
            .and_then(|x| x.as_array())
            .map(|rows| rows.iter().filter_map(compact_from_verbose).collect::<Vec<_>>())
            .unwrap_or_default();
        if compact.is_empty() && orchard_actions.is_empty() {
            continue;
        }
        txs += 1;
        let mut hit = 0u32;
        let mut orchard_hit = 0u32;
        let mut rev_hit = 0u32;
        let mut why_parse = 0u32;
        let mut why_decrypt = 0u32;
        let mut why_cmx = 0u32;
        let probe = |action: &zync_core::client::CompactAction, rev: bool| {
            let flip = |b: [u8; 32]| {
                if !rev {
                    return b;
                }
                let mut x = b;
                x.reverse();
                x
            };
            ScanAction {
                nullifier: flip(action.nullifier),
                cmx: flip(action.cmx),
                ephemeral_key: flip(action.ephemeral_key),
                enc_ciphertext: action.ciphertext.clone(),
                block_height: 0,
                position_in_block: 0,
            }
        };
        for action in &compact {
            let why = scanner.explain_ironwood(&probe(action, false));
            match why {
                "ok" => hit += 1,
                "parse" => why_parse += 1,
                "cmx" => why_cmx += 1,
                _ => why_decrypt += 1,
            }
            if why != "ok" && scanner.try_decrypt_ironwood(&probe(action, true)).is_some() {
                rev_hit += 1;
            }
        }
        for action in &orchard_actions {
            if scanner.try_decrypt(&probe(action, false)).is_some()
                || scanner.try_decrypt(&probe(action, true)).is_some()
            {
                orchard_hit += 1;
            }
        }
        eprintln!(
            "house mempool tx ironwood_actions {} orchard_actions {} ironwood_hit {hit} rev {rev_hit} orchard_hit {orchard_hit} parse {why_parse} decrypt {why_decrypt} cmx {why_cmx}",
            compact.len(),
            orchard_actions.len()
        );
        let before = notes.len();
        let txid_bytes = match hex::decode(txid) {
            Ok(b) if b.len() == 32 => {
                let mut a = [0u8; 32];
                a.copy_from_slice(&b);
                a
            }
            _ => [0u8; 32],
        };
        decrypt_ironwood_tx(scanner, 0, &txid_bytes, &compact, notes);
        if notes.len() > before {
            owned += 1;
        }
    }
    eprintln!("house mempool: ironwood txs {txs} new notes {owned}");
}


fn confirm_mined_notes(notes: &mut Vec<serde_json::Value>) {
    let mut flipped = 0u32;
    for note in notes.iter_mut() {
        if note.get("mined").and_then(|m| m.as_bool()).unwrap_or(true) {
            continue;
        }
        let Some(txid) = note.get("txid").and_then(|t| t.as_str()) else {
            continue;
        };
        if txid.len() != 64 || !txid.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(tx) = house_rpc("getrawtransaction", &serde_json::json!([txid, 1])) else {
            continue;
        };
        let height = tx
            .pointer("/result/height")
            .and_then(|h| h.as_u64())
            .unwrap_or(0);
        if height > 0 {
            note["mined"] = serde_json::json!(true);
            flipped += 1;
        }
    }
    if flipped > 0 {
        eprintln!("house mempool: marked mined {flipped}");
    }
}

async fn sync_compact() -> Result<(), String> {
    let url = compact_server()?;
    let mut client = LightwalletdClient::connect(&url)
        .await
        .map_err(|e| format!("compact connect: {e}"))?;
    let info = client
        .get_lightd_info()
        .await
        .map_err(|e| format!("lightd info: {e}"))?;
    let vendor = info.vendor.to_lowercase();
    if vendor.contains("rotko") || vendor.contains("zec.rocks") || vendor.contains("zidecar") {
        return Err("compact server vendor refused".into());
    }
    let (tip, _) = client
        .get_latest_block()
        .await
        .map_err(|e| format!("latest block: {e}"))?;
    let fvk = load_fvk()?.ok_or("ironwood viewing key missing")?;
    let scanner = Scanner::from_fvk(&fvk);
    let mut doc = read_notes();
    let notes = doc
        .get_mut("notes")
        .and_then(|n| n.as_array_mut())
        .ok_or("notes array")?;
    notes.retain(|n| n.get("pool").and_then(|p| p.as_str()) != Some("orchard"));
    let mut cursor = read_tip().unwrap_or(IRONWOOD_ACTIVATION.saturating_sub(1));
    if cursor < IRONWOOD_ACTIVATION.saturating_sub(1) {
        cursor = IRONWOOD_ACTIVATION.saturating_sub(1);
    }
    let budget = scan_budget();
    let mut scanned = 0u64;
    while cursor < tip && scanned < budget {
        let start = cursor + 1;
        let end = start
            .saturating_add(999)
            .min(tip)
            .min(start.saturating_add(budget - scanned).saturating_sub(1));
        let txs = client
            .get_ironwood_block_range(start, end)
            .await
            .map_err(|e| format!("ironwood block range: {e}"))?;
        for tx in txs {
            decrypt_ironwood_tx(&scanner, tx.height, &tx.txid, &tx.actions, notes);
        }
        cursor = end;
        scanned = scanned.saturating_add(end - start + 1);
        write_tip(cursor)?;
    }
    match client.ironwood_mempool().await {
        Ok(txs) => {
            for tx in txs {
                decrypt_ironwood_tx(&scanner, 0, &tx.txid, &tx.actions, notes);
            }
        }
        Err(e) => eprintln!("ironwood mempool: {e}"),
    }
    merge_house_rpc_mempool(&scanner, notes);
    confirm_mined_notes(notes);
    write_notes(&doc)?;
    // Same scanner, same server, same keys: keep the house wallet state current.
    match wallet::sync(&mut client, &url, &fvk, tip).await {
        Ok(st) => eprintln!("house wallet synced through {} of {tip}", st.cursor),
        Err(e) => eprintln!("house wallet sync: {e}"),
    }
    eprintln!("synced tip {tip} through {cursor} ironwood");
    Ok(())
}


fn merge_wallet_list_tx(notes: &mut Vec<serde_json::Value>) {
    let bin = env::var("ZCASH_DEVTOOL_BIN")
        .unwrap_or_else(|_| "/home/returniflost/.terp/zcash-devtool".into());
    let wdir = wallet_dir().display().to_string();
    let out = match Command::new(&bin)
        .args(["wallet", "-w", &wdir, "list-tx"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("list-tx: {e}");
            return;
        }
    };
    if !out.status.success() {
        eprintln!(
            "list-tx failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for note in dregg::zec_notes::parse_list_tx(&text) {
        if !note.mined && note.memo.is_empty() && note.address.is_empty() {
            continue;
        }
        let txid = note.txid.clone();
        if let Some(existing) = notes.iter_mut().find(|n| n.get("txid").and_then(|t| t.as_str()) == Some(txid.as_str())) {
            if !note.address.is_empty() {
                existing["address"] = serde_json::json!(note.address);
            }
            if !note.memo.is_empty() {
                existing["memo"] = serde_json::json!(note.memo);
            }
            existing["mined"] = serde_json::json!(note.mined);
            existing["pool"] = serde_json::json!("ironwood");
            if note.zat > 0 {
                existing["zat"] = serde_json::json!(note.zat);
            }
            continue;
        }
        notes.push(serde_json::json!({
            "txid": note.txid,
            "address": note.address,
            "memo": note.memo,
            "zat": note.zat,
            "mined": note.mined,
            "pool": "ironwood",
            "receiver": note.receiver,
            "spent": false,
        }));
    }
}

fn cmd_balance() {
    // In-process house wallet state (kept current by the watcher's sync). No devtool.
    match wallet::load() {
        Some(st) => println!("{}", wallet::balance_json(&st)),
        None => die("house wallet not synced yet"),
    }
}

fn wallet_addresses(bin: &str, wdir: &str, sub: &str) -> Result<String, String> {
    let out = Command::new(bin)
        .args(["wallet", "-w", wdir, sub])
        .output()
        .map_err(|e| format!("{sub}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(format!("{sub} failed: {} {}", stderr.trim(), stdout.trim()));
    }
    if stdout.lines().any(|l| l.contains("Address:") && l.contains("u1")) {
        return Ok(stdout);
    }
    Err(format!("{sub} returned no unified address: {} {}", stderr.trim(), stdout.trim()))
}

fn ensure_fvk() -> Result<zync_core::OrchardFvk, String> {
    if let Some(fvk) = load_fvk()? {
        return Ok(fvk);
    }
    let dir = wallet_dir();
    if env::var("SVG_MINT_ZRT_ALLOW_NEW").ok().as_deref() != Some("1") {
        return Err(format!(
            "no house wallet at {}; refusing to create a new one (set SVG_MINT_ZRT_ALLOW_NEW=1 for first setup)",
            dir.display()
        ));
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    let seed_path = dir.join("spend.seed");
    let fvk_path = dir.join("orchard-fvk");
    if seed_path.is_file() {
        return Err("spend.seed exists without a viewing key; refusing to replace it".into());
    }
    let mut seed = [0u8; 32];
    {
        use std::io::Read;
        let mut src = fs::File::open("/dev/urandom").map_err(|e| format!("urandom: {e}"))?;
        src.read_exact(&mut seed).map_err(|e| format!("urandom: {e}"))?;
    }
    let sk = zync_core::SpendingKey::from_bytes(seed)
        .into_option()
        .ok_or("orchard spending key rejected")?;
    let fvk = zync_core::OrchardFvk::from(&sk);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    let mut seed_file = match opts.open(&seed_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            seed.fill(0);
            return load_fvk()?.ok_or_else(|| {
                "spend.seed exists without a viewing key; refusing to replace it".into()
            });
        }
        Err(e) => {
            seed.fill(0);
            return Err(e.to_string());
        }
    };
    use std::io::Write;
    if let Err(e) = seed_file.write_all(&seed) {
        seed.fill(0);
        let _ = fs::remove_file(&seed_path);
        return Err(e.to_string());
    }
    seed.fill(0);
    if let Err(e) = fs::write(&fvk_path, fvk.to_bytes()) {
        return Err(format!("viewing key write failed; spend.seed kept: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&seed_path, fs::Permissions::from_mode(0o600));
        let _ = fs::set_permissions(&fvk_path, fs::Permissions::from_mode(0o600));
    }
    eprintln!(
        "svg-mint-zrt created a viewing key at {}; the seed value is not logged",
        fvk_path.display()
    );
    Ok(fvk)
}

fn unified_address(fvk: &zync_core::OrchardFvk, index: u32) -> Result<String, String> {
    let address = fvk.address_at(index, zync_core::Scope::External);
    let raw = address.to_raw_address_bytes();
    use zcash_address::unified::{Address as UnifiedAddress, Encoding, Receiver};
    use zcash_protocol::consensus::NetworkType;
    let ua = UnifiedAddress::try_from_items(vec![Receiver::Orchard(raw)])
        .map_err(|_| "unified address rejected".to_string())?;
    let encoded = ua.encode(&NetworkType::Main);
    let (net, _) = UnifiedAddress::decode(&encoded)
        .map_err(|e| format!("unified address decode: {e}"))?;
    if net != NetworkType::Main || !encoded.starts_with("u1") {
        return Err("unified address is not a mainnet ua".into());
    }
    Ok(encoded)
}

fn cmd_addresses() {
    let index = env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let fvk = match ensure_fvk() {
        Ok(fvk) => fvk,
        Err(e) => die(&e),
    };
    match unified_address(&fvk, index) {
        Ok(ua) => println!("{ua}"),
        Err(e) => die(&e),
    }
}
fn cmd_send(args: &[String]) {
    let mut zat = 0u64;
    let mut addr = String::new();
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--zat" if i + 1 < args.len() => {
                zat = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--address" if i + 1 < args.len() => {
                addr = args[i + 1].clone();
                i += 2;
            }
            "--dry-run" => {
                dry = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if zat == 0 {
        die("send --zat <n> required");
    }
    if !addr.starts_with("u1") {
        die("send --address must be a unified address");
    }
    if let Err(e) = donation_only(&addr) {
        die(&e);
    }
    let url = compact_server().unwrap_or_else(|e| die(&e));
    let fvk = match load_fvk() {
        Ok(Some(f)) => f,
        Ok(None) => die("house viewing key missing"),
        Err(e) => die(&e),
    };
    let res = runtime().block_on(async {
        let mut client = LightwalletdClient::connect(&url).await.map_err(|e| format!("compact connect: {e}"))?;
        wallet::send(&mut client, &url, &fvk, &addr, zat, dry).await
    });
    match res {
        Ok(v) => println!("{v}"),
        Err(e) => die(&format!("send failed: {e}")),
    }
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        eprintln!(
            "svg-mint-zrt — product ZEC wallet for permissionless SVG mint\n\
             \n\
             Commands:\n\
               balance --json\n\
               sync\n\
               addresses\n\
               notes\n\
               send --zat <n> --address <u1…> [--dry-run]\n\
             \n\
             Compact server: ZTREAMER_GRPC or ZCASH_DEVTOOL_SERVER (required, loopback, never zec.rocks)."
        );
        std::process::exit(0);
    }
    let cmd = args.remove(0);
    match cmd.as_str() {
        "balance" => cmd_balance(),
        "sync" => {
            if let Err(e) = runtime().block_on(sync_compact()) {
                die(&e);
            }
        }
        "addresses" => cmd_addresses(),
        "notes" => {
            if let Err(e) = runtime().block_on(sync_compact()) {
                die(&e);
            }
            println!("{}", read_notes());
        }
        "send" => cmd_send(&args),
        "wallet-check" => {
            let url = compact_server().unwrap_or_else(|e| die(&e));
            let r = runtime().block_on(async {
                let mut c = LightwalletdClient::connect(&url).await.map_err(|e| format!("compact connect: {e}"))?;
                wallet::check_anchor(&mut c, &url).await
            });
            match r { Ok(v) => println!("{v}"), Err(e) => die(&e) }
        }
        other => die(&format!("unknown command {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_host_is_loopback_only() {
        assert!(normalize_compact("https://zec.rocks:443").is_err());
        assert!(normalize_compact("https://zcash.rotko.net:443").is_err());
        assert!(normalize_compact("https://zidecar.example:9067").is_err());
        assert!(normalize_compact("1.2.3.4:9067").is_err());
        assert_eq!(
            normalize_compact("127.0.0.1:9067").unwrap(),
            "http://127.0.0.1:9067"
        );
        assert_eq!(
            normalize_compact("grpc://localhost:9067").unwrap(),
            "http://localhost:9067"
        );
    }

    #[test]
    fn send_refuses_non_donation() {
        assert!(donation_only("u1notthedonation").is_err());
        assert!(donation_only(DONATION_UA).is_ok());
    }
}
