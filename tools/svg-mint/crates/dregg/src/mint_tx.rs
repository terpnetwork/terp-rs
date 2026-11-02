//! Runtime mint signer. One secp256k1 key file. No `terpd`, no mnemonic print.
//!
//! The file is `~/.terp/dregg-svg-runtime.sk` (override `DREGG_RUNTIME_SK_FILE`):
//! 32 raw bytes, mode `0600`, written once. A short file exits the load and is
//! not replaced. `DREGG_RUNTIME_NO_CREATE=1` refuses to write a missing file.
//! The public `terp1` is the address to fund with `uthiol` for gas.
//! Mint executes carry no funds. Code 98 does not keep a runtime-minter bypass.
//! A collection admits this signer only when `DREGG_PAYMINT_PROOFS_FILE` has
//! that collection's whitelist `alloc` and `proof_hashes`.

use k256::ecdsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
use k256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MSG_TYPE: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const PK_TYPE: &str = "/cosmos.crypto.secp256k1.PubKey";

pub fn key_path() -> PathBuf {
    if let Ok(p) = std::env::var("DREGG_RUNTIME_SK_FILE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".terp/dregg-svg-runtime.sk")
}

/// Address already on disk. Does not create a key.
pub fn address_if_present() -> Option<String> {
    let raw = std::fs::read(key_path()).ok()?;
    if raw.len() != 32 {
        return None;
    }
    address_of(&raw).ok()
}

/// Load the key file or write it once. Returns the `terp1` only.
pub fn ensure_address() -> Result<String, String> {
    let raw = load_or_create_secret()?;
    address_of(&raw)
}

fn address_of(raw: &[u8]) -> Result<String, String> {
    let sk = SigningKey::from_slice(raw).map_err(|e| e.to_string())?;
    let pk = sk.verifying_key().to_sec1_point(true);
    crate::adr36::terp1_from_secp256k1_compressed(pk.as_bytes())
}

fn load_or_create_secret() -> Result<Vec<u8>, String> {
    let path = key_path();
    if path.is_file() {
        let raw = std::fs::read(&path).map_err(|e| format!("runtime sk: {e}"))?;
        if raw.len() != 32 {
            return Err(
                "dregg runtime sk file is not 32 bytes; refusing to replace it".into(),
            );
        }
        return Ok(raw);
    }
    if std::env::var("DREGG_RUNTIME_NO_CREATE").ok().as_deref() == Some("1") {
        return Err("runtime sk missing".into());
    }
    let mut raw = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut raw);
    if SigningKey::from_slice(&raw).is_err() {
        return Err("runtime sk not on the secp256k1 curve".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &raw).map_err(|e| format!("write runtime sk: {e}"))?;
    set_mode_600(&path);
    Ok(raw.to_vec())
}

fn set_mode_600(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

/// Whitelist leaf for one collection. Absent means an empty proof (logos today).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymintProof {
    pub alloc: u32,
    pub proof_hashes: Vec<String>,
}

fn paymint_path() -> PathBuf {
    if let Ok(p) = std::env::var("DREGG_PAYMINT_PROOFS_FILE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".terp/dregg-paymint-proofs.json")
}

/// `{ "terp1…": { "alloc": 10000, "proof_hashes": ["…"] } }`.
/// The alloc is the merkle leaf cap, not a decreasing counter.
pub fn paymint_for(collection: &str) -> PaymintProof {
    let Ok(text) = std::fs::read_to_string(paymint_path()) else {
        return PaymintProof { alloc: 0, proof_hashes: Vec::new() };
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return PaymintProof { alloc: 0, proof_hashes: Vec::new() };
    };
    let Some(row) = v.get(collection) else {
        return PaymintProof { alloc: 0, proof_hashes: Vec::new() };
    };
    let alloc = row.get("alloc").and_then(|n| {
        n.as_u64().or_else(|| n.as_str().and_then(|s| s.parse().ok()))
    }).unwrap_or(0) as u32;
    let proof_hashes = row
        .get("proof_hashes")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    PaymintProof { alloc, proof_hashes }
}

fn paymint_used_path() -> PathBuf {
    if let Ok(p) = std::env::var("DREGG_PAYMINT_USED_FILE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".terp/dregg-paymint-used.json")
}

fn paymint_used(collection: &str) -> u64 {
    let Ok(text) = std::fs::read_to_string(paymint_used_path()) else {
        return 0;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return 0;
    };
    v.get(collection)
        .and_then(|n| n.as_u64().or_else(|| n.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0)
}

fn note_paymint_used(collection: &str, n: u64) {
    let path = paymint_used_path();
    let mut v: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let prev = v.get(collection).and_then(|n| n.as_u64()).unwrap_or(0);
    if let Some(obj) = v.as_object_mut() {
        obj.insert(collection.to_string(), serde_json::json!(prev.saturating_add(n)));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, v.to_string());
}

/// Room left under the whitelist alloc. `None` when this collection has no proof.
pub fn paymint_room(collection: &str) -> Option<u64> {
    let proof = paymint_for(collection);
    if proof.proof_hashes.is_empty() || proof.alloc == 0 {
        return None;
    }
    Some((proof.alloc as u64).saturating_sub(paymint_used(collection)))
}

/// Empty-funds SVG mint. Proof fields come from [`paymint_for`].
pub fn svg_mint_msg(collection: &str, amnt: u32) -> Vec<u8> {
    let proof = paymint_for(collection);
    serde_json::json!({
        "update_extension": {
            "msg": {
                "mint": {
                    "alloc": proof.alloc,
                    "amnt": amnt.max(1),
                    "ext": null,
                    "proof_hashes": proof.proof_hashes
                }
            }
        }
    })
    .to_string()
    .into_bytes()
}

/// Address to name on `/health` when the key file is absent.
/// Sk file wins. Then `DREGG_RUNTIME_ADDR`, then `DREGG_MINTER_ADDR`.
pub fn runtime_addr_for_health() -> Option<String> {
    if let Some(a) = address_if_present() {
        return Some(a);
    }
    for name in ["DREGG_RUNTIME_ADDR", "DREGG_MINTER_ADDR"] {
        if let Ok(s) = std::env::var(name) {
            if s.starts_with("terp1") {
                return Some(s);
            }
        }
    }
    None
}

pub fn transfer_nft_msg(recipient: &str, token_id: &str) -> Vec<u8> {
    serde_json::json!({
        "transfer_nft": { "recipient": recipient, "token_id": token_id }
    })
    .to_string()
    .into_bytes()
}

pub fn mint_gas() -> u64 {
    std::env::var("DREGG_MINT_GAS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &u64| *n > 0)
        .unwrap_or(800_000)
}

/// House floor is `0.05uthiol`. `fee = ceil(gas * price)`.
pub fn fee_uthiol(gas: u64) -> u128 {
    let (num, den) = gas_price_ratio();
    let g = gas as u128;
    let q = g.saturating_mul(num) / den;
    let rem = g.saturating_mul(num) % den;
    if rem == 0 {
        q
    } else {
        q.saturating_add(1)
    }
}

fn gas_price_ratio() -> (u128, u128) {
    let raw = std::env::var("DREGG_MINT_GAS_PRICE").unwrap_or_else(|_| "0.05".into());
    let raw = raw.trim().trim_end_matches("uthiol").trim();
    if let Some((w, f)) = raw.split_once('.') {
        let den = 10u128.pow(f.len() as u32);
        let whole: u128 = w.parse().unwrap_or(0);
        let part: u128 = if f.is_empty() { 0 } else { f.parse().unwrap_or(0) };
        let num = whole.saturating_mul(den).saturating_add(part);
        if num == 0 || den == 0 {
            (5, 100)
        } else {
            (num, den)
        }
    } else {
        let n: u128 = raw.parse().unwrap_or(0);
        if n == 0 {
            (5, 100)
        } else {
            (n, 1)
        }
    }
}

pub fn chain_id() -> String {
    std::env::var("DREGG_CHAIN_ID").unwrap_or_else(|_| "morocco-1".into())
}

pub fn lcd_base() -> Result<String, String> {
    for name in ["DREGG_MINTER_LCD", "DREGG_LCD"] {
        if let Ok(s) = std::env::var(name) {
            let s = s.trim().trim_end_matches('/').to_string();
            if !s.is_empty() {
                return Ok(s);
            }
        }
    }
    Err("DREGG_MINTER_LCD unset".into())
}

/// SIGN_MODE_DIRECT tx bytes for one or more wasm executes. No funds attached.
pub fn tx_raw(
    secret32: &[u8],
    contract: &str,
    msgs: &[Vec<u8>],
    account_number: u64,
    sequence: u64,
    chain_id: &str,
    gas: u64,
    fee_amount: u128,
) -> Result<Vec<u8>, String> {
    if secret32.len() != 32 {
        return Err("runtime sk 32 bytes".into());
    }
    let sk = SigningKey::from_slice(secret32).map_err(|e| e.to_string())?;
    let pk = sk.verifying_key().to_sec1_point(true);
    let sender = crate::adr36::terp1_from_secp256k1_compressed(pk.as_bytes())?;
    tx_raw_as(&sk, &sender, contract, msgs, account_number, sequence, chain_id, gas, fee_amount)
}

fn tx_raw_as(
    sk: &SigningKey,
    sender: &str,
    contract: &str,
    msgs: &[Vec<u8>],
    account_number: u64,
    sequence: u64,
    chain_id: &str,
    gas: u64,
    fee_amount: u128,
) -> Result<Vec<u8>, String> {
    let pk = sk.verifying_key().to_sec1_point(true);
    let mut body = Vec::new();
    for msg in msgs {
        let exec = msg_execute(&sender, contract, msg);
        body.extend(ld(1, &any(MSG_TYPE, &exec)));
    }
    let pk_msg = ld(1, pk.as_bytes());
    let pk_any = any(PK_TYPE, &pk_msg);
    let mut signer = ld(1, &pk_any);
    signer.extend(ld(2, &mode_direct()));
    signer.extend(vi(3, sequence));
    let coin = coin_msg("uthiol", &fee_amount.to_string());
    let mut fee = ld(1, &coin);
    fee.extend(vi(2, gas));
    let mut auth = ld(1, &signer);
    auth.extend(ld(2, &fee));
    let sig = sign_direct(sk, &body, &auth, chain_id, account_number)?;
    let mut raw = ld(1, &body);
    raw.extend(ld(2, &auth));
    raw.extend(ld(3, &sig));
    Ok(raw)
}

fn sign_direct(
    sk: &SigningKey,
    body: &[u8],
    auth: &[u8],
    chain_id: &str,
    account_number: u64,
) -> Result<Vec<u8>, String> {
    let mut doc = ld(1, body);
    doc.extend(ld(2, auth));
    doc.extend(ld(3, chain_id.as_bytes()));
    doc.extend(vi(4, account_number));
    let hash = Sha256::digest(&doc);
    let sig: Signature = sk.sign_prehash(&hash).map_err(|e| e.to_string())?;
    let sig = sig.normalize_s();
    let vk = sk.verifying_key();
    vk.verify_prehash(&hash, &sig)
        .map_err(|_| "runtime signature did not verify".to_string())?;
    Ok(sig.to_bytes().to_vec())
}

pub fn account_number_sequence(v: &serde_json::Value) -> Result<(u64, u64), String> {
    fn num(v: &serde_json::Value) -> Option<u64> {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }
    fn walk(v: &serde_json::Value) -> Option<(u64, u64)> {
        let obj = v.as_object()?;
        if let (Some(a), Some(s)) = (obj.get("account_number"), obj.get("sequence")) {
            if let (Some(a), Some(s)) = (num(a), num(s)) {
                return Some((a, s));
            }
        }
        for child in obj.values() {
            if let Some(p) = walk(child) {
                return Some(p);
            }
        }
        None
    }
    walk(v).ok_or_else(|| "account number missing".into())
}

pub fn token_ids_from_tx(v: &serde_json::Value) -> Vec<String> {
    let mut ids = Vec::new();
    walk_token_ids(v, &mut ids);
    ids
}

fn attr_text(s: &str) -> String {
    b64_decode_utf8(s).unwrap_or_else(|| s.to_string())
}

fn b64_decode_utf8(s: &str) -> Option<String> {
    let bytes = b64_decode(s)?;
    String::from_utf8(bytes).ok()
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let raw: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    if raw.is_empty() || raw.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::new();
    for chunk in raw.chunks(4) {
        let pad = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            if *c == b'=' {
                if i < 4 - pad {
                    return None;
                }
                continue;
            }
            n = (n << 6) | val(*c)? as u32;
        }
        n <<= 6 * pad as u32;
        let nbytes = 3 - pad;
        for i in 0..nbytes {
            out.push((n >> (8 * (2 - i))) as u8);
        }
    }
    Some(out)
}

fn walk_token_ids(v: &serde_json::Value, ids: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            if let (Some(k), Some(val)) = (
                m.get("key").and_then(|x| x.as_str()),
                m.get("value").and_then(|x| x.as_str()),
            ) {
                let key = attr_text(k);
                let value = attr_text(val);
                if key == "token_id" || key.ends_with(".token_id") {
                    if !value.is_empty() && !ids.iter().any(|e| e == &value) {
                        ids.push(value);
                    }
                }
            }
            for child in m.values() {
                walk_token_ids(child, ids);
            }
        }
        serde_json::Value::Array(a) => {
            for child in a {
                walk_token_ids(child, ids);
            }
        }
        _ => {}
    }
}

pub fn tx_code(v: &serde_json::Value) -> Option<u64> {
    v.pointer("/tx_response/code")
        .and_then(json_u64)
        .or_else(|| v.get("code").and_then(json_u64))
}

pub fn tx_hash(v: &serde_json::Value) -> Option<String> {
    v.pointer("/tx_response/txhash")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("txhash").and_then(|x| x.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn json_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// What `/cosmos/tx/v1beta1/simulate` said about an empty-funds SVG mint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimVerdict {
    /// The contract accepted the message in simulation.
    Accepted,
    /// The contract ran and rejected this signer. Keep the address out of the pay set.
    Rejected(String),
    /// The chain could not judge the contract (missing account, fees, LCD). Pay stays closed.
    Unproven(String),
}

fn clip_reason(raw: &str) -> String {
    let one = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s: String = one.chars().take(180).collect();
    if one.chars().count() > 180 {
        s.push('…');
    }
    s
}

fn simulate_message(body: &serde_json::Value) -> String {
    let pick = body
        .get("message")
        .and_then(|x| x.as_str())
        .or_else(|| body.get("raw_log").and_then(|x| x.as_str()))
        .or_else(|| body.pointer("/result/log").and_then(|x| x.as_str()))
        .or_else(|| {
            body.pointer("/tx_response/raw_log")
                .and_then(|x| x.as_str())
        })
        .unwrap_or("");
    clip_reason(pick)
}

fn unproven_marker(low: &str) -> bool {
    low.contains("insufficient fee")
        || (low.contains("account") && low.contains("not found"))
        || low.contains("signature verification")
        || low.contains("account sequence")
        || low.contains("sequence mismatch")
        || low.contains("out of gas")
        || low.contains("runtime sk")
}

fn contract_marker(low: &str) -> bool {
    low.contains("failed to execute message")
        || low.contains("execute wasm")
        || low.contains("insufficient funds")
        || low.contains("unauthorized")
        || low.contains("wrong amount")
}

/// Classify a simulate HTTP response. Contract rejection is the only `Rejected`.
pub fn classify_simulate(status: u16, body: &serde_json::Value) -> SimVerdict {
    let msg = simulate_message(body);
    let low = msg.to_ascii_lowercase();
    if unproven_marker(&low) {
        return SimVerdict::Unproven(if msg.is_empty() {
            "simulate unproven".into()
        } else {
            msg
        });
    }
    let code = body
        .get("code")
        .and_then(json_u64)
        .or_else(|| body.pointer("/result/code").and_then(json_u64))
        .unwrap_or(0);
    if status == 200 && code == 0 && body.get("gas_info").is_some() {
        return SimVerdict::Accepted;
    }
    if contract_marker(&low) {
        return SimVerdict::Rejected(msg);
    }
    let why = if msg.is_empty() {
        format!("simulate status {status}")
    } else {
        msg
    };
    SimVerdict::Unproven(why)
}

fn read_secret() -> Result<Vec<u8>, String> {
    let path = key_path();
    if !path.is_file() {
        return Err("runtime sk missing".into());
    }
    let raw = std::fs::read(&path).map_err(|e| format!("runtime sk: {e}"))?;
    if raw.len() != 32 {
        return Err("runtime sk file is not 32 bytes; refusing to replace it".into());
    }
    Ok(raw)
}

/// Simulate one empty-funds `update_extension` mint. Does not write a key or broadcast.
pub fn simulate_empty_mint(contract: &str) -> SimVerdict {
    if !contract.starts_with("terp1") {
        return SimVerdict::Rejected("mint contract must be terp1".into());
    }
    // A present key signs as itself. With no key file, simulate still names
    // `DREGG_RUNTIME_ADDR` / `DREGG_MINTER_ADDR` and does not write a key.
    let (secret, sender, keyless) = if let Ok(raw) = read_secret() {
        match address_of(&raw) {
            Ok(sender) => (raw, sender, false),
            Err(e) => return SimVerdict::Unproven(e),
        }
    } else if let Some(sender) = runtime_addr_for_health() {
        (vec![7u8; 32], sender, true)
    } else {
        return SimVerdict::Unproven("runtime sk missing".into());
    };
    let lcd = match lcd_base() {
        Ok(s) => s,
        Err(e) => return SimVerdict::Unproven(e),
    };
    let acct = match crate::httpx::get_json(&format!(
        "{lcd}/cosmos/auth/v1beta1/accounts/{sender}"
    )) {
        Ok(v) => v,
        Err(e) => return SimVerdict::Unproven(clip_reason(&e)),
    };
    let (account_number, sequence) = match account_number_sequence(&acct) {
        Ok(p) => p,
        Err(e) => return SimVerdict::Unproven(e),
    };
    let gas = 2_000_000u64;
    let fee = fee_uthiol(gas);
    let sk = match SigningKey::from_slice(&secret) {
        Ok(sk) => sk,
        Err(e) => return SimVerdict::Unproven(e.to_string()),
    };
    let raw = match tx_raw_as(
        &sk,
        &sender,
        contract,
        &[svg_mint_msg(contract, 1)],
        account_number,
        sequence,
        &chain_id(),
        gas,
        fee,
    ) {
        Ok(b) => b,
        Err(e) => return SimVerdict::Unproven(clip_reason(&e)),
    };
    let _ = keyless;
    let body = serde_json::json!({ "tx_bytes": b64(&raw) });
    match crate::httpx::post_json_status(&format!("{lcd}/cosmos/tx/v1beta1/simulate"), &body) {
        Ok((status, v)) => classify_simulate(status, &v),
        Err(e) => SimVerdict::Unproven(clip_reason(&e)),
    }
}

/// A mint the chain accepted. `transfer_tx` is empty until the NFT moves to the payer.
#[derive(Clone, Debug)]
pub struct PaidMint {
    pub mint_tx: String,
    pub token_ids: Vec<String>,
    pub transfer_tx: String,
}

/// Broadcast the mint, wait for the committed tx, then transfer the new tokens
/// to `recipient`. If the mint is committed and the transfer fails, the `Err`
/// still starts with `minted {hash}` so the caller does not mint again.
pub fn broadcast_paid_mint(
    contract: &str,
    recipient: &str,
    nfts: u32,
) -> Result<PaidMint, String> {
    if !contract.starts_with("terp1") {
        return Err("mint contract must be terp1".into());
    }
    if !recipient.starts_with("terp1") {
        return Err("mint recipient must be terp1".into());
    }
    if !crate::collections::contains(contract) {
        return Err("collection is not in the watched set".into());
    }
    let secret = load_or_create_secret()?;
    let nfts = nfts.max(1);
    if let Some(room) = paymint_room(contract) {
        if nfts as u64 > room {
            return Err("paymint alloc exhausted".into());
        }
    }
    let (hash, ids) = submit(&secret, contract, &[svg_mint_msg(contract, nfts)])?;
    if ids.is_empty() {
        return Err(format!("minted {hash}"));
    }
    let transfers: Vec<Vec<u8>> = ids
        .iter()
        .map(|id| transfer_nft_msg(recipient, id))
        .collect();
    match submit(&secret, contract, &transfers) {
        Ok((xfer, _)) => {
            note_paymint_used(contract, nfts as u64);
            Ok(PaidMint {
                mint_tx: hash,
                token_ids: ids,
                transfer_tx: xfer,
            })
        }
        Err(e) => Err(format!(
            "minted {hash} ids {} transfer: {e}",
            ids.join(",")
        )),
    }
}

/// Poll a mint hash that already succeeded CheckTx and finish the transfer.
pub fn finish_transfer(
    contract: &str,
    recipient: &str,
    mint_hash: &str,
    known_ids: &[String],
) -> Result<(Vec<String>, String), String> {
    if !crate::collections::contains(contract) {
        return Err("collection is not in the watched set".into());
    }
    let ids = if known_ids.is_empty() {
        let lcd = lcd_base()?;
        let v = crate::httpx::get_json(&format!(
            "{lcd}/cosmos/tx/v1beta1/txs/{mint_hash}"
        ))?;
        let code = tx_code(&v).ok_or_else(|| format!("mint {mint_hash} not committed"))?;
        if code != 0 {
            return Err(format!("mint {mint_hash} code {code}"));
        }
        let ids = token_ids_from_tx(&v);
        if ids.is_empty() {
            return Err(format!("mint {mint_hash} has no token_id yet"));
        }
        ids
    } else {
        known_ids.to_vec()
    };
    let secret = load_or_create_secret()?;
    let transfers: Vec<Vec<u8>> = ids
        .iter()
        .map(|id| transfer_nft_msg(recipient, id))
        .collect();
    let (xfer, _) = submit(&secret, contract, &transfers)?;
    Ok((ids, xfer))
}

fn submit(
    secret: &[u8],
    contract: &str,
    msgs: &[Vec<u8>],
) -> Result<(String, Vec<String>), String> {
    let lcd = lcd_base()?;
    let sender = address_of(secret)?;
    let acct = crate::httpx::get_json(&format!(
        "{lcd}/cosmos/auth/v1beta1/accounts/{sender}"
    ))?;
    let (account_number, sequence) = account_number_sequence(&acct)?;
    let gas = mint_gas().saturating_mul(msgs.len() as u64);
    let fee = fee_uthiol(gas);
    let raw = tx_raw(
        secret,
        contract,
        msgs,
        account_number,
        sequence,
        &chain_id(),
        gas,
        fee,
    )?;
    let body = serde_json::json!({
        "tx_bytes": b64(&raw),
        "mode": "BROADCAST_MODE_SYNC",
    });
    let sent = crate::httpx::post_json(&format!("{lcd}/cosmos/tx/v1beta1/txs"), &body)?;
    let code = tx_code(&sent).unwrap_or(0);
    let hash = tx_hash(&sent).ok_or_else(|| {
        let log = sent
            .pointer("/tx_response/raw_log")
            .and_then(|x| x.as_str())
            .unwrap_or("no txhash");
        format!("broadcast rejected: {log}")
    })?;
    if code != 0 {
        let log = sent
            .pointer("/tx_response/raw_log")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        return Err(format!("broadcast code {code} {hash} {log}"));
    }
    let committed = poll_tx(&lcd, &hash)?;
    let ids = token_ids_from_tx(&committed);
    Ok((hash, ids))
}

fn poll_tx(lcd: &str, hash: &str) -> Result<serde_json::Value, String> {
    let url = format!("{lcd}/cosmos/tx/v1beta1/txs/{hash}");
    let mut last = String::from("not found");
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        match crate::httpx::get_json(&url) {
            Ok(v) => {
                // LCD answers 200 with `{"code":5,"message":"tx not found"}` before
                // the tx is indexed. That is not a committed failure.
                if v.get("tx_response").is_none() {
                    last = v
                        .get("message")
                        .and_then(|x| x.as_str())
                        .unwrap_or("tx not indexed")
                        .to_string();
                    continue;
                }
                if let Some(code) = tx_code(&v) {
                    if code != 0 {
                        let log = v
                            .pointer("/tx_response/raw_log")
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        return Err(format!("tx {hash} code {code} {log}"));
                    }
                    return Ok(v);
                }
                last = "tx has no code yet".into();
            }
            Err(e) => last = e,
        }
    }
    Err(format!("tx {hash} not committed: {last}"))
}

fn msg_execute(sender: &str, contract: &str, msg: &[u8]) -> Vec<u8> {
    let mut o = ld(1, sender.as_bytes());
    o.extend(ld(2, contract.as_bytes()));
    o.extend(ld(3, msg));
    o
}

fn any(type_url: &str, value: &[u8]) -> Vec<u8> {
    let mut o = ld(1, type_url.as_bytes());
    o.extend(ld(2, value));
    o
}

fn mode_direct() -> Vec<u8> {
    // ModeInfo.single = Single { mode: SIGN_MODE_DIRECT (1) }
    ld(1, &vi(1, 1))
}

fn coin_msg(denom: &str, amount: &str) -> Vec<u8> {
    let mut o = ld(1, denom.as_bytes());
    o.extend(ld(2, amount.as_bytes()));
    o
}

fn varint(mut n: u64) -> Vec<u8> {
    let mut o = Vec::new();
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            b |= 0x80;
        }
        o.push(b);
        if n == 0 {
            break;
        }
    }
    o
}

fn tag(field: u32, wire: u8) -> Vec<u8> {
    varint(((field as u64) << 3) | wire as u64)
}

fn ld(field: u32, bytes: &[u8]) -> Vec<u8> {
    let mut o = tag(field, 2);
    o.extend(varint(bytes.len() as u64));
    o.extend_from_slice(bytes);
    o
}

fn vi(field: u32, n: u64) -> Vec<u8> {
    let mut o = tag(field, 0);
    o.extend(varint(n));
    o
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    match data.len() - i {
        1 => {
            let n = (data[i] as u32) << 16;
            out.push(T[((n >> 18) & 63) as usize] as char);
            out.push(T[((n >> 12) & 63) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
            out.push(T[((n >> 18) & 63) as usize] as char);
            out.push(T[((n >> 12) & 63) as usize] as char);
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // rustc 1.87 made these unsafe. 1.86 still accepts the block.
    fn env_set(key: &str, val: impl AsRef<std::ffi::OsStr>) {
        unsafe { std::env::set_var(key, val) }
    }

    fn env_remove(key: &str) {
        unsafe { std::env::remove_var(key) }
    }

    #[test]
    fn simulate_accepts_gas_info() {
        let v = serde_json::json!({
            "gas_info": {"gas_wanted": "2000000", "gas_used": "180000"},
            "result": {"log": "", "events": []}
        });
        assert_eq!(classify_simulate(200, &v), SimVerdict::Accepted);
    }

    #[test]
    fn simulate_rejects_wasm_execute() {
        let v = serde_json::json!({
            "code": 5,
            "message": "failed to execute message; message index: 0: Needed funds: execute wasm contract failed"
        });
        match classify_simulate(400, &v) {
            SimVerdict::Rejected(m) => assert!(m.contains("Needed funds")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn simulate_insufficient_fees_is_unproven() {
        let v = serde_json::json!({"code": 13, "message": "insufficient fees; got: 0uthiol"});
        assert!(matches!(classify_simulate(400, &v), SimVerdict::Unproven(_)));
        let missing = serde_json::json!({"code": 5, "message": "account terp1abc not found"});
        assert!(matches!(classify_simulate(404, &missing), SimVerdict::Unproven(_)));
    }

    #[test]
    fn fee_matches_house_floor() {
        let _g = crate::checkout::minter_env_lock();
        env_remove("DREGG_MINT_GAS_PRICE");
        assert_eq!(fee_uthiol(800_000), 40_000);
    }

    #[test]
    fn account_walks_base_account() {
        let v = serde_json::json!({
            "account": {
                "@type": "/cosmos.auth.v1beta1.BaseAccount",
                "address": "terp1x",
                "account_number": "7",
                "sequence": "3"
            }
        });
        assert_eq!(account_number_sequence(&v).unwrap(), (7, 3));
    }

    #[test]
    fn token_ids_from_wasm_events() {
        let v = serde_json::json!({
            "tx_response": {
                "code": 0,
                "txhash": "AB",
                "events": [{
                    "type": "wasm",
                    "attributes": [
                        {"key": "action", "value": "mint"},
                        {"key": "token_id", "value": "4"},
                        {"key": "token_id", "value": "5"}
                    ]
                }]
            }
        });
        assert_eq!(token_ids_from_tx(&v), vec!["4".to_string(), "5".to_string()]);
        let encoded = serde_json::json!({
            "tx_response": {
                "code": 0,
                "events": [{
                    "type": "wasm-mint",
                    "attributes": [
                        {"key": "dG9rZW5faWQ=", "value": "MA=="}
                    ]
                }]
            }
        });
        assert_eq!(token_ids_from_tx(&encoded), vec!["0".to_string()]);
        let missing = serde_json::json!({"code": 5, "message": "tx not found"});
        assert!(missing.get("tx_response").is_none());
        assert_eq!(tx_code(&v), Some(0));
        assert_eq!(tx_hash(&v).as_deref(), Some("AB"));
    }

    #[test]
    fn direct_signature_verifies_and_address_is_stable() {
        let secret = [7u8; 32];
        let msg = svg_mint_msg("terp1contract", 1);
        let raw = tx_raw(&secret, "terp1contract", &[msg], 1, 0, "morocco-1", 800_000, 40_000)
            .unwrap();
        assert!(raw.len() > 80);
        let a = address_of(&secret).unwrap();
        let b = address_of(&secret).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("terp1"));
        let sk = SigningKey::from_slice(&secret).unwrap();
        let vk = k256::ecdsa::VerifyingKey::from(sk);
        assert_eq!(vk.to_sec1_point(true).as_bytes().len(), 33);
    }

    #[test]
    fn key_file_is_created_once() {
        let _g = crate::checkout::minter_env_lock();
        let dir = std::env::temp_dir().join(format!(
            "dregg-runtime-sk-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runtime.sk");
        env_set("DREGG_RUNTIME_SK_FILE", &path);
        let a = ensure_address().unwrap();
        let b = ensure_address().unwrap();
        assert_eq!(a, b);
        assert_eq!(std::fs::read(&path).unwrap().len(), 32);
        std::fs::write(&path, [1u8; 16]).unwrap();
        assert!(ensure_address().is_err());
        env_remove("DREGG_RUNTIME_SK_FILE");
        env_remove("DREGG_RUNTIME_NO_CREATE");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_create_does_not_write_a_key() {
        let _g = crate::checkout::minter_env_lock();
        let path = std::env::temp_dir().join(format!("dregg-nocreate-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        env_set("DREGG_RUNTIME_SK_FILE", &path);
        env_set("DREGG_RUNTIME_NO_CREATE", "1");
        assert!(ensure_address().is_err());
        assert!(!path.exists());
        env_remove("DREGG_RUNTIME_NO_CREATE");
        env_remove("DREGG_RUNTIME_SK_FILE");
    }

    #[test]
    fn fractal_msg_matches_paymint_template() {
        let _g = crate::checkout::minter_env_lock();
        let fractal = "terp15anc99l7lvk2v2pzjusnmylr3lw3xnpxg89xv8uh0fj6pwrumrksqwzs6x";
        let logo = "terp1yldtf5nsychkasyz3gddemqy45fvl2fxpmc8jrsqvt5fv7ggghysektnqk";
        let proof = "59a73b7800d05280d8838a704cc496993b7ca578e7a3b4d016c433a8b869aaf5";
        let template = r#"{"update_extension":{"msg":{"mint":{"alloc":10000,"amnt":1,"ext":null,"proof_hashes":["59a73b7800d05280d8838a704cc496993b7ca578e7a3b4d016c433a8b869aaf5"]}}}}"#;
        let path = std::env::temp_dir().join(format!("dregg-paymint-{}.json", std::process::id()));
        let body = serde_json::json!({
            fractal: { "alloc": 10000, "proof_hashes": [proof] }
        });
        std::fs::write(&path, body.to_string()).unwrap();
        env_set("DREGG_PAYMINT_PROOFS_FILE", &path);
        let msg = svg_mint_msg(fractal, 1);
        let got: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        let expect: serde_json::Value = serde_json::from_str(template).unwrap();
        assert_eq!(got, expect);
        assert_eq!(String::from_utf8(msg).unwrap(), template);
        let bare = svg_mint_msg(logo, 1);
        let bare_v: serde_json::Value = serde_json::from_slice(&bare).unwrap();
        assert_eq!(bare_v["update_extension"]["msg"]["mint"]["alloc"], 0);
        assert!(bare_v["update_extension"]["msg"]["mint"]["proof_hashes"]
            .as_array()
            .unwrap()
            .is_empty());
        env_remove("DREGG_PAYMINT_PROOFS_FILE");
        let _ = std::fs::remove_file(&path);
    }
}
