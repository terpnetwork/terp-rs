//! Spend seed, unified address, and the Ironwood note ledger.
//!
//! The seed file is the account. Restarting the process opens that file.
//! It does not generate a replacement.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub const DONATION_UA: &str = "u1uml44spc4nn732t6605wjcwu3wn9dla4aehpyrvnhssq2ew3dk8mqk5e9jhnndzfny7fkrkx5ape05nzf8gwruqykkvc27fm0v9ln4hfrwkmtfrs27r06xu0uc9eneh777le3m8362v7xaurgtkgxtspy5eqhgpzw6axjwgwdy5wjdjt";

const SEED_FILE: &str = "spend.seed";
const ACCOUNT_FILE: &str = "account.json";
const NOTES_FILE: &str = "notes.sqlite";

#[derive(Clone, Debug)]
pub struct Account {
    pub dir: PathBuf,
    pub address: String,
    pub diversifier_index: u32,
    pub receiver_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendPlan {
    pub zat: u64,
    pub address: String,
    pub nullifiers: Vec<String>,
}

pub fn dir_from_env() -> PathBuf {
    if let Ok(p) = std::env::var("SVG_MINT_ZRT_DIR") {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".terp").join("svg-mint-zrt")
}

pub fn init(dir: &Path) -> Result<Account, String> {
    if seed_path(dir).is_file() {
        return Err("wallet already exists; refusing to generate a new spend seed".into());
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    set_dir_mode(dir);
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| format!("seed entropy: {e}"))?;
    write_seed_new(&seed_path(dir), &seed)?;
    let derived = derive(&seed)?;
    write_account(dir, derived.0, &derived.2)?;
    open_db(dir)?;
    // Drop the seed bytes before returning. The account is the address, not the key.
    seed.fill(0);
    open(dir)
}

pub fn open(dir: &Path) -> Result<Account, String> {
    let path = seed_path(dir);
    if !path.is_file() {
        return Err(
            "spend seed missing; refusing to create a wallet (run init once, against this directory)"
                .into(),
        );
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if bytes.len() != 32 {
        return Err("spend seed must be 32 bytes; refusing to replace it".into());
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    let derived = derive(&seed)?;
    seed.fill(0);
    let stored = read_account(dir)?;
    if stored.0 != derived.0 || stored.1 != derived.2 {
        return Err(
            "account file does not match the spend seed; refusing to continue".into(),
        );
    }
    open_db(dir)?;
    Ok(Account {
        dir: dir.to_path_buf(),
        address: derived.2,
        diversifier_index: derived.0,
        receiver_hex: hex::encode(derived.1),
    })
}

pub fn balance(dir: &Path) -> Result<Value, String> {
    let db = open_db(dir)?;
    let iron: i64 = db
        .query_row(
            "select coalesce(sum(zat), 0) from notes where mined = 1 and spent = 0 and pool = 'ironwood'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let iron = u64::try_from(iron).unwrap_or(0);
    let height: Option<i64> = db
        .query_row(
            "select cast(v as integer) from meta where k = 'chain_tip_height'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(json!({
        "total": iron,
        "sapling_spendable": 0,
        "orchard_spendable": 0,
        "ironwood_spendable": iron,
        "transparent_spendable": 0,
        "chain_tip_height": height,
    }))
}

pub fn notes_json(dir: &Path) -> Result<Value, String> {
    let acct = open(dir)?;
    let db = open_db(dir)?;
    let mut stmt = db
        .prepare(
            "select nullifier, txid, receiver, address, memo, zat, mined, spent, pool from notes order by rowid",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "nullifier": r.get::<_, String>(0)?,
                "txid": r.get::<_, String>(1)?,
                "receiver": r.get::<_, String>(2)?,
                "address": r.get::<_, String>(3)?,
                "memo": r.get::<_, String>(4)?,
                "zat": r.get::<_, i64>(5)?,
                "mined": r.get::<_, i64>(6)? != 0,
                "spent": r.get::<_, i64>(7)? != 0,
                "pool": r.get::<_, String>(8)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    let mut notes = Vec::new();
    for row in rows {
        notes.push(row.map_err(|e| e.to_string())?);
    }
    Ok(json!({
        "address": acct.address,
        "diversifier_index": acct.diversifier_index,
        "notes": notes,
    }))
}

/// Record a note the scanner already trial-decrypted. Does not invent value.
pub fn credit_ironwood(
    dir: &Path,
    nullifier_hex: &str,
    txid: &str,
    zat: u64,
    mined: bool,
    memo: &str,
) -> Result<(), String> {
    let acct = open(dir)?;
    if zat == 0 {
        return Err("note value must be non-zero".into());
    }
    let nullifier = normalize_hex(nullifier_hex, 32, "nullifier")?;
    if txid.is_empty() {
        return Err("txid required".into());
    }
    let db = open_db(dir)?;
    db.execute(
        "insert into notes (nullifier, txid, receiver, address, memo, zat, mined, spent, pool, spent_txid)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'ironwood', null)",
        params![
            nullifier,
            txid,
            acct.receiver_hex,
            acct.address,
            memo,
            zat as i64,
            if mined { 1 } else { 0 }
        ],
    )
    .map_err(|e| format!("credit: {e}"))?;
    Ok(())
}

/// Select unspent mined Ironwood notes. Does not mark them spent.
pub fn prepare_send(dir: &Path, zat: u64, address: &str) -> Result<SendPlan, String> {
    let _acct = open(dir)?;
    if zat == 0 {
        return Err("send zat must be non-zero".into());
    }
    if address != DONATION_UA {
        return Err("send refuses dest that is not the donation UA".into());
    }
    let db = open_db(dir)?;
    let mut stmt = db
        .prepare(
            "select nullifier, zat from notes where mined = 1 and spent = 0 and pool = 'ironwood' order by rowid",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut have = 0u64;
    let mut nullifiers = Vec::new();
    for row in rows {
        let (nf, z) = row.map_err(|e| e.to_string())?;
        let z = u64::try_from(z).unwrap_or(0);
        nullifiers.push(nf);
        have = have.saturating_add(z);
        if have >= zat {
            break;
        }
    }
    if have < zat {
        return Err(format!("insufficient ironwood spendable: have {have} need {zat}"));
    }
    Ok(SendPlan {
        zat,
        address: address.to_string(),
        nullifiers,
    })
}

/// Mark notes spent only after a transaction id exists. A failed send must not call this.
pub fn commit_spend(dir: &Path, nullifiers: &[String], spent_txid: &str) -> Result<(), String> {
    if spent_txid.is_empty() {
        return Err("spend txid required".into());
    }
    if nullifiers.is_empty() {
        return Err("no notes to mark spent".into());
    }
    let db = open_db(dir)?;
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    for nf in nullifiers {
        let n = tx
            .execute(
                "update notes set spent = 1, spent_txid = ?2 where nullifier = ?1 and spent = 0 and pool = 'ironwood'",
                params![nf, spent_txid],
            )
            .map_err(|e| e.to_string())?;
        if n != 1 {
            return Err(format!("note {nf} was not an unspent ironwood note"));
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

fn seed_path(dir: &Path) -> PathBuf {
    dir.join(SEED_FILE)
}

fn write_seed_new(path: &Path, seed: &[u8; 32]) -> Result<(), String> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(|e| format!("create spend seed: {e}"))?;
    f.write_all(seed).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

fn set_dir_mode(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
}

fn write_account(dir: &Path, diversifier_index: u32, address: &str) -> Result<(), String> {
    let body = json!({
        "v": 1,
        "diversifier_index": diversifier_index,
        "address": address,
    });
    let path = dir.join(ACCOUNT_FILE);
    fs::write(&path, serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn read_account(dir: &Path) -> Result<(u32, String), String> {
    let raw = fs::read(dir.join(ACCOUNT_FILE)).map_err(|e| format!("account file: {e}"))?;
    let v: Value = serde_json::from_slice(&raw).map_err(|e| format!("account json: {e}"))?;
    let index = v
        .get("diversifier_index")
        .and_then(|n| n.as_u64())
        .ok_or("account missing diversifier_index")? as u32;
    let address = v
        .get("address")
        .and_then(|s| s.as_str())
        .ok_or("account missing address")?
        .to_string();
    Ok((index, address))
}

fn open_db(dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let db = Connection::open(dir.join(NOTES_FILE)).map_err(|e| e.to_string())?;
    db.execute_batch(
        "create table if not exists meta (k text primary key, v text not null);
         create table if not exists notes (
           nullifier text primary key,
           txid text not null,
           receiver text not null,
           address text not null,
           memo text not null,
           zat integer not null,
           mined integer not null,
           spent integer not null,
           pool text not null,
           spent_txid text
         );",
    )
    .map_err(|e| e.to_string())?;
    Ok(db)
}

fn normalize_hex(s: &str, nbytes: usize, what: &str) -> Result<String, String> {
    let h = s.trim().trim_start_matches("0x");
    let raw = hex::decode(h).map_err(|_| format!("{what} is not hex"))?;
    if raw.len() != nbytes {
        return Err(format!("{what} must be {nbytes} bytes"));
    }
    Ok(hex::encode(raw))
}

/// ZIP-32 account 0, coin type 133.
///
/// orchard 0.16 accepts every 11-byte diversifier, so index 0 is stored and reused.
/// The unified address is an Orchard receiver. zcash_address 0.13 has no Ironwood
/// receiver type. This address is not a payment target until an Ironwood scan exists.
fn derive(seed: &[u8; 32]) -> Result<(u32, [u8; 43], String), String> {
    use orchard::keys::{FullViewingKey, Scope, SpendingKey};
    let sk = SpendingKey::from_zip32_seed(seed, 133, zip32::AccountId::ZERO)
        .map_err(|e| format!("zip32: {e}"))?;
    let fvk = FullViewingKey::from(&sk);
    let index = 0u32;
    let div = orchard::keys::Diversifier::from_bytes(diversifier_bytes(seed, index));
    let addr = fvk.address(div, Scope::External);
    let receiver = addr.to_raw_address_bytes();
    let ua = unified_orchard(&receiver)?;
    Ok((index, receiver, ua))
}

fn diversifier_bytes(seed: &[u8], index: u32) -> [u8; 11] {
    let mut h = blake3::Hasher::new();
    h.update(b"svg-mint-zrt-div-v1");
    h.update(seed);
    h.update(&index.to_le_bytes());
    let out = h.finalize();
    let mut d = [0u8; 11];
    d.copy_from_slice(&out.as_bytes()[..11]);
    d
}

fn unified_orchard(receiver: &[u8; 43]) -> Result<String, String> {
    use zcash_address::unified::{Address, Encoding, Receiver};
    use zcash_protocol::consensus::NetworkType;
    let addr = Address::try_from_items(vec![Receiver::Orchard(*receiver)])
        .map_err(|e| format!("unified address: {e}"))?;
    Ok(addr.encode(&NetworkType::Main))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "svg-mint-zrt-test-{}-{}",
            std::process::id(),
            rand_tag()
        ));
        let _ = fs::remove_dir_all(&p);
        p
    }

    fn rand_tag() -> u32 {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).unwrap();
        u32::from_le_bytes(b)
    }

    #[test]
    fn default_dir_is_under_terp_not_tmp() {
        let prev = std::env::var("SVG_MINT_ZRT_DIR").ok();
        std::env::remove_var("SVG_MINT_ZRT_DIR");
        let d = dir_from_env();
        assert!(d.ends_with("svg-mint-zrt"));
        assert!(d.components().any(|c| c.as_os_str() == ".terp"));
        assert!(!d.starts_with("/tmp"));
        if let Some(p) = prev {
            std::env::set_var("SVG_MINT_ZRT_DIR", p);
        }
    }

    #[test]
    fn missing_seed_does_not_create_one() {
        let dir = scratch();
        fs::create_dir_all(&dir).unwrap();
        let err = open(&dir).unwrap_err();
        assert!(err.contains("refusing to create"));
        assert!(!seed_path(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restart_keeps_the_same_address_and_balance() {
        let dir = scratch();
        let first = init(&dir).unwrap();
        let again = open(&dir).unwrap();
        assert_eq!(first.address, again.address);
        assert!(first.address.starts_with("u1"));
        assert_eq!(first.diversifier_index, again.diversifier_index);
        let seed_before = fs::read(seed_path(&dir)).unwrap();
        let err = init(&dir).unwrap_err();
        assert!(err.contains("refusing"));
        assert_eq!(fs::read(seed_path(&dir)).unwrap(), seed_before);

        credit_ironwood(&dir, &hex::encode([7u8; 32]), "tx-1", 150_000, true, "").unwrap();
        let bal = balance(&dir).unwrap();
        assert_eq!(bal["ironwood_spendable"], 150_000);
        let addr = first.address.clone();
        drop(first);
        let bal2 = balance(&dir).unwrap();
        assert_eq!(bal2["total"], 150_000);

        let plan = prepare_send(&dir, 100_000, DONATION_UA).unwrap();
        assert_eq!(plan.nullifiers.len(), 1);
        let bal3 = balance(&dir).unwrap();
        assert_eq!(bal3["total"], 150_000, "prepare_send must not spend");

        let other = "u1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
        assert!(prepare_send(&dir, 1, other).unwrap_err().contains("donation"));
        assert_eq!(balance(&dir).unwrap()["total"], 150_000);

        commit_spend(&dir, &plan.nullifiers, "spend-tx").unwrap();
        assert_eq!(balance(&dir).unwrap()["total"], 0);
        let notes = notes_json(&dir).unwrap();
        assert_eq!(notes["notes"][0]["spent"], true);
        assert_eq!(open(&dir).unwrap().address, addr);
        assert_eq!(balance(&dir).unwrap()["total"], 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unmined_note_is_not_spendable() {
        let dir = scratch();
        init(&dir).unwrap();
        credit_ironwood(&dir, &hex::encode([9u8; 32]), "tx-m", 50_000, false, "memo").unwrap();
        assert_eq!(balance(&dir).unwrap()["total"], 0);
        assert!(prepare_send(&dir, 1, DONATION_UA).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
