//! Product mint prices (DREGG is SSOT). FE queries `GET /v1/prices`, do not hardcode.

/// ETH / Base: 0.0005 ether → wei
pub const ETH_WEI: u128 = 500_000_000_000_000;
/// BTC: 0.000015 bitcoin → sats
pub const BTC_SATS: u64 = 1_500;
/// ZEC: 0.0015 ZEC → zatoshis
pub const ZEC_ZAT: u64 = 150_000;
/// Native 100 TERP or 100 THIOL (6 decimals)
pub const NATIVE_AMOUNT: u128 = 100_000_000;
/// First N rail deposits mint two NFTs (buy one get one).
pub const CROSS_CHAIN_BOGO_CAP: u64 = 1000;

pub fn eth_wei() -> String {
    std::env::var("DREGG_ETH_WEI").unwrap_or_else(|_| ETH_WEI.to_string())
}

/// Base wei: `DREGG_BASE_WEI` or the ETH amount (same product price unless overridden).
pub fn base_wei() -> String {
    std::env::var("DREGG_BASE_WEI").unwrap_or_else(|_| eth_wei())
}

pub fn base_chain_id() -> u64 {
    std::env::var("DREGG_BASE_CHAIN_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8453)
}
pub fn btc_sats() -> u64 {
    std::env::var("DREGG_BTC_SATS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(BTC_SATS)
}
pub fn zec_zat() -> u64 {
    std::env::var("DREGG_ZEC_ZAT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(ZEC_ZAT)
}
pub fn native_denom() -> String {
    std::env::var("GAS_DENOM").unwrap_or_else(|_| "uthiol".into())
}

/// permissionless.money fractal (code 98). Logos are not on this desk.
pub const FRACTAL_COLLECTION: &str =
    "terp15anc99l7lvk2v2pzjusnmylr3lw3xnpxg89xv8uh0fj6pwrumrksqwzs6x";

/// Default zat per fractal tier: 0.001, 0.005, 0.01 ZEC.
pub const FRACTAL_ZEC_ZAT_DEFAULT: [u64; 3] = [100_000, 500_000, 1_000_000];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FractalTier {
    pub from: u64,
    pub until: u64,
    pub thiol: u64,
    pub zat: u64,
}

impl FractalTier {
    pub fn uthiol(self) -> u128 {
        self.thiol as u128 * 1_000_000
    }

    fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "from": self.from,
            "until": self.until,
            "thiol": self.thiol.to_string(),
            "uthiol": self.uthiol().to_string(),
            "zat": self.zat,
            "zec": crate::zip321::zat_to_zec_decimal(self.zat),
        })
    }
}

/// `DREGG_FRACTAL_ZEC_ZAT` is three positive zat amounts, comma-separated.
/// A missing or malformed value keeps [`FRACTAL_ZEC_ZAT_DEFAULT`].
pub fn fractal_zec_zat() -> [u64; 3] {
    let Ok(raw) = std::env::var("DREGG_FRACTAL_ZEC_ZAT") else {
        return FRACTAL_ZEC_ZAT_DEFAULT;
    };
    let parts: Vec<u64> = raw
        .split(',')
        .map(str::trim)
        .filter_map(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .collect();
    if parts.len() == 3 {
        [parts[0], parts[1], parts[2]]
    } else {
        FRACTAL_ZEC_ZAT_DEFAULT
    }
}

pub fn fractal_tiers() -> [FractalTier; 3] {
    let z = fractal_zec_zat();
    [
        FractalTier {
            from: 1,
            until: 1_000,
            thiol: 100,
            zat: z[0],
        },
        FractalTier {
            from: 1_001,
            until: 5_000,
            thiol: 500,
            zat: z[1],
        },
        FractalTier {
            from: 5_001,
            until: 10_000,
            thiol: 1_000,
            zat: z[2],
        },
    ]
}

/// Tier for the next mint. `minted` is on-chain `num_tokens` (0 before #1).
pub fn tier_for_next(minted: u64) -> Option<FractalTier> {
    let next = minted.saturating_add(1);
    fractal_tiers()
        .into_iter()
        .find(|t| next >= t.from && next <= t.until)
}

fn tier_json(minted: Option<u64>) -> serde_json::Value {
    let tiers: Vec<_> = fractal_tiers().into_iter().map(|t| t.to_json()).collect();
    let active = minted.and_then(tier_for_next).map(|t| t.to_json());
    serde_json::json!({
        "mode": "fixed_per_tier",
        "collection": FRACTAL_COLLECTION,
        "minted": minted,
        "per_tier": tiers,
        "active": active,
    })
}

/// Unit ZEC quote for a pay-target. Fractal uses the tier for `minted`.
/// Any other collection keeps the flat rail amount.
pub fn unit_zat(collection: &str, minted: Option<u64>) -> Option<u64> {
    if collection == FRACTAL_COLLECTION {
        return minted.and_then(tier_for_next).map(|t| t.zat);
    }
    Some(zec_zat())
}

/// Unit native price in `uthiol` for a pay-target.
pub fn unit_uthiol(collection: &str, minted: Option<u64>) -> Option<u128> {
    if collection == FRACTAL_COLLECTION {
        return minted.and_then(tier_for_next).map(|t| t.uthiol());
    }
    Some(NATIVE_AMOUNT)
}

fn lcd_base() -> Option<String> {
    for name in ["DREGG_MINTER_LCD", "DREGG_LCD"] {
        if let Ok(s) = std::env::var(name) {
            let s = s.trim().trim_end_matches('/').to_string();
            if s.starts_with("http://") {
                return Some(s);
            }
        }
    }
    None
}

fn b64_std(input: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

fn count_from_query(v: &serde_json::Value) -> Option<u64> {
    let data = v.get("data").unwrap_or(v);
    let count = data.get("count")?;
    count
        .as_u64()
        .or_else(|| count.as_str().and_then(|s| s.parse().ok()))
}

/// `num_tokens` on a cw721. LCD must be `http://`. Does not broadcast.
pub fn query_num_tokens(contract: &str) -> Result<u64, String> {
    let lcd = lcd_base().ok_or_else(|| "lcd unset".to_string())?;
    let q = b64_std(br#"{"num_tokens":{}}"#);
    let url = format!("{lcd}/cosmwasm/wasm/v1/contract/{contract}/smart/{q}");
    let v = crate::httpx::get_json(&url)?;
    count_from_query(&v).ok_or_else(|| "num_tokens missing".into())
}

fn fractal_minted_from_chain() -> Option<u64> {
    lcd_base()?;
    query_num_tokens(FRACTAL_COLLECTION).ok()
}

fn collection_rows(minted: Option<u64>) -> serde_json::Value {
    serde_json::json!([{
        "name": "permissionless.money fractal",
        "address": FRACTAL_COLLECTION,
        "code_id": 98,
        "served": true,
        "tiers": fractal_tiers().into_iter().map(|t| t.to_json()).collect::<Vec<_>>(),
        "active": minted.and_then(tier_for_next).map(|t| t.to_json()),
        "minted": minted,
        "whitelist": "terp1k5vz89g9z2w0yhg4yfgwwza5d2gg0l9396sjqh27wfck4957pkvq2at0z7",
        "paymint": paymint_configured(FRACTAL_COLLECTION),
    }])
}

/// Fractal only. Logos stay off `/v1/prices` until a whitelist path is chosen.
pub fn collection_tiers() -> serde_json::Value {
    collection_rows(fractal_minted_from_chain())
}

fn paymint_configured(address: &str) -> bool {
    let proof = crate::mint_tx::paymint_for(address);
    !proof.proof_hashes.is_empty() && proof.alloc > 0
}

/// Catalog for FE (`GET /v1/prices`). Amounts only. `ok` is not a Pay signal;
/// the HTTP handler adds live `checkout_ready` from `checkout_now`.
pub fn catalog() -> serde_json::Value {
    let minted = fractal_minted_from_chain();
    let shown = minted.and_then(tier_for_next).unwrap_or(fractal_tiers()[0]);
    serde_json::json!({
        "ok": true,
        "source": "dregg",
        "code_id": 98,
        "eth": { "wei": eth_wei(), "ether": "0.0005", "chain_id": 1 },
        "base": { "wei": base_wei(), "ether": "0.0005", "chain_id": base_chain_id() },
        "btc": { "sats": btc_sats(), "btc": "0.000015" },
        "zec": {
            "zat": shown.zat,
            "zec": crate::zip321::zat_to_zec_decimal(shown.zat)
        },
        "zec_quote": tier_json(minted),
        "native": { "amount": NATIVE_AMOUNT, "denom": native_denom(), "display": "100" },
        "bogo_cap": CROSS_CHAIN_BOGO_CAP,
        "collections": collection_rows(minted),
    })
}

/// How many NFTs a rail deposit mints (2 while under BOGO cap).
pub fn nft_count_for_rail(cross_chain_seen: u64) -> u32 {
    if cross_chain_seen < CROSS_CHAIN_BOGO_CAP {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bogo_first_thousand() {
        assert_eq!(nft_count_for_rail(0), 2);
        assert_eq!(nft_count_for_rail(999), 2);
        assert_eq!(nft_count_for_rail(1000), 1);
    }
    #[test]
    fn product_amounts() {
        assert_eq!(ETH_WEI, 500_000_000_000_000);
        assert_eq!(BTC_SATS, 1_500);
        assert_eq!(ZEC_ZAT, 150_000);
    }

    #[test]
    fn fractal_tiers_follow_supply_and_env() {
        let _g = crate::checkout::minter_env_lock();
        let prev = std::env::var("DREGG_FRACTAL_ZEC_ZAT").ok();
        let lcd = std::env::var("DREGG_MINTER_LCD").ok();
        unsafe { std::env::remove_var("DREGG_FRACTAL_ZEC_ZAT") };
        unsafe { std::env::remove_var("DREGG_MINTER_LCD") };
        assert_eq!(tier_for_next(0).unwrap().zat, 100_000);
        assert_eq!(tier_for_next(999).unwrap().thiol, 100);
        assert_eq!(tier_for_next(1000).unwrap().zat, 500_000);
        assert_eq!(
            crate::zip321::zat_to_zec_decimal(tier_for_next(5000).unwrap().zat),
            "0.01"
        );
        assert!(tier_for_next(10_000).is_none());
        assert!(unit_zat(FRACTAL_COLLECTION, Some(10_000)).is_none());
        assert_eq!(
            unit_uthiol(FRACTAL_COLLECTION, Some(0)).unwrap(),
            100_000_000
        );
        unsafe { std::env::set_var("DREGG_FRACTAL_ZEC_ZAT", "3,4,5") };
        assert_eq!(fractal_zec_zat(), [3, 4, 5]);
        unsafe { std::env::set_var("DREGG_FRACTAL_ZEC_ZAT", "nope") };
        assert_eq!(fractal_zec_zat(), FRACTAL_ZEC_ZAT_DEFAULT);
        unsafe { std::env::remove_var("DREGG_FRACTAL_ZEC_ZAT") };
        let c = catalog();
        assert_eq!(c["code_id"], 98);
        assert_eq!(c["zec_quote"]["mode"], "fixed_per_tier");
        assert_eq!(c["zec_quote"]["per_tier"][0]["zec"], "0.001");
        assert_eq!(c["zec_quote"]["per_tier"][1]["zec"], "0.005");
        assert_eq!(c["zec_quote"]["per_tier"][2]["zec"], "0.01");
        assert!(c["zec_quote"]["minted"].is_null());
        let rows = c["collections"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["address"], FRACTAL_COLLECTION);
        assert_eq!(rows[0]["paymint"], false);
        assert_eq!(b64_std(br#"{"num_tokens":{}}"#), "eyJudW1fdG9rZW5zIjp7fX0=");
        match prev {
            Some(v) => unsafe { std::env::set_var("DREGG_FRACTAL_ZEC_ZAT", v) },
            None => unsafe { std::env::remove_var("DREGG_FRACTAL_ZEC_ZAT") },
        }
        match lcd {
            Some(v) => unsafe { std::env::set_var("DREGG_MINTER_LCD", v) },
            None => unsafe { std::env::remove_var("DREGG_MINTER_LCD") },
        }
    }
}
