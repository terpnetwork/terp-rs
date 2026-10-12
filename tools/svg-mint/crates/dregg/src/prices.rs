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

/// Catalog for FE (`GET /v1/prices`). Amounts only. `ok` is not a Pay signal;
/// the HTTP handler adds live `checkout_ready` from `checkout_now`.
pub fn catalog() -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "source": "dregg",
        "eth": { "wei": eth_wei(), "ether": "0.0005", "chain_id": 1 },
        "base": { "wei": base_wei(), "ether": "0.0005", "chain_id": base_chain_id() },
        "btc": { "sats": btc_sats(), "btc": "0.000015" },
        "zec": { "zat": zec_zat(), "zec": crate::zip321::zat_to_zec_decimal(zec_zat()) },
        "native": { "amount": NATIVE_AMOUNT, "denom": native_denom(), "display": "100" },
        "bogo_cap": CROSS_CHAIN_BOGO_CAP,
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
}
