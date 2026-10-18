//! Halo2 epoch claim on a loopback terpd.
//!
//! Proves one canonical-body claim, serves the hash-market router on
//! 127.0.0.1, and pays that claim through SetEpochCampaign,
//! StageEpochBatch, and the hashmerchant sudo. The custody key stays
//! in this process. The paid note is leaf 0 of a 32,769-note
//! Poseidon-v1 suite tree (suite depth 16) on that leaf's real path.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const AMOUNT: u64 = 15;
const HRP: &str = "terp";
const DENOM_RAW: &str = "uterp";
const SIDECAR_CUTOFF: u64 = 50;
const CONTRACT_CUTOFF: u64 = 80;
const PENDING_HEIGHT: u64 = 10;
const EARLY_HEIGHT: u64 = 50;
const PAY_HEIGHT: u64 = 80;

struct Cfg {
    terpd: PathBuf,
    home: PathBuf,
    node: String,
    chain_id: String,
    wasm: PathBuf,
    wavs: PathBuf,
    recipient: String,
    owner_key: String,
    stranger_key: String,
    bind: String,
}

struct ClaimPub {
    ctx: [u8; 32],
    root: [u8; 32],
    denom: [u8; 32],
    nullifier: [u8; 32],
    digest: [u8; 32],
    destination: String,
    amount: u64,
    proof_hex: String,
    replay_proof_hex: String,
    message: String,
}

struct Step {
    name: &'static str,
    pass: bool,
    detail: String,
}

struct Report {
    steps: Vec<Step>,
}

impl Report {
    fn push(&mut self, name: &'static str, pass: bool, detail: impl Into<String>) {
        let detail = detail.into();
        eprintln!("step {name} pass={pass} {detail}");
        self.steps.push(Step { name, pass, detail });
    }

    fn counts(&self) -> (usize, usize) {
        let pass = self.steps.iter().filter(|s| s.pass).count();
        (pass, self.steps.len() - pass)
    }
}

fn random_32() -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut file = fs::File::open("/dev/urandom").expect("urandom");
    file.read_exact(&mut out).expect("urandom");
    out
}

fn load_custody_seed() -> anyhow::Result<[u8; 32]> {
    let path = std::env::var("CUSTODY_SEED_FILE")
        .map_err(|_| anyhow::anyhow!("CUSTODY_SEED_FILE is required"))?;
    let bytes = fs::read(&path)?;
    if bytes.len() != 32 {
        anyhow::bail!("custody seed is {} bytes", bytes.len());
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    Ok(seed)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let dig = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

fn denom_bytes() -> [u8; 32] {
    let mut hash = *blake3::hash(DENOM_RAW.as_bytes()).as_bytes();
    hash[31] &= 0x1f;
    hash
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn hex_bytes(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// Sorted pending preimage. Matches `pending_root_sorted` and the sidecar
/// `pending_root` (that one sorts before hashing).
fn pending_root_sorted(digests: &[[u8; 32]]) -> [u8; 32] {
    let mut ordered = digests.to_vec();
    ordered.sort();
    let mut pre = b"terp-headstash-pending-v1".to_vec();
    pre.extend_from_slice(&(ordered.len() as u32).to_le_bytes());
    for digest in &ordered {
        pre.extend_from_slice(digest);
    }
    sha256(&pre)
}

fn accepted_root(
    allocation: &[u8; 32],
    nullifier: &[u8; 32],
    denom: &[u8; 32],
    destination: &str,
    amount: u64,
) -> [u8; 32] {
    let mut pre = b"terp-headstash-accepted-v1".to_vec();
    pre.extend_from_slice(allocation);
    pre.extend_from_slice(&1u32.to_le_bytes());
    pre.extend_from_slice(nullifier);
    pre.extend_from_slice(denom);
    let dest = destination.as_bytes();
    pre.extend_from_slice(&(dest.len() as u16).to_le_bytes());
    pre.extend_from_slice(dest);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

fn attested_root(batch: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"headstash-batch".to_vec();
    pre.extend_from_slice(batch);
    pre.extend_from_slice(&1u64.to_le_bytes());
    sha256(&pre)
}

fn scrub(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let low = line.to_ascii_lowercase();
            let phrase = line.split_whitespace().count() >= 12
                && line
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_whitespace());
            !phrase
                && !low.contains("mnemonic")
                && !low.contains("priv_key")
                && !low.contains("private key")
        })
        .map(|line| {
            if line.len() > 400 {
                line[..400].to_string()
            } else {
                line.to_string()
            }
        })
        .take(8)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn env_required(name: &str) -> anyhow::Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("missing {name}"))
}

fn cfg_from_env() -> anyhow::Result<Cfg> {
    Ok(Cfg {
        terpd: PathBuf::from(env_required("TERPD")?),
        home: PathBuf::from(env_required("HOME_CHAIN")?),
        node: std::env::var("NODE").unwrap_or_else(|_| "tcp://127.0.0.1:36667".into()),
        chain_id: std::env::var("CHAIN_ID").unwrap_or_else(|_| "headstash-epoch-e2e".into()),
        wasm: PathBuf::from(env_required("WASM")?),
        wavs: PathBuf::from(env_required("WAVS")?),
        recipient: env_required("RECIPIENT_ADDR")?,
        owner_key: std::env::var("OWNER_KEY").unwrap_or_else(|_| "owner".into()),
        stranger_key: std::env::var("STRANGER_KEY").unwrap_or_else(|_| "stranger".into()),
        bind: std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:19080".into()),
    })
}

fn terpd_raw(cfg: &Cfg, args: &[String]) -> std::process::Output {
    let mut cmd = Command::new(&cfg.terpd);
    cmd.args(args);
    cmd.output().expect("terpd")
}

fn query(cfg: &Cfg, args: &[&str]) -> anyhow::Result<Value> {
    let mut owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    owned.extend([
        "--home".into(),
        cfg.home.display().to_string(),
        "--node".into(),
        cfg.node.clone(),
        "-o".into(),
        "json".into(),
    ]);
    let out = terpd_raw(cfg, &owned);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        anyhow::bail!(
            "query failed {}: {}",
            args.first().unwrap_or(&""),
            scrub(&format!("{text} {}", String::from_utf8_lossy(&out.stderr)))
        );
    }
    serde_json::from_str(&text).map_err(|err| anyhow::anyhow!("query json: {err}"))
}

fn as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn find_attr(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Array(items) => items.iter().find_map(|item| find_attr(item, key)),
        Value::Object(map) => {
            if map.get("key").and_then(Value::as_str) == Some(key) {
                if let Some(found) = map.get("value").and_then(Value::as_str) {
                    return Some(found.to_string());
                }
            }
            map.values().find_map(|child| find_attr(child, key))
        }
        _ => None,
    }
}

fn broadcast(cfg: &Cfg, from: &str, gas: u64, args: &[&str]) -> anyhow::Result<Value> {
    let mut owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    owned.extend([
        "--from".into(),
        from.into(),
        "--home".into(),
        cfg.home.display().to_string(),
        "--keyring-backend".into(),
        "test".into(),
        "--chain-id".into(),
        cfg.chain_id.clone(),
        "--node".into(),
        cfg.node.clone(),
        "--gas".into(),
        gas.to_string(),
        "--gas-prices".into(),
        "0.025uterp".into(),
        "-y".into(),
        "-b".into(),
        "sync".into(),
        "-o".into(),
        "json".into(),
    ]);
    let out = terpd_raw(cfg, &owned);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let synced: Value = serde_json::from_str(&text).unwrap_or(json!({
        "raw": scrub(&text),
        "err": scrub(&String::from_utf8_lossy(&out.stderr)),
    }));
    let Some(hash) = synced.get("txhash").and_then(Value::as_str) else {
        anyhow::bail!("no txhash: {}", scrub(&synced.to_string()));
    };
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(120) {
            anyhow::bail!("tx {hash} not committed");
        }
        let polled = query(cfg, &["query", "tx", hash]);
        match polled {
            Ok(value) if value.get("code").is_some() || value.get("txhash").is_some() => {
                return Ok(value);
            }
            _ => std::thread::sleep(Duration::from_secs(2)),
        }
    }
}

fn tx_code(value: &Value) -> u64 {
    as_u64(&value["code"]).unwrap_or(0)
}

fn balance_of(cfg: &Cfg, address: &str) -> anyhow::Result<u128> {
    let value = query(cfg, &["query", "bank", "balances", address])?;
    let mut total = 0u128;
    for coin in value["balances"].as_array().into_iter().flatten() {
        if coin["denom"].as_str() == Some(DENOM_RAW) {
            let amount = coin["amount"].as_str().unwrap_or("0");
            total += amount.parse::<u128>().unwrap_or(0);
        }
    }
    Ok(total)
}

fn chain_height(cfg: &Cfg) -> anyhow::Result<u64> {
    let url = cfg.node.replacen("tcp://", "http://", 1) + "/status";
    let out = Command::new("curl")
        .args(["-fsS", "--max-time", "3", &url])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("status {}", scrub(&String::from_utf8_lossy(&out.stderr)));
    }
    let value: Value = serde_json::from_slice(&out.stdout)?;
    value["result"]["sync_info"]["latest_block_height"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no height"))?
        .parse()
        .map_err(|err| anyhow::anyhow!("height: {err}"))
}

fn wait_height_at_least(cfg: &Cfg, target: u64) -> anyhow::Result<u64> {
    let start = Instant::now();
    loop {
        let height = chain_height(cfg).unwrap_or(0);
        if height >= target {
            return Ok(height);
        }
        if start.elapsed() > Duration::from_secs(90) {
            anyhow::bail!("chain stayed at {height}, wanted {target}");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn smart(cfg: &Cfg, contract: &str, query_msg: &Value) -> anyhow::Result<Value> {
    let raw = query(
        cfg,
        &[
            "query",
            "wasm",
            "contract-state",
            "smart",
            contract,
            &query_msg.to_string(),
        ],
    )?;
    let data = &raw["data"];
    if data.is_object() {
        return Ok(data.clone());
    }
    if let Some(text) = data.as_str() {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(text) {
            if let Ok(inner) = serde_json::from_slice::<Value>(&bytes) {
                return Ok(inner);
            }
        }
        if let Ok(inner) = serde_json::from_str::<Value>(text) {
            return Ok(inner);
        }
    }
    Ok(raw)
}

fn prove_claim_pub(recipient: &str) -> anyhow::Result<ClaimPub> {
    let denom = denom_bytes();
    let raw = headstash_claim::statement::decode_destination(HRP, recipient)
        .map_err(|_| anyhow::anyhow!("recipient is not canonical bech32"))?;
    let mut padded = [0u8; 32];
    padded[..20].copy_from_slice(&raw);
    let ctx = random_32();
    eprintln!("halo2 epoch claim");
    let bundle = headstash_epoch::prove_epoch_claim(recipient, padded, AMOUNT, ctx)
        .map_err(|err| anyhow::anyhow!("prove: {err}"))?;
    if bundle.destination != recipient {
        anyhow::bail!("claim destination is not the recipient");
    }
    if bundle.amount != AMOUNT {
        anyhow::bail!("amount {}", bundle.amount);
    }
    if bundle.ctx != ctx || bundle.proof.len() != bundle.replay_proof.len() {
        anyhow::bail!("replay proof length");
    }
    if bundle.proof == bundle.replay_proof {
        anyhow::bail!("replay proof matched the first proof");
    }
    eprintln!("halo2 proof bytes {}", bundle.proof.len());
    Ok(ClaimPub {
        ctx: bundle.ctx,
        root: bundle.root,
        denom,
        nullifier: bundle.nullifier,
        digest: sha256(&bundle.proof),
        destination: bundle.destination,
        amount: bundle.amount,
        proof_hex: hex_bytes(&bundle.proof),
        replay_proof_hex: hex_bytes(&bundle.replay_proof),
        message: bundle.message,
    })
}

fn campaign_body(claim: &ClaimPub, backend: &str, ctx: &[u8; 32], custody_pk: &[u8; 32]) -> Value {
    json!({
        "backend": backend,
        "ctx_hex": hex_bytes(ctx),
        "allocation_root_hex": hex_bytes(&claim.root),
        "bech32_hrp": HRP,
        "denom_hex": hex_bytes(&claim.denom),
        "proof_len": claim.proof_hex.len() / 2,
        "cutoff_height": SIDECAR_CUTOFF,
        "amounts": [{"amount": AMOUNT, "cap": 1}],
        "trusted_hex": [hex_bytes(custody_pk)],
        "trusted_verify_threshold": 1,
        "trusted_sign_threshold": 1,
        "max_rows": 4,
        "reserved_hex": [],
    })
}

fn claim_body(proof_hex: &str, message: &str, denom: &[u8; 32]) -> Value {
    json!({
        "claims": [{
            "proof_hex": proof_hex,
            "message": message,
            "denom_hex": hex_bytes(denom),
        }]
    })
}

fn error_text(value: &Value) -> String {
    value["error"]
        .as_str()
        .or_else(|| value["rejected"][0]["error"].as_str())
        .unwrap_or("")
        .to_string()
}

fn same_balance(before: u128, after: u128) -> bool {
    before == after
}

async fn http_json(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    body: Option<&Value>,
) -> anyhow::Result<(u16, Value)> {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url).json(body.unwrap()),
        _ => anyhow::bail!("method"),
    };
    let res = req
        .send()
        .await
        .map_err(|err| anyhow::anyhow!("{url}: {err}"))?;
    let status = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    let value = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
    Ok((status, value))
}

fn chain_campaign(claim: &ClaimPub, custody_pk: &[u8; 32]) -> Value {
    json!({
        "set_epoch_campaign": {
            "campaign": {
                "ctx": b64(&claim.ctx),
                "backend": "halo2",
                "cutoff_height": CONTRACT_CUTOFF,
                "bech32_hrp": HRP,
                "denom": b64(&claim.denom),
                "amounts": [{"amount": AMOUNT, "cap": 1}],
                "reserved": [],
                "trusted": [b64(custody_pk)],
                "trusted_verify_threshold": 1,
                "trusted_sign_threshold": 1,
                "max_rows": 4,
                "allocation_root": b64(&claim.root),
                "root_id": 0,
            }
        }
    })
}

fn stage_msg(digests: &[[u8; 32]], rows: &[Value]) -> Value {
    json!({
        "stage_epoch_batch": {
            "digests": digests.iter().map(|d| b64(d)).collect::<Vec<_>>(),
            "rows": rows,
        }
    })
}

fn payout_row(nullifier: &[u8; 32], denom: &[u8; 32], destination: &str, amount: u64) -> Value {
    json!({
        "nullifier": b64(nullifier),
        "denom": b64(denom),
        "destination": destination,
        "amount": amount,
    })
}

fn minted_attr(tx: &Value) -> String {
    find_attr(tx, "minted").unwrap_or_else(|| "-".into())
}

async fn run() -> anyhow::Result<Report> {
    if std::env::var("HASHMERCHANT_CHAIN_UID").ok().as_deref() != Some("headstash-claim") {
        anyhow::bail!("HASHMERCHANT_CHAIN_UID must be headstash-claim");
    }
    if std::env::var_os("HASHMERCHANT_ATTESTATIONS").is_some() {
        anyhow::bail!("HASHMERCHANT_ATTESTATIONS must be unset");
    }
    let cfg = cfg_from_env()?;
    if !cfg.bind.starts_with("127.0.0.1:") {
        anyhow::bail!("bind {} is not loopback", cfg.bind);
    }
    let mut report = Report { steps: Vec::new() };
    eprintln!("proving halo2 epoch claim");
    let claim = prove_claim_pub(&cfg.recipient)?;
    if claim.amount != AMOUNT {
        anyhow::bail!("amount {}", claim.amount);
    }
    let real_pending = pending_root_sorted(&[claim.digest]);
    let real_accepted = accepted_root(
        &claim.root,
        &claim.nullifier,
        &claim.denom,
        &claim.destination,
        claim.amount,
    );
    let real_attested = attested_root(&real_accepted);

    let mut custody_seed = load_custody_seed()?;
    let custody = SigningKey::from_bytes(&custody_seed);
    custody_seed.fill(0);
    let custody_pk = custody.verifying_key().to_bytes();

    let data_dir = cfg.home.join("hm");
    fs::create_dir_all(&data_dir)?;
    let state = hash_market::server::AppState::new(
        None::<hash_market::VoteExtensionHandler>,
        cfg.chain_id.clone(),
        data_dir,
        true,
    )?;
    state.headstash.write().await.install_signer(custody);
    let state = Arc::new(state);
    let app = hash_market::server::router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    let local = listener.local_addr()?;
    if !local.ip().is_loopback() {
        anyhow::bail!("listener {local} is not loopback");
    }
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            eprintln!("server stopped: {err}");
        }
    });
    let base = format!("http://{}", cfg.bind);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;
    let health = format!("{base}/ve/health");
    let start = Instant::now();
    loop {
        if client
            .get(&health)
            .send()
            .await
            .ok()
            .is_some_and(|r| r.status().is_success())
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            anyhow::bail!("sidecar health did not come up");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    wait_height_at_least(&cfg, 3)?;
    let stored = broadcast(
        &cfg,
        &cfg.owner_key,
        250_000_000,
        &["tx", "wasm", "store", &cfg.wasm.display().to_string()],
    )?;
    if tx_code(&stored) != 0 {
        anyhow::bail!(
            "store code {}: {}",
            tx_code(&stored),
            scrub(stored["raw_log"].as_str().unwrap_or(""))
        );
    }
    let code_id = find_attr(&stored, "code_id").ok_or_else(|| {
        anyhow::anyhow!(
            "no code_id: {}",
            scrub(stored["raw_log"].as_str().unwrap_or(""))
        )
    })?;
    eprintln!("stored code_id {code_id}");
    let wavs: Value = serde_json::from_str(&fs::read_to_string(&cfg.wavs)?)?;
    let init = json!({
        "genesis_root": b64(&claim.root),
        "genesis_label": "headstash-epoch-e2e",
        "token_strategy": {"existing_fungible": {"proof": b64(&claim.denom), "raw": DENOM_RAW}},
        "wavs": wavs,
    });
    let init_msg = init.to_string();
    fs::write(cfg.home.join("instantiate.json"), &init_msg)?;
    let owner_addr = {
        let out = terpd_raw(
            &cfg,
            &[
                "keys".into(),
                "show".into(),
                cfg.owner_key.clone(),
                "-a".into(),
                "--keyring-backend".into(),
                "test".into(),
                "--home".into(),
                cfg.home.display().to_string(),
            ],
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    if !owner_addr.starts_with("terp1") {
        anyhow::bail!("owner address missing");
    }
    let instantiated = broadcast(
        &cfg,
        &cfg.owner_key,
        80_000_000,
        &[
            "tx",
            "wasm",
            "instantiate",
            &code_id,
            &init_msg,
            "--label",
            "headstash-epoch-e2e",
            "--admin",
            &owner_addr,
            "--amount",
            "1000000uterp",
        ],
    )?;
    if tx_code(&instantiated) != 0 {
        anyhow::bail!(
            "instantiate {}: {}",
            tx_code(&instantiated),
            scrub(&instantiated.to_string())
        );
    }
    let contract = find_attr(&instantiated, "_contract_address")
        .ok_or_else(|| anyhow::anyhow!("no contract: {}", scrub(&instantiated.to_string())))?;
    eprintln!("contract {contract}");
    let campaign_msg = chain_campaign(&claim, &custody_pk).to_string();
    fs::write(cfg.home.join("campaign.json"), &campaign_msg)?;
    let set = broadcast(
        &cfg,
        &cfg.owner_key,
        5_000_000,
        &["tx", "wasm", "execute", &contract, &campaign_msg],
    )?;
    if tx_code(&set) != 0 {
        anyhow::bail!(
            "set campaign {}: {}",
            tx_code(&set),
            scrub(&set.to_string())
        );
    }
    let registered = broadcast(
        &cfg,
        &cfg.owner_key,
        3_000_000,
        &[
            "tx",
            "hashmerchant",
            "register-contract",
            &contract,
            "headstash-claim",
            "bank",
            "1000000uterp",
        ],
    )?;
    if tx_code(&registered) != 0 {
        anyhow::bail!(
            "register {}: {}",
            tx_code(&registered),
            scrub(&registered.to_string())
        );
    }

    let baseline = balance_of(&cfg, &cfg.recipient)?;
    eprintln!("baseline {baseline}");

    let campaign_url = format!("{base}/ve/headstash/campaign");
    let claims_url = format!("{base}/ve/headstash/claims");
    let seal_url = format!("{base}/ve/headstash/seal");
    let vote_url = format!("{base}/vote-extension");

    let mut wrong_ctx = claim.ctx;
    wrong_ctx[0] ^= 0x01;
    let (status, _body) = http_json(
        &client,
        "POST",
        &campaign_url,
        Some(&campaign_body(&claim, "halo2", &wrong_ctx, &custody_pk)),
    )
    .await?;
    let (cstatus, cbody) = http_json(
        &client,
        "POST",
        &claims_url,
        Some(&claim_body(&claim.proof_hex, &claim.message, &claim.denom)),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "wrong_ctx",
        status == 200
            && cstatus == 200
            && error_text(&cbody).contains("context does not match")
            && same_balance(baseline, after),
        format!(
            "install {status} claim {cstatus} err {}",
            error_text(&cbody)
        ),
    );

    let mut wrong_root = campaign_body(&claim, "halo2", &claim.ctx, &custody_pk);
    let mut flipped = claim.root;
    flipped[0] ^= 0xff;
    wrong_root["allocation_root_hex"] = json!(hex_bytes(&flipped));
    let (status, _body) = http_json(&client, "POST", &campaign_url, Some(&wrong_root)).await?;
    let (cstatus, cbody) = http_json(
        &client,
        "POST",
        &claims_url,
        Some(&claim_body(&claim.proof_hex, &claim.message, &claim.denom)),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "wrong_root",
        status == 200
            && cstatus == 200
            && error_text(&cbody).contains("allocation root does not match")
            && same_balance(baseline, after),
        format!(
            "install {status} claim {cstatus} err {}",
            error_text(&cbody)
        ),
    );

    let (status, body) = http_json(
        &client,
        "POST",
        &campaign_url,
        Some(&campaign_body(&claim, "halo2", &claim.ctx, &custody_pk)),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "halo2_campaign",
        status == 200 && same_balance(baseline, after),
        format!("status {status} {}", error_text(&body)),
    );
    if status != 200 {
        return Ok(report);
    }

    let signed = json!({
        "claims": [{
            "proof_hex": claim.proof_hex,
            "message": claim.message,
            "signature_hex": "00",
            "leaf_x_hex": "00",
            "denom_hex": hex_bytes(&claim.denom),
        }]
    });
    let (status, body) = http_json(&client, "POST", &claims_url, Some(&signed)).await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "signed_body_refused",
        status == 400
            && error_text(&body).contains("claim body")
            && same_balance(baseline, after),
        format!("status {status} err {}", error_text(&body)),
    );

    let mut parts: Vec<String> = claim.message.split(':').map(str::to_string).collect();
    if parts.len() != 6 {
        anyhow::bail!("message fields {}", parts.len());
    }
    parts[5] = "16".into();
    let wrong_amount = parts.join(":");
    let (status, body) = http_json(
        &client,
        "POST",
        &claims_url,
        Some(&claim_body(&claim.proof_hex, &wrong_amount, &claim.denom)),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "wrong_amount",
        status == 200
            && error_text(&body).contains("amount is not in the published set")
            && same_balance(baseline, after),
        format!("err {}", error_text(&body)),
    );

    parts[5] = AMOUNT.to_string();
    parts[4] = format!("{HRP}1not-a-bech32-address");
    let wrong_dest = parts.join(":");
    let (status, body) = http_json(
        &client,
        "POST",
        &claims_url,
        Some(&claim_body(&claim.proof_hex, &wrong_dest, &claim.denom)),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "wrong_destination",
        status == 200
            && error_text(&body).contains("destination is not canonical bech32")
            && same_balance(baseline, after),
        format!("err {}", error_text(&body)),
    );

    let (status, body) = http_json(
        &client,
        "POST",
        &claims_url,
        Some(&claim_body(&claim.proof_hex, &claim.message, &claim.denom)),
    )
    .await?;
    let accepted = body["accepted"]
        .as_array()
        .map(|rows| rows.len())
        .unwrap_or(0);
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "good_claim",
        status == 200
            && accepted == 1
            && body["rejected"]
                .as_array()
                .is_some_and(|rows| rows.is_empty())
            && same_balance(baseline, after),
        format!("accepted {accepted} err {} bal {after}", error_text(&body)),
    );

    let (status, body) = http_json(
        &client,
        "POST",
        &claims_url,
        Some(&claim_body(&claim.proof_hex, &claim.message, &claim.denom)),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "replay_digest",
        status == 200
            && error_text(&body).contains("proof digest is already")
            && same_balance(baseline, after),
        format!("err {}", error_text(&body)),
    );

    let (status, body) = http_json(
        &client,
        "POST",
        &claims_url,
        Some(&claim_body(
            &claim.replay_proof_hex,
            &claim.message,
            &claim.denom,
        )),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "replay_nullifier",
        status == 200
            && error_text(&body).contains("nullifier is already spent")
            && same_balance(baseline, after),
        format!("err {}", error_text(&body)),
    );

    let mut other_ctx = random_32();
    if other_ctx == claim.ctx {
        other_ctx[1] ^= 0xff;
    }
    let (status, body) = http_json(
        &client,
        "POST",
        &campaign_url,
        Some(&campaign_body(&claim, "halo2", &other_ctx, &custody_pk)),
    )
    .await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "second_campaign_blocked",
        status == 409 && same_balance(baseline, after),
        format!("status {status} {}", error_text(&body)),
    );

    let real_row = payout_row(
        &claim.nullifier,
        &claim.denom,
        &claim.destination,
        claim.amount,
    );
    let mut stage_msg_text = stage_msg(&[claim.digest], &[real_row.clone()]).to_string();
    fs::write(cfg.home.join("stage.json"), &stage_msg_text)?;
    let stranger = broadcast(
        &cfg,
        &cfg.stranger_key,
        3_000_000,
        &["tx", "wasm", "execute", &contract, &stage_msg_text],
    )?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "non_owner_stage",
        tx_code(&stranger) != 0 && same_balance(baseline, after),
        format!("code {}", tx_code(&stranger)),
    );

    async fn seal_at(client: &reqwest::Client, url: &str, height: u64) -> anyhow::Result<Value> {
        let (status, body) =
            http_json(client, "POST", url, Some(&json!({"height": height}))).await?;
        if status != 200 {
            anyhow::bail!("seal {height} status {status} {body}");
        }
        Ok(body)
    }

    let pending_vote = seal_at(&client, &seal_url, PENDING_HEIGHT).await?;
    let pending_value = pending_vote["attestations"][0]["value"]
        .as_str()
        .unwrap_or("");
    let vote_now = http_json(&client, "GET", &vote_url, None).await?;
    if vote_now.0 != 200 {
        anyhow::bail!("vote-extension {}", vote_now.0);
    }
    let stored_pending = wait_epoch(&cfg, &contract, "headstash-pending", None).await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "pending_sudo",
        pending_vote["algo"].as_str() == Some("headstash-pending")
            && pending_value == hex_bytes(&real_pending)
            && stored_pending.0 == PENDING_HEIGHT
            && stored_pending.1 == b64(&attested_root(&real_pending))
            && same_balance(baseline, after),
        format!("height {} bal {after}", stored_pending.0),
    );

    let early = seal_at(&client, &seal_url, EARLY_HEIGHT).await?;
    let stored_early =
        wait_epoch(&cfg, &contract, "headstash-accepted", Some(EARLY_HEIGHT)).await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    let early_ok = early["algo"].as_str() == Some("headstash-accepted")
        && early["foreign_height"].as_u64() == Some(EARLY_HEIGHT)
        && stored_early.0 == EARLY_HEIGHT
        && stored_early.1 == b64(&real_attested)
        && same_balance(baseline, after);
    report.push(
        "accepted_root_below_cutoff",
        early_ok,
        format!("stored_height {} bal {after}", stored_early.0),
    );

    let staged = broadcast(
        &cfg,
        &cfg.owner_key,
        5_000_000,
        &["tx", "wasm", "execute", &contract, &stage_msg_text],
    )?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "accepted_sudo_before_cutoff",
        tx_code(&staged) == 0 && minted_attr(&staged) == "0" && same_balance(baseline, after),
        format!(
            "code {} minted {} bal {after}",
            tx_code(&staged),
            minted_attr(&staged)
        ),
    );

    let mut bad_nf = [0xabu8; 32];
    if bad_nf == claim.nullifier {
        bad_nf[0] = 0xcd;
    }
    let mut bad_digest = [0x22u8; 32];
    if bad_digest == claim.digest {
        bad_digest[0] = 0x44;
    }
    let bad_row = payout_row(&bad_nf, &claim.denom, &claim.destination, claim.amount);
    stage_msg_text = stage_msg(&[bad_digest], &[bad_row]).to_string();
    fs::write(cfg.home.join("stage.json"), &stage_msg_text)?;
    let mismatched = broadcast(
        &cfg,
        &cfg.owner_key,
        5_000_000,
        &["tx", "wasm", "execute", &contract, &stage_msg_text],
    )?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "restage_mismatch_below_cutoff",
        tx_code(&mismatched) == 0
            && minted_attr(&mismatched) == "0"
            && same_balance(baseline, after),
        format!("minted {}", minted_attr(&mismatched)),
    );

    let paying_vote = seal_at(&client, &seal_url, PAY_HEIGHT).await?;
    let stored_pay = wait_epoch(&cfg, &contract, "headstash-accepted", Some(PAY_HEIGHT)).await?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    report.push(
        "mismatched_stage_sudo_pays_nothing",
        paying_vote["algo"].as_str() == Some("headstash-accepted")
            && paying_vote["root"].as_str() == Some(hex_bytes(&real_attested).as_str())
            && stored_pay.0 == PAY_HEIGHT
            && stored_pay.1 == b64(&real_attested)
            && same_balance(baseline, after),
        format!("stored {} bal {after}", stored_pay.0),
    );

    stage_msg_text = stage_msg(&[claim.digest], &[real_row]).to_string();
    fs::write(cfg.home.join("stage.json"), &stage_msg_text)?;
    let paid = broadcast(
        &cfg,
        &cfg.owner_key,
        5_000_000,
        &["tx", "wasm", "execute", &contract, &stage_msg_text],
    )?;
    let after = balance_of(&cfg, &cfg.recipient)?;
    let delta = after.saturating_sub(baseline);
    report.push(
        "accepted_sudo_pays_exact_amount",
        tx_code(&paid) == 0 && minted_attr(&paid) == "1" && delta == u128::from(AMOUNT),
        format!("minted {} delta {delta}", minted_attr(&paid)),
    );

    Ok(report)
}

async fn wait_epoch(
    cfg: &Cfg,
    contract: &str,
    algo: &str,
    min_height: Option<u64>,
) -> anyhow::Result<(u64, String)> {
    let start = Instant::now();
    let query_msg = json!({"epoch_root": {"algo": algo}});
    loop {
        if let Ok(value) = smart(cfg, contract, &query_msg) {
            let height = as_u64(&value["height"]).unwrap_or(0);
            let root = value["root"].as_str().unwrap_or("").to_string();
            if !root.is_empty() && min_height.is_none_or(|want| height >= want) {
                return Ok((height, root));
            }
        }
        if start.elapsed() > Duration::from_secs(90) {
            anyhow::bail!("epoch root {algo} did not arrive");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(report) => {
            let (pass, fail) = report.counts();
            let steps: Vec<Value> = report
                .steps
                .iter()
                .map(|step| {
                    json!({
                        "name": step.name,
                        "pass": step.pass,
                        "detail": step.detail,
                    })
                })
                .collect();
            let out = json!({
                "passed": pass,
                "failed": fail,
                "steps": steps,
            });
            let path = std::env::var("RESULTS")
                .unwrap_or_else(|_| "/tmp/headstash-epoch-e2e-results.json".into());
            let _ = fs::write(
                &path,
                serde_json::to_string_pretty(&out).unwrap_or_default(),
            );
            println!("E2E_RESULT passed={pass} failed={fail}");
            if fail != 0 {
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("e2e error: {err:#}");
            std::process::exit(1);
        }
    }
}
