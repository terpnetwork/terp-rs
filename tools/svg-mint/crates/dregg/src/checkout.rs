//! Checkout invariants: FE must not open Pay unless the runtime can mint.
//!
//! `GET /health` and `GET /v1/status` expose `checkout_ready`. Pay-targets
//! refuse when it is false. Token count is `floor(paid / unit)`; extra dust
//! does not mint. Large qty is split into mint batches. Entitled deposits
//! that never settle are refunded after `mint_timeout_secs`. Unpaid pay-targets
//! expire after `pending_ttl_secs` so abandoned carts stop being watched.

use crate::prices::{self, NATIVE_AMOUNT};
use crate::zec_tip::{self, ZecTip};

/// Max NFTs per on-chain mint tx (large qty is split so the mempool can breathe).
pub fn mint_batch_size() -> u32 {
    std::env::var("DREGG_MINT_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(5)
}

/// Seconds after entitle without a confirmed mint before DREGG refunds.
pub fn mint_timeout_secs() -> u64 {
    std::env::var("DREGG_MINT_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20 * 60)
}

/// Seconds after pay-targets before an unpaid HD dest is abandoned.
/// `0` disables (ops escape). Default 10 minutes so abandoned carts clear.
pub fn pending_ttl_secs() -> u64 {
    std::env::var("DREGG_PENDING_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10 * 60)
}

/// Unpaid cart past TTL. `issued_at == 0` is a pre-TTL snapshot with unknown
/// age — treat as stale so abandoned carts cannot linger.
pub fn pending_expired(issued_at: u64, now: u64, ttl_secs: u64) -> bool {
    if ttl_secs == 0 {
        return false;
    }
    if issued_at == 0 {
        return true;
    }
    now.saturating_sub(issued_at) >= ttl_secs
}

/// Extra native the runtime keeps for gas on top of `NATIVE_AMOUNT` per NFT.
pub fn minter_gas_reserve() -> u128 {
    std::env::var("DREGG_MINTER_GAS_RESERVE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000_000)
}

/// Pieces control on Pay: `?qty=N` on pay-targets. Clamp 1…20.
pub const QTY_MIN: u32 = 1;
pub const QTY_MAX: u32 = 20;

pub fn parse_qty(raw: Option<&str>) -> u32 {
    let n = raw
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(QTY_MIN);
    n.clamp(QTY_MIN, QTY_MAX)
}

/// Whole paid units (floor). Underpay → 0.
pub fn units_paid(amount: u128, unit_price: u128) -> u64 {
    if unit_price == 0 {
        return 0;
    }
    (amount / unit_price) as u64
}

/// NFTs for `units` paid, advancing BOGO for each unit.
pub fn nfts_for_units(units: u64, mut bogo_seen: u64) -> u64 {
    let mut n = 0u64;
    for _ in 0..units {
        n = n.saturating_add(prices::nft_count_for_rail(bogo_seen) as u64);
        bogo_seen = bogo_seen.saturating_add(1);
    }
    n
}

/// Split `nfts` into batches of `batch` (last batch may be smaller).
pub fn mint_batches(nfts: u64, batch: u32) -> Vec<u32> {
    let batch = batch.max(1) as u64;
    let mut out = Vec::new();
    let mut left = nfts;
    while left > 0 {
        let n = left.min(batch) as u32;
        out.push(n);
        left -= n as u64;
    }
    out
}

pub fn native_needed_for_nfts(nfts: u64) -> u128 {
    NATIVE_AMOUNT
        .saturating_mul(nfts as u128)
        .saturating_add(minter_gas_reserve())
}

/// Serializes test mutations of minter LCD/BALANCE env (integration + unit).
pub fn minter_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// Prefer live LCD `DREGG_MINTER_LCD` + `DREGG_MINTER_ADDR`.
/// If LCD is set, a miss (`https not used`, HTTP fail, bad JSON) is `None` —
/// never fall through to `DREGG_MINTER_BALANCE` as live proof.
/// `DREGG_MINTER_BALANCE` is only used when LCD is unset.
pub fn minter_balance_native() -> Option<u128> {
    if std::env::var("DREGG_MINTER_LCD")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some()
    {
        return query_minter_lcd();
    }
    std::env::var("DREGG_MINTER_BALANCE")
        .ok()
        .and_then(|s| s.parse().ok())
}

fn query_minter_lcd() -> Option<u128> {
    let lcd = std::env::var("DREGG_MINTER_LCD").ok().filter(|s| !s.is_empty())?;
    let addr = std::env::var("DREGG_MINTER_ADDR").ok().filter(|s| s.starts_with("terp1"))?;
    let denom = prices::native_denom();
    let url = format!(
        "{}/cosmos/bank/v1beta1/balances/{}",
        lcd.trim_end_matches('/'),
        addr
    );
    let v: serde_json::Value = crate::httpx::get_json(&url).ok()?;
    let bals = v.get("balances")?.as_array()?;
    for c in bals {
        if c.get("denom").and_then(|d| d.as_str()) == Some(denom.as_str()) {
            return c.get("amount").and_then(|a| a.as_str()).and_then(|s| s.parse().ok());
        }
    }
    Some(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckoutStatus {
    pub checkout_ready: bool,
    pub minter_funded: bool,
    pub minter_balance: u128,
    pub remaining_supply: u64,
    pub minted: u64,
    pub max_supply: u64,
    pub collection_bound: bool,
    pub mint_batch_size: u32,
    pub mint_timeout_secs: u64,
    pub pending_ttl_secs: u64,
    pub reasons: Vec<String>,
    pub zec_tip: ZecTip,
}

impl CheckoutStatus {
    pub fn evaluate(
        collection_bound: bool,
        minted: u64,
        max_supply: u64,
        minter_balance: Option<u128>,
    ) -> Self {
        let remaining = if max_supply == 0 {
            0
        } else {
            max_supply.saturating_sub(minted)
        };
        let mut reasons = Vec::new();
        if !collection_bound || max_supply == 0 {
            reasons.push("collection_unbound".into());
        }
        if remaining == 0 {
            reasons.push("sold_out".into());
        }
        let (minter_funded, bal) = match minter_balance {
            Some(b) => {
                // Cover at least one BOGO pair (or 1 after cap) plus gas — not a single NFT.
                let cover = prices::nft_count_for_rail(0) as u64;
                let need = native_needed_for_nfts(cover);
                if b < need {
                    reasons.push("minter_underfunded".into());
                    (false, b)
                } else {
                    (true, b)
                }
            }
            None => {
                reasons.push("minter_balance_unknown".into());
                (false, 0)
            }
        };
        let checkout_ready = reasons.is_empty();
        Self {
            checkout_ready,
            minter_funded,
            minter_balance: bal,
            remaining_supply: remaining,
            minted,
            max_supply,
            collection_bound,
            mint_batch_size: mint_batch_size(),
            mint_timeout_secs: mint_timeout_secs(),
            pending_ttl_secs: pending_ttl_secs(),
            reasons,
            zec_tip: ZecTip::skipped(zec_tip::max_drift()),
        }
    }

    /// Record ZEC house-vs-public tip. Drift hides the ZEC rail (`zec_tip.ok`)
    /// but does not close native TERP checkout. Pay-targets must omit
    /// `zec_uri` while `zec_tip.ok` is false so the house node is not paid.
    pub fn apply_zec_tip(&mut self, tip: ZecTip) {
        if tip.configured && !tip.ok {
            if !self.reasons.iter().any(|r| r == zec_tip::REASON_DRIFT) {
                self.reasons.push(zec_tip::REASON_DRIFT.into());
            }
        }
        self.zec_tip = tip;
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "checkout_ready": self.checkout_ready,
            "minter_funded": self.minter_funded,
            "minter_balance": self.minter_balance.to_string(),
            "remaining_supply": self.remaining_supply,
            "minted": self.minted,
            "max_supply": self.max_supply,
            "collection_bound": self.collection_bound,
            "mint_batch_size": self.mint_batch_size,
            "mint_timeout_secs": self.mint_timeout_secs,
            "pending_ttl_secs": self.pending_ttl_secs,
            "unit_native": NATIVE_AMOUNT.to_string(),
            "reasons": self.reasons,
            "zec_tip": self.zec_tip.to_json(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_units() {
        assert_eq!(units_paid(0, 1500), 0);
        assert_eq!(units_paid(1499, 1500), 0);
        assert_eq!(units_paid(1500, 1500), 1);
        assert_eq!(units_paid(4500, 1500), 3);
        assert_eq!(units_paid(4501, 1500), 3);
        assert_eq!(units_paid(450_000, crate::prices::ZEC_ZAT as u128), 3);
    }

    #[test]
    fn qty_clamp_1_to_20() {
        assert_eq!(parse_qty(None), 1);
        assert_eq!(parse_qty(Some("")), 1);
        assert_eq!(parse_qty(Some("3")), 3);
        assert_eq!(parse_qty(Some("20")), 20);
        assert_eq!(parse_qty(Some("99")), 20);
        assert_eq!(parse_qty(Some("0")), 1);
    }

    #[test]
    fn bogo_units() {
        assert_eq!(nfts_for_units(1, 0), 2);
        assert_eq!(nfts_for_units(2, 0), 4);
        assert_eq!(nfts_for_units(1, 1000), 1);
        assert_eq!(nfts_for_units(3, 999), 2 + 1 + 1);
    }

    #[test]
    fn batches() {
        assert_eq!(mint_batches(0, 5), Vec::<u32>::new());
        assert_eq!(mint_batches(3, 5), vec![3]);
        assert_eq!(mint_batches(12, 5), vec![5, 5, 2]);
        assert_eq!(mint_batches(5, 5), vec![5]);
    }

    #[test]
    fn blocks_unfunded() {
        let s = CheckoutStatus::evaluate(true, 0, 100, Some(0));
        assert!(!s.checkout_ready);
        assert!(s.reasons.iter().any(|r| r == "minter_underfunded"));
    }

    #[test]
    fn blocks_unknown_balance() {
        let s = CheckoutStatus::evaluate(true, 0, 100, None);
        assert!(!s.checkout_ready);
        assert!(s.reasons.iter().any(|r| r == "minter_balance_unknown"));
    }

    #[test]
    fn ready_when_funded_and_supply() {
        let s = CheckoutStatus::evaluate(true, 1, 10, Some(NATIVE_AMOUNT * 10));
        assert!(s.checkout_ready);
        assert_eq!(s.remaining_supply, 9);
        assert!(s.reasons.is_empty());
    }

    #[test]
    fn bogo_pair_needs_two_nfts_of_native() {
        let one = native_needed_for_nfts(1);
        let s = CheckoutStatus::evaluate(true, 0, 10, Some(one));
        assert!(!s.checkout_ready);
        assert!(s.reasons.iter().any(|r| r == "minter_underfunded"));
        let two = native_needed_for_nfts(2);
        let s = CheckoutStatus::evaluate(true, 0, 10, Some(two));
        assert!(s.checkout_ready);
    }

    #[test]
    fn blocks_sold_out() {
        let s = CheckoutStatus::evaluate(true, 10, 10, Some(u128::MAX / 8));
        assert!(!s.checkout_ready);
        assert!(s.reasons.iter().any(|r| r == "sold_out"));
    }

    #[test]
    fn health_and_status_json_expose_checkout_ready() {
        let s = CheckoutStatus::evaluate(true, 0, 100, Some(NATIVE_AMOUNT * 10));
        let j = s.to_json();
        assert_eq!(j["checkout_ready"], true);
        assert_eq!(j["minter_funded"], true);
        assert!(j.get("mint_timeout_secs").is_some());
        assert!(j.get("pending_ttl_secs").is_some());
        assert!(j.get("reasons").is_some());
        assert_eq!(j["zec_tip"]["ok"], true);
        assert_eq!(j["zec_tip"]["configured"], false);
        let closed = CheckoutStatus::evaluate(false, 0, 0, None);
        let j2 = closed.to_json();
        assert_eq!(j2["checkout_ready"], false);
        assert_eq!(j2["minter_funded"], false);
        // FE must block Pay; /health is 503 ok:false when closed (fed).
        assert!(!closed.checkout_ready);
        let err = format!("checkout closed: {}", closed.reasons.join(","));
        assert!(err.starts_with("checkout closed"));
    }

    #[test]
    fn unpaid_cart_ttl() {
        assert!(pending_expired(0, 10_000, 60));
        assert!(!pending_expired(0, 10_000, 0));
        assert!(!pending_expired(100, 159, 60));
        assert!(pending_expired(100, 160, 60));
        assert!(!pending_expired(100, 10_000, 0));
    }

    #[test]
    fn pending_ttl_default_ten_minutes() {
        let _g = minter_env_lock();
        std::env::remove_var("DREGG_PENDING_TTL_SECS");
        assert_eq!(pending_ttl_secs(), 10 * 60);
    }

    #[test]
    fn minter_funded_fail_closed() {
        let unknown = CheckoutStatus::evaluate(true, 0, 10, None);
        assert!(!unknown.minter_funded);
        assert!(!unknown.checkout_ready);
        let funded = CheckoutStatus::evaluate(true, 0, 10, Some(u128::MAX / 4));
        assert!(funded.minter_funded);
        assert!(funded.checkout_ready);
        let dust = CheckoutStatus::evaluate(true, 0, 10, Some(1));
        assert!(!dust.minter_funded);
        assert!(dust.reasons.iter().any(|r| r == "minter_underfunded"));
    }

    #[test]
    fn minter_balance_native_env() {
        let _g = minter_env_lock();
        std::env::remove_var("DREGG_MINTER_FUNDED");
        std::env::remove_var("DREGG_MINTER_BALANCE");
        std::env::remove_var("DREGG_MINTER_LCD");
        std::env::remove_var("DREGG_MINTER_ADDR");
        assert_eq!(minter_balance_native(), None);
        std::env::set_var("DREGG_MINTER_BALANCE", "12345");
        assert_eq!(minter_balance_native(), Some(12345));
        std::env::set_var("DREGG_MINTER_FUNDED", "1");
        // FUNDED is not a funded proof (BALANCE still wins only because LCD is unset).
        assert_eq!(minter_balance_native(), Some(12345));
        std::env::remove_var("DREGG_MINTER_BALANCE");
        assert_eq!(minter_balance_native(), None);
        std::env::remove_var("DREGG_MINTER_FUNDED");
    }

    #[test]
    fn lcd_https_miss_is_unknown_not_balance() {
        let _g = minter_env_lock();
        std::env::set_var("DREGG_MINTER_LCD", "https://127.0.0.1:1317");
        std::env::set_var(
            "DREGG_MINTER_ADDR",
            "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqp3wq3h",
        );
        std::env::set_var("DREGG_MINTER_BALANCE", "999999999");
        std::env::set_var("DREGG_MINTER_FUNDED", "1");
        assert_eq!(minter_balance_native(), None);
        let s = CheckoutStatus::evaluate(true, 0, 10, minter_balance_native());
        assert!(!s.checkout_ready);
        assert!(s.reasons.iter().any(|r| r == "minter_balance_unknown"));
        std::env::remove_var("DREGG_MINTER_LCD");
        std::env::remove_var("DREGG_MINTER_ADDR");
        std::env::remove_var("DREGG_MINTER_BALANCE");
        std::env::remove_var("DREGG_MINTER_FUNDED");
    }

    #[test]
    fn zec_tip_drift_hides_zec_keeps_native() {
        let mut s = CheckoutStatus::evaluate(true, 0, 10, Some(NATIVE_AMOUNT * 10));
        assert!(s.checkout_ready);
        s.apply_zec_tip(ZecTip::from_heights(10, 13, 2));
        assert!(s.checkout_ready);
        assert!(s.reasons.iter().any(|r| r == zec_tip::REASON_DRIFT));
        assert_eq!(s.to_json()["zec_tip"]["ok"], false);
        assert_eq!(s.to_json()["zec_tip"]["reason"], "drift");
        let mut ok = CheckoutStatus::evaluate(true, 0, 10, Some(NATIVE_AMOUNT * 10));
        ok.apply_zec_tip(ZecTip::from_heights(10, 12, 2));
        assert!(ok.checkout_ready);
        assert!(ok.reasons.is_empty());
    }
}
