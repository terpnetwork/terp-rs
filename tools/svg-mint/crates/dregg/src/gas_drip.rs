//! Gas drip: after a ZEC-paid mint settles, send a small uthiol amount to the recipient when
//! it holds almost none, so the minter's account (passkey or wallet) can transfer, burn or
//! submit its pieces. Every settled paid mint goes through here, not only passkey accounts.
//!
//! OFF unless `SVG_MINT_PASSKEY_DRIP_UTHIOL` is set above 0 (the env name predates the wider
//! scope). It spends the runtime minter's own THIOL on mainnet.
//!
//! Limits: one drip per address ever (ledger file), a daily count cap, recipients that
//! already hold `SVG_MINT_PASSKEY_DRIP_MAX_BALANCE` uthiol or more are skipped, and the
//! minter keeps `minter_gas_reserve()` for mints. Registering the passkey authenticator on
//! the smart account is not done here: MsgInstantiateContract2 and MsgAddAuthenticator must
//! be signed by the account itself, so the page prompts for it.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DripConfig {
    /// uthiol per drip. 0 = off.
    pub amount: u128,
    /// Drips per UTC day across all addresses.
    pub daily_cap: u32,
    /// Skip recipients already holding at least this much uthiol.
    pub max_recipient_balance: u128,
}

/// `None` when the drip is off (the default).
pub fn config() -> Option<DripConfig> {
    config_from(|k| std::env::var(k).ok())
}

pub fn config_from(get: impl Fn(&str) -> Option<String>) -> Option<DripConfig> {
    let num = |k: &str, d: u128| get(k).and_then(|s| s.trim().parse().ok()).unwrap_or(d);
    let amount = num("SVG_MINT_PASSKEY_DRIP_UTHIOL", 0);
    if amount == 0 {
        return None;
    }
    Some(DripConfig {
        amount,
        daily_cap: num("SVG_MINT_PASSKEY_DRIP_DAILY_CAP", 25).min(u32::MAX as u128) as u32,
        max_recipient_balance: num("SVG_MINT_PASSKEY_DRIP_MAX_BALANCE", 50_000),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DripRow {
    pub at: u64,
    pub amount: String,
    pub tx: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ledger {
    pub rows: BTreeMap<String, DripRow>,
}

pub fn ledger_path() -> PathBuf {
    if let Ok(p) = std::env::var("SVG_MINT_PASSKEY_DRIP_LEDGER") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".terp/svg-mint-fed-drip.json")
}

impl Ledger {
    pub fn load() -> Ledger {
        std::fs::read(ledger_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        let path = ledger_path();
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
    fn today_count(&self, now: u64) -> u32 {
        let day = now / 86_400;
        self.rows.values().filter(|r| r.at / 86_400 == day).count() as u32
    }
}

/// Recent drip outcomes (sent, skipped, failed), newest last, for the ops page.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DripEvent {
    pub at: u64,
    pub address: String,
    pub outcome: &'static str,
    pub detail: String,
}

static EVENTS: std::sync::Mutex<Vec<DripEvent>> = std::sync::Mutex::new(Vec::new());
const EVENTS_KEEP: usize = 100;

fn record(address: &str, outcome: &'static str, detail: String) {
    let mut ev = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    ev.push(DripEvent { at: now(), address: address.to_string(), outcome, detail });
    let n = ev.len();
    if n > EVENTS_KEEP {
        ev.drain(..n - EVENTS_KEEP);
    }
}

/// Config, ledger and recent outcomes, for `/v1/admin/drips`. No secrets: addresses and txids only.
pub fn status_json() -> serde_json::Value {
    let cfg = config();
    let ledger = Ledger::load();
    let t = now();
    let mut rows: Vec<serde_json::Value> = ledger
        .rows
        .iter()
        .map(|(a, r)| serde_json::json!({"address": a, "at": r.at, "amount": r.amount, "tx": r.tx}))
        .collect();
    rows.sort_by(|a, b| b["at"].as_u64().cmp(&a["at"].as_u64()));
    rows.truncate(200);
    let events: Vec<DripEvent> = {
        let ev = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
        ev.iter().rev().cloned().collect()
    };
    serde_json::json!({
        "ok": true,
        "on": cfg.is_some(),
        "amount_uthiol": cfg.as_ref().map(|c| c.amount.to_string()),
        "daily_cap": cfg.as_ref().map(|c| c.daily_cap),
        "max_recipient_balance_uthiol": cfg.as_ref().map(|c| c.max_recipient_balance.to_string()),
        "minter_reserve_uthiol": crate::checkout::minter_gas_reserve().to_string(),
        "source": "runtime minter",
        "total": ledger.rows.len(),
        "today": ledger.today_count(t),
        "rows": rows,
        "events": events,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Send(u128),
    Skip(&'static str),
}

/// Pure rule, tested below. `minter_balance` and `recipient_balance` are uthiol.
pub fn decide(
    cfg: &DripConfig,
    ledger: &Ledger,
    recipient: &str,
    recipient_balance: u128,
    minter_balance: u128,
    minter_reserve: u128,
    now: u64,
) -> Decision {
    if !recipient.starts_with("terp1") {
        return Decision::Skip("not a terp1 address");
    }
    if ledger.rows.contains_key(recipient) {
        return Decision::Skip("already dripped");
    }
    if recipient_balance >= cfg.max_recipient_balance {
        return Decision::Skip("recipient has gas");
    }
    if ledger.today_count(now) >= cfg.daily_cap {
        return Decision::Skip("daily cap reached");
    }
    if minter_balance < minter_reserve.saturating_add(cfg.amount) {
        return Decision::Skip("minter below reserve");
    }
    Decision::Send(cfg.amount)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run after a settled paid mint. Logs and returns; never panics into the caller.
pub fn maybe_drip(cfg: &DripConfig, recipient: &str) {
    use std::sync::Mutex;
    static ONE: Mutex<()> = Mutex::new(());
    let _guard = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ledger = Ledger::load();
    let rb = match crate::mint_tx::balance_uthiol(recipient) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("gas drip {recipient}: balance: {e}");
            record(recipient, "error", format!("balance: {e}"));
            return;
        }
    };
    let Some(minter) = crate::mint_tx::address_if_present() else {
        eprintln!("gas drip {recipient}: no runtime minter key");
        record(recipient, "error", "no runtime minter key".into());
        return;
    };
    let mb = crate::mint_tx::balance_uthiol(&minter).unwrap_or(0);
    let t = now();
    match decide(cfg, &ledger, recipient, rb, mb, crate::checkout::minter_gas_reserve(), t) {
        Decision::Skip(why) => {
            eprintln!("gas drip {recipient}: skip ({why}; recipient {rb} uthiol, minter {mb} uthiol)");
            record(recipient, "skip", format!("{why}; recipient {rb} uthiol"));
        }
        Decision::Send(amount) => match crate::mint_tx::send_uthiol(recipient, amount) {
            Ok(tx) => {
                ledger.rows.insert(
                    recipient.to_string(),
                    DripRow { at: t, amount: amount.to_string(), tx: tx.clone() },
                );
                if let Err(e) = ledger.save() {
                    eprintln!("gas drip ledger: {e}");
                }
                eprintln!("gas drip {recipient}: sent {amount} uthiol tx {tx}");
                record(recipient, "sent", format!("{amount} uthiol tx {tx}"));
            }
            Err(e) => {
                eprintln!("gas drip {recipient}: send: {e}");
                record(recipient, "error", format!("send: {e}"));
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "terp1kppremjjtsu37uzztgtn8kwqtpl5k6y2at384n";
    fn cfg() -> DripConfig {
        DripConfig { amount: 100_000, daily_cap: 2, max_recipient_balance: 50_000 }
    }

    #[test]
    fn off_by_default() {
        assert_eq!(config_from(|_| None), None);
        assert_eq!(config_from(|k| (k == "SVG_MINT_PASSKEY_DRIP_UTHIOL").then(|| "0".into())), None);
        let on = config_from(|k| (k == "SVG_MINT_PASSKEY_DRIP_UTHIOL").then(|| "100000".into())).unwrap();
        assert_eq!(on, DripConfig { amount: 100_000, daily_cap: 25, max_recipient_balance: 50_000 });
    }

    #[test]
    fn bank_send_encodes_type_and_amount() {
        let b = crate::mint_tx::bank_send_any("terp1from", "terp1to", "uthiol", 100_000);
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("/cosmos.bank.v1beta1.MsgSend") && s.contains("terp1to") && s.contains("100000"));
    }

    #[test]
    fn sends_once_per_address() {
        let mut l = Ledger::default();
        assert_eq!(decide(&cfg(), &l, A, 0, 100_000_000, 50_000_000, 1_000), Decision::Send(100_000));
        l.rows.insert(A.into(), DripRow { at: 1_000, amount: "100000".into(), tx: "T".into() });
        assert_eq!(decide(&cfg(), &l, A, 0, 100_000_000, 50_000_000, 2_000), Decision::Skip("already dripped"));
    }

    #[test]
    fn one_thiol_drip_with_one_thiol_threshold() {
        let get = |k: &str| match k {
            "SVG_MINT_PASSKEY_DRIP_UTHIOL" => Some("1000000".to_string()),
            "SVG_MINT_PASSKEY_DRIP_MAX_BALANCE" => Some("1000000".to_string()),
            _ => None,
        };
        let c = config_from(get).unwrap();
        assert_eq!(c, DripConfig { amount: 1_000_000, daily_cap: 25, max_recipient_balance: 1_000_000 });
        let l = Ledger::default();
        // Under 1 THIOL: send 1 THIOL. At 1 THIOL or more: skip.
        assert_eq!(decide(&c, &l, A, 999_999, 121_000_000, 50_000_000, 0), Decision::Send(1_000_000));
        assert_eq!(decide(&c, &l, A, 1_000_000, 121_000_000, 50_000_000, 0), Decision::Skip("recipient has gas"));
        // Never below the 50 THIOL reserve.
        assert_eq!(decide(&c, &l, A, 0, 50_999_999, 50_000_000, 0), Decision::Skip("minter below reserve"));
        assert_eq!(decide(&c, &l, A, 0, 51_000_000, 50_000_000, 0), Decision::Send(1_000_000));
    }

    #[test]
    fn status_lists_events() {
        record("terp1x", "skip", "already dripped".into());
        let v = status_json();
        assert!(v["events"].as_array().unwrap().iter().any(|e| e["address"] == "terp1x" && e["outcome"] == "skip"));
        assert!(v.get("minter_reserve_uthiol").is_some());
    }

    #[test]
    fn limits() {
        let mut l = Ledger::default();
        assert_eq!(decide(&cfg(), &l, A, 50_000, 100_000_000, 50_000_000, 0), Decision::Skip("recipient has gas"));
        assert_eq!(decide(&cfg(), &l, A, 0, 50_050_000, 50_000_000, 0), Decision::Skip("minter below reserve"));
        assert_eq!(decide(&cfg(), &l, "cosmos1x", 0, 100_000_000, 50_000_000, 0), Decision::Skip("not a terp1 address"));
        for a in ["terp1a", "terp1b"] {
            l.rows.insert(a.into(), DripRow { at: 86_400 * 3 + 5, amount: "1".into(), tx: "T".into() });
        }
        assert_eq!(decide(&cfg(), &l, A, 0, 100_000_000, 50_000_000, 86_400 * 3 + 99), Decision::Skip("daily cap reached"));
        assert_eq!(decide(&cfg(), &l, A, 0, 100_000_000, 50_000_000, 86_400 * 4), Decision::Send(100_000));
    }
}
