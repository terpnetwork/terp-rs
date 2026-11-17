//! House Ironwood wallet inside svg-mint-zrt: no zcash-devtool.
//!
//! One scanner (zync-core, the same one the checkout watcher uses) reads compact
//! blocks from the house lightwalletd. State is a small JSON file next to the
//! viewing key: note positions, values, own nullifiers, spent heights, and the
//! commitment-tree size at the cursor (self-checked against lightwalletd).
//! Withdraw rebuilds witnesses from the birthday frontier, proves and signs an
//! Ironwood v6 tx in process (zafu-wasm builder), and broadcasts through lwd.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

use zync_core::client::lightwalletd_proto::{compact_tx_streamer_client::CompactTxStreamerClient, BlockId};
use zync_core::{LightwalletdClient, OrchardFvk, ScanAction, Scanner, Scope};

/// Wallet keys were created 2026-10-30 ~19:08 UTC (≈ height 3,537,770). Birthday a bit before.
pub const DEFAULT_BIRTHDAY: u64 = 3_537_000;
pub const ANCHOR_OFFSET: u64 = 3;
const NU6_3_BRANCH_ID: u32 = 0x37a5_165b;
const ZIP317_MARGINAL: u64 = 5_000;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WNote {
    pub txid: String,
    pub height: u64,
    pub position: u64,
    pub value: u64,
    pub nf: String,
    pub scope: String,
    pub spent_height: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WState {
    pub birthday: u64,
    pub cursor: u64,
    pub tree_size: u64,
    pub tip: u64,
    pub synced_at: u64,
    pub notes: Vec<WNote>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn state_path() -> PathBuf {
    crate::wallet_dir().join("house-wallet.json")
}

pub fn birthday() -> u64 {
    std::env::var("SVG_MINT_WALLET_BIRTHDAY").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_BIRTHDAY)
}

pub fn load() -> Option<WState> {
    fs::read_to_string(state_path()).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn save(st: &WState) -> Result<(), String> {
    let dir = crate::wallet_dir();
    if !dir.join("orchard-fvk").is_file() {
        return Err("house wallet folder has no viewing key; refusing to write state".into());
    }
    let tmp = dir.join("house-wallet.json.tmp");
    fs::write(&tmp, serde_json::to_vec(st).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, state_path()).map_err(|e| e.to_string())
}

/// Simple cross-process lock so the watcher and a withdraw never write state at once.
pub struct Lock(PathBuf);
impl Lock {
    pub fn take() -> Result<Lock, String> {
        let p = crate::wallet_dir().join("house-wallet.lock");
        for _ in 0..120 {
            match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
                Ok(_) => return Ok(Lock(p)),
                Err(_) => {
                    let stale = fs::metadata(&p).and_then(|m| m.modified()).ok()
                        .and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() > 900).unwrap_or(false);
                    if stale {
                        let _ = fs::remove_file(&p);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
        Err("house wallet busy".into())
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

async fn tree_state_hex(url: &str, height: u64) -> Result<String, String> {
    let mut c = CompactTxStreamerClient::connect(url.to_string()).await.map_err(|e| format!("lwd connect: {e}"))?;
    let ts = c
        .get_tree_state(BlockId { height, hash: vec![] })
        .await
        .map_err(|e| format!("tree state {height}: {e}"))?
        .into_inner();
    if ts.ironwood_tree.trim().is_empty() {
        return Err(format!("lightwalletd returned no ironwood tree at {height}"));
    }
    Ok(ts.ironwood_tree)
}

fn tree_size(hex_tree: &str) -> Result<u64, String> {
    let b = hex::decode(hex_tree.trim()).map_err(|e| format!("tree hex: {e}"))?;
    zafu_wasm::witness::compute_frontier_tree_size(&b)
}

fn scan_one(ext: &Scanner, int: &Scanner, a: &zync_core::client::CompactAction, h: u32) -> Option<(zync_core::DecryptedNote, &'static str)> {
    let sa = ScanAction {
        nullifier: a.nullifier,
        cmx: a.cmx,
        ephemeral_key: a.ephemeral_key,
        enc_ciphertext: a.ciphertext.clone(),
        block_height: h,
        position_in_block: 0,
    };
    if let Some(d) = ext.try_decrypt_ironwood(&sa) {
        return Some((d, "external"));
    }
    int.try_decrypt_ironwood(&sa).map(|d| (d, "internal"))
}

/// Incremental sync to `tip`. Creates state from the birthday frontier the first time.
pub async fn sync(client: &mut LightwalletdClient, url: &str, fvk: &OrchardFvk, tip: u64) -> Result<WState, String> {
    let _lock = Lock::take()?;
    let mut st = match load() {
        Some(s) => s,
        None => {
            let b = birthday();
            let t = tree_state_hex(url, b - 1).await?;
            WState { birthday: b, cursor: b - 1, tree_size: tree_size(&t)?, tip, synced_at: now(), notes: vec![] }
        }
    };
    let ext = Scanner::from_fvk(fvk);
    let int = Scanner::from_fvk_scoped(fvk, Scope::Internal);
    let budget: u64 = std::env::var("SVG_MINT_WALLET_SCAN_MAX").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let stop = tip.min(st.cursor + budget);
    while st.cursor < stop {
        let start = st.cursor + 1;
        let end = (start + 999).min(stop);
        let txs = client.get_ironwood_block_range(start, end).await.map_err(|e| format!("ironwood range: {e}"))?;
        for tx in txs {
            let txid = hex::encode(tx.txid.iter().rev().copied().collect::<Vec<u8>>());
            for a in &tx.actions {
                let pos = st.tree_size;
                st.tree_size += 1;
                let nf_hex = hex::encode(a.nullifier);
                for n in st.notes.iter_mut() {
                    if n.spent_height.is_none() && n.nf == nf_hex {
                        n.spent_height = Some(tx.height as u64);
                    }
                }
                if let Some((d, scope)) = scan_one(&ext, &int, a, tx.height) {
                    if !st.notes.iter().any(|n| n.position == pos) {
                        st.notes.push(WNote {
                            txid: txid.clone(),
                            height: tx.height as u64,
                            position: pos,
                            value: d.value,
                            nf: hex::encode(d.note.nullifier(fvk).to_bytes()),
                            scope: scope.into(),
                            spent_height: None,
                        });
                    }
                }
            }
        }
        st.cursor = end;
    }
    // Self-check: our counted tree size must equal lightwalletd's frontier at the cursor.
    let t = tree_state_hex(url, st.cursor).await?;
    let want = tree_size(&t)?;
    if want != st.tree_size {
        let _ = fs::rename(state_path(), crate::wallet_dir().join(format!("house-wallet.json.mismatch-{}", now())));
        return Err(format!("tree size mismatch at {} (counted {}, lwd {}); state set aside, will rescan", st.cursor, st.tree_size, want));
    }
    st.tip = tip;
    st.synced_at = now();
    save(&st)?;
    Ok(st)
}

pub fn balance_json(st: &WState) -> Value {
    let unspent: Vec<&WNote> = st.notes.iter().filter(|n| n.spent_height.is_none()).collect();
    let spendable: u64 = unspent.iter().filter(|n| n.height + ANCHOR_OFFSET <= st.cursor).map(|n| n.value).sum();
    let pending: u64 = unspent.iter().filter(|n| n.height + ANCHOR_OFFSET > st.cursor).map(|n| n.value).sum();
    json!({
        "total": spendable + pending,
        "sapling_spendable": 0,
        "orchard_spendable": 0,
        "ironwood_spendable": spendable,
        "transparent_spendable": 0,
        "pending": pending,
        "chain_tip_height": st.tip,
        "synced_height": st.cursor,
        "birthday": st.birthday,
        "synced_at": st.synced_at,
        "unspent_notes": unspent.len(),
        "notes_seen": st.notes.len(),
        "engine": "svg-mint-zrt in-process (no zcash-devtool)",
    })
}

pub fn zip317_fee(spends: usize) -> u64 {
    ZIP317_MARGINAL * (spends.max(2) as u64)
}

fn load_spend_key(fvk: &OrchardFvk) -> Result<(orchard::keys::SpendingKey, orchard::keys::SpendAuthorizingKey), String> {
    let p = crate::wallet_dir().join("spend.seed");
    let raw = fs::read(&p).map_err(|e| format!("spend key unreadable: {}", e.kind()))?;
    if raw.len() != 32 {
        return Err("spend key has the wrong length".into());
    }
    let mut b = [0u8; 32];
    b.copy_from_slice(&raw);
    let sk: Option<orchard::keys::SpendingKey> = orchard::keys::SpendingKey::from_bytes(b).into();
    b.fill(0);
    let sk = sk.ok_or("spend key rejected")?;
    let derived = orchard::keys::FullViewingKey::from(&sk);
    if derived.to_bytes() != fvk.to_bytes() {
        return Err("spend key does not match the house viewing key".into());
    }
    let ask = orchard::keys::SpendAuthorizingKey::from(&sk);
    Ok((sk, ask))
}

/// Build (and unless `dry_run`, broadcast) an Ironwood withdraw.
pub async fn send(client: &mut LightwalletdClient, url: &str, fvk: &OrchardFvk, to: &str, amount: u64, dry_run: bool) -> Result<Value, String> {
    let (tip, _) = client.get_latest_block().await.map_err(|e| format!("latest block: {e}"))?;
    let st = sync(client, url, fvk, tip).await?;
    if st.cursor < tip {
        return Err(format!("wallet still syncing ({} of {tip})", st.cursor));
    }
    let _lock = Lock::take()?;
    let anchor_h = tip - ANCHOR_OFFSET;
    let mut cands: Vec<WNote> = st.notes.iter().filter(|n| n.spent_height.is_none() && n.height <= anchor_h).cloned().collect();
    cands.sort_by(|a, b| b.value.cmp(&a.value));
    let mut chosen: Vec<WNote> = vec![];
    let mut sum = 0u64;
    let mut fee = zip317_fee(1);
    for n in cands {
        if sum >= amount + fee {
            break;
        }
        sum += n.value;
        chosen.push(n);
        fee = zip317_fee(chosen.len());
    }
    if sum < amount + fee {
        return Err(format!("insufficient spendable ironwood: have {sum} zat, need {} zat (amount {amount} + ZIP-317 fee {fee})", amount + fee));
    }
    let recipient = zafu_wasm::parse_ironwood_recipient(to, true)?;
    // Rebuild witnesses: birthday frontier, then every ironwood cmx through the anchor.
    let front = tree_state_hex(url, st.birthday - 1).await?;
    let positions: Vec<u64> = chosen.iter().map(|n| n.position).collect();
    let mut replay = zafu_wasm::witness::WitnessReplay::from_frontier_bytes(&hex::decode(&front).map_err(|e| e.to_string())?, &positions)?;
    let ext = Scanner::from_fvk(fvk);
    let int = Scanner::from_fvk_scoped(fvk, Scope::Internal);
    let mut notes: Vec<Option<orchard::Note>> = vec![None; chosen.len()];
    let mut pos = tree_size(&front)?;
    let mut h = st.birthday;
    while h <= anchor_h {
        let end = (h + 999).min(anchor_h);
        for tx in client.get_ironwood_block_range(h, end).await.map_err(|e| format!("ironwood range: {e}"))? {
            for a in &tx.actions {
                if let Some(i) = positions.iter().position(|p| *p == pos) {
                    let (d, _) = scan_one(&ext, &int, a, tx.height).ok_or("selected note no longer decrypts")?;
                    notes[i] = Some(d.note);
                }
                replay.append_cmx_bytes(&a.cmx)?;
                pos += 1;
            }
        }
        h = end + 1;
    }
    let anchor = replay.anchor();
    let want_root = zafu_wasm::witness::compute_tree_root(&hex::decode(tree_state_hex(url, anchor_h).await?).map_err(|e| e.to_string())?)?;
    if anchor.to_bytes() != want_root {
        return Err("rebuilt anchor does not match lightwalletd tree root".into());
    }
    let paths = replay.into_paths()?;
    let mut prepared = vec![];
    for (n, p) in notes.into_iter().zip(paths) {
        prepared.push((n.ok_or("selected note not found during replay")?, p));
    }
    let (_sk, ask) = load_spend_key(fvk)?;
    let tx = zafu_wasm::build_signed_ironwood_send_core(
        zcash_protocol::consensus::MainNetwork,
        fvk,
        &ask,
        prepared,
        recipient,
        amount,
        fee,
        anchor,
        (tip + 1) as u32,
        NU6_3_BRANCH_ID,
        zcash_protocol::memo::MemoBytes::empty(),
    )?;
    let tx_hex = hex::encode(&tx);
    let txid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| zafu_wasm::compute_txid(&tx_hex).ok())).ok().flatten();
    let mut out = json!({
        "dry_run": dry_run,
        "to": to,
        "zat": amount,
        "fee_zat": fee,
        "spends": chosen.len(),
        "inputs_zat": sum,
        "change_zat": sum - amount - fee,
        "anchor_height": anchor_h,
        "target_height": tip + 1,
        "tx_bytes": tx.len(),
        "txid": txid,
    });
    if dry_run {
        return Ok(out);
    }
    let r = client.send_transaction(tx).await.map_err(|e| format!("broadcast: {e}"))?;
    if r.error_code != 0 {
        return Err(format!("lightwalletd rejected the tx: {} {}", r.error_code, r.error_message));
    }
    out["broadcast"] = json!(true);
    Ok(out)
}

/// Self-test with no spend: replay every ironwood cmx from the birthday frontier to
/// tip-3 and compare the root with lightwalletd's tree state there.
pub async fn check_anchor(client: &mut LightwalletdClient, url: &str) -> Result<Value, String> {
    let (tip, _) = client.get_latest_block().await.map_err(|e| format!("latest block: {e}"))?;
    let b = load().map(|s| s.birthday).unwrap_or_else(birthday);
    let anchor_h = tip - ANCHOR_OFFSET;
    let front = tree_state_hex(url, b - 1).await?;
    let mut replay = zafu_wasm::witness::WitnessReplay::from_frontier_bytes(&hex::decode(&front).map_err(|e| e.to_string())?, &[])?;
    let mut n = 0u64;
    let mut h = b;
    while h <= anchor_h {
        let end = (h + 999).min(anchor_h);
        for tx in client.get_ironwood_block_range(h, end).await.map_err(|e| format!("ironwood range: {e}"))? {
            for a in &tx.actions {
                replay.append_cmx_bytes(&a.cmx)?;
                n += 1;
            }
        }
        h = end + 1;
    }
    let want = zafu_wasm::witness::compute_tree_root(&hex::decode(tree_state_hex(url, anchor_h).await?).map_err(|e| e.to_string())?)?;
    Ok(json!({"anchor_height": anchor_h, "cmx_replayed": n, "root_match": replay.anchor().to_bytes() == want}))
}
