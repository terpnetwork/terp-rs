//! Passkey gas drip: after a paid mint settles, send a small uthiol amount to a recipient
//! that holds almost none, so a passkey-only account can transfer, burn or submit its pieces.
//!
//! OFF unless `SVG_MINT_PASSKEY_DRIP_UTHIOL` is set above 0. It spends the runtime minter's
//! own THIOL on mainnet, so it stays off until the operator turns it on.
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
            eprintln!("passkey drip {recipient}: balance: {e}");
            return;
        }
    };
    let Some(minter) = crate::mint_tx::address_if_present() else {
        return;
    };
    let mb = crate::mint_tx::balance_uthiol(&minter).unwrap_or(0);
    let t = now();
    match decide(cfg, &ledger, recipient, rb, mb, crate::checkout::minter_gas_reserve(), t) {
        Decision::Skip(why) => eprintln!("passkey drip {recipient}: skip ({why})"),
        Decision::Send(amount) => match crate::mint_tx::send_uthiol(recipient, amount) {
            Ok(tx) => {
                ledger.rows.insert(
                    recipient.to_string(),
                    DripRow { at: t, amount: amount.to_string(), tx: tx.clone() },
                );
                if let Err(e) = ledger.save() {
                    eprintln!("passkey drip ledger: {e}");
                }
                eprintln!("passkey drip {recipient}: sent {amount} uthiol tx {tx}");
            }
            Err(e) => eprintln!("passkey drip {recipient}: send: {e}"),
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
