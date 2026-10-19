//! Yi Jing Harmony (dao-harmony 10-19-26): cw721-svg InstantiateMsg for the packed-template
//! (SVP1, `svg_packed`) cw721-svg on terp-rs HEAD.
//!
//! Design spec: terp/design/specs/yijing-harmony-2026-10-19.md (v2) and MINT-COST.md.
//! Template + variables: `svgs/research/yijing-harmony/` (template.svg is the prototype's, byte for byte).
//!
//! What the current contract can and cannot express
//! ------------------------------------------------
//! * Template: stored once, SVP1-packed (`cw721_svg::pack_svg`), checked by the same validators
//!   the contract runs on instantiate (`check_template`).
//! * Variables: the current contract only has seeded kinds (`Options`, `Range`, `Rgb`,
//!   `RgbStyled`). Each mint picks every option from sha256(token_id || seed || height), so the
//!   tournament's hexagram, lines, grid and captions CANNOT be set per token yet.
//!   - The 121 prototype `SuppliedOptions` variables map 1:1 to `Options` with the same lists, so
//!     the template, slots and option tables are already final.
//!   - The 12 `SuppliedText` variables map to `Options` over the 4 sample texts (preview only).
//!   TODO(yijing-v2-values): replace both with per-token supplied values; see
//!   `svgs/research/yijing-harmony/v2-variables.json` and spec §"v2 handoff".
//! * Price: one tier at 0.36 USDT. USDT arrives over a Penumbra IBC channel that is not open yet,
//!   so the denom and decimals are placeholders (`YiJingConfig::denom`, `usdt_decimals`).
//! * Royalty: cw721-svg has no royalty field. The 9% is set on the marketplace after
//!   instantiate (`crates/marketplace` `ExecuteMsg::SetRoyaltyFee`); see `royalty_msg`.
use cosmwasm_std::{Binary, coin};
use cw_svg::VariableDef;

pub const NAME: &str = "Yi Jing Harmony";
pub const SYMBOL: &str = "YIJING";
/// One token per distinct line of the dao-harmony 10-19-26 tally.
pub const SUPPLY: u64 = 1_426;
/// TODO(penumbra-usdt): swap for the real IBC denom once the Penumbra channel is open.
pub const PENUMBRA_USDT_DENOM_PLACEHOLDER: &str = "ibc/<PENUMBRA_USDT_TBD>";
/// TODO(penumbra-usdt): confirm decimals of the bridged USDT (6 is the usual USDT value).
pub const USDT_DECIMALS_PLACEHOLDER: u32 = 6;
/// 0.36 USDT, in hundredths.
pub const PRICE_CENTS: u128 = 36;
/// 9 % royalty (marketplace, not cw721-svg).
pub const ROYALTY_PERCENT: u64 = 9;
pub const MINTER_PLACEHOLDER: &str = "<MINTER_ADDR_TBD>";
pub const CREATOR_PLACEHOLDER: &str = "<CREATOR_ADDR_TBD>";
pub const WITHDRAW_PLACEHOLDER: &str = "<WITHDRAW_ADDR_TBD>";
pub const ROYALTY_RECIPIENT_PLACEHOLDER: &str = "<ROYALTY_RECIPIENT_TBD>";
pub const COLLECTION_ADDR_PLACEHOLDER: &str = "<YIJING_COLLECTION_ADDR>";
pub const SEED_TAG: &str = "yijing-harmony/dao-harmony-10-19-26/yh1";

pub const TEMPLATE: &str = include_str!("../svgs/research/yijing-harmony/template.svg");
pub const VARIABLES_JSON: &str = include_str!("../svgs/research/yijing-harmony/variables.json");

/// Every swap-able value in one place. `Default` is the placeholder config.
#[derive(Clone, Debug)]
pub struct YiJingConfig {
    pub denom: String,
    pub usdt_decimals: u32,
    /// `None` = the collection contract mints itself (public self-serve mint), as cw721-svg does.
    pub minter: Option<String>,
    pub creator: Option<String>,
    pub withdraw_address: Option<String>,
    pub royalty_recipient: Option<String>,
    /// Seconds after instantiate before minting opens (`None` = open at once).
    pub mint_start_offset_secs: Option<u64>,
}

impl Default for YiJingConfig {
    fn default() -> Self {
        Self {
            denom: PENUMBRA_USDT_DENOM_PLACEHOLDER.into(),
            usdt_decimals: USDT_DECIMALS_PLACEHOLDER,
            minter: Some(MINTER_PLACEHOLDER.into()),
            creator: Some(CREATOR_PLACEHOLDER.into()),
            withdraw_address: Some(WITHDRAW_PLACEHOLDER.into()),
            royalty_recipient: Some(ROYALTY_RECIPIENT_PLACEHOLDER.into()),
            mint_start_offset_secs: None,
        }
    }
}

impl YiJingConfig {
    /// 0.36 in base units of `denom`.
    pub fn price_amount(&self) -> u128 {
        assert!(self.usdt_decimals >= 2, "usdt_decimals must be >= 2");
        PRICE_CENTS * 10u128.pow(self.usdt_decimals - 2)
    }
    /// Any `<…TBD>` / `ibc/<…>` placeholder still in the config.
    pub fn placeholders(&self) -> Vec<String> {
        let mut v = vec![];
        let mut chk = |k: &str, s: &Option<String>| {
            if let Some(s) = s {
                if s.contains('<') {
                    v.push(format!("{k}={s}"))
                }
            }
        };
        chk("denom", &Some(self.denom.clone()));
        chk("minter", &self.minter);
        chk("creator", &self.creator);
        chk("withdraw_address", &self.withdraw_address);
        chk("royalty_recipient", &self.royalty_recipient);
        v
    }
}

pub fn variables() -> Vec<VariableDef> {
    serde_json::from_str(VARIABLES_JSON).expect("yijing variables.json")
}

/// Runs the checks cw721-svg's instantiate runs, in the same order, then packs and round-trips.
pub fn check_template(template: &str, vars: &[VariableDef]) -> anyhow::Result<Binary> {
    use cw721_svg::contract::{MAX_SVG_SIZE, decode_packed, scan_slots, scribe_layout};
    cw_svg::validate_variables(vars).map_err(|e| anyhow::anyhow!("variables: {e}"))?;
    let packed = cw721_svg::pack_svg(template, vars);
    anyhow::ensure!(
        packed.len() <= MAX_SVG_SIZE,
        "packed {} > MAX_SVG_SIZE {}",
        packed.len(),
        MAX_SVG_SIZE
    );
    let (t2, v2) = decode_packed(&packed).map_err(|e| anyhow::anyhow!("decode: {e}"))?;
    anyhow::ensure!(t2 == template && v2 == vars, "SVP1 round-trip mismatch");
    let slots = scan_slots(&t2, &v2).map_err(|e| anyhow::anyhow!("slots: {e}"))?;
    cw_svg::validate_template_slots(&t2, &v2, &slots).map_err(|e| anyhow::anyhow!("slots: {e}"))?;
    cw_svg::validate_animated_paint(&t2).map_err(|e| anyhow::anyhow!("animated paint: {e}"))?;
    cw_svg::links::validate_links(&t2, &v2).map_err(|e| anyhow::anyhow!("links: {e}"))?;
    scribe_layout(&v2).map_err(|e| anyhow::anyhow!("scribe layout: {e}"))?;
    Ok(packed)
}

pub fn yijing_harmony_init(cfg: &YiJingConfig) -> anyhow::Result<cw721_svg::InstantiateMsg> {
    let packed = check_template(TEMPLATE, &variables())?;
    Ok(cw721_svg::InstantiateMsg {
        name: NAME.into(),
        symbol: SYMBOL.into(),
        collection_info_extension: cw721_svg::SvgCollectionMetadata {
            // Seed only drives the interim seeded picks; v2 values make it irrelevant to the art.
            seed: blake3::hash(SEED_TAG.as_bytes()).as_bytes().into(),
            packed,
            price_tiers: vec![cw721_svg::PriceTier {
                until_count: SUPPLY,
                price: coin(cfg.price_amount(), cfg.denom.clone()),
            }],
            total: SUPPLY,
            mint_start_time: cfg
                .mint_start_offset_secs
                .map(cosmwasm_std::Timestamp::from_seconds),
            mint_end_time: None,
            whitelist: None,
            require_auth: false,
        },
        minter: cfg.minter.clone(),
        creator: cfg.creator.clone(),
        withdraw_address: cfg.withdraw_address.clone(),
    })
}

/// Marketplace royalty registration (crates/marketplace/src/auction.rs `ExecuteMsg::SetRoyaltyFee`),
/// sent to the marketplace contract by a royalty admin after the collection exists.
pub fn royalty_msg(cfg: &YiJingConfig, collection_addr: Option<&str>) -> serde_json::Value {
    serde_json::json!({ "set_royalty_fee": {
        "contract_addr": collection_addr.unwrap_or(COLLECTION_ADDR_PLACEHOLDER),
        "creator": cfg.royalty_recipient.clone().unwrap_or_else(|| ROYALTY_RECIPIENT_PLACEHOLDER.into()),
        "royalty_fee": cosmwasm_std::Decimal::percent(ROYALTY_PERCENT),
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholder_msg_builds_and_validates() {
        let cfg = YiJingConfig::default();
        let m = yijing_harmony_init(&cfg).unwrap();
        assert_eq!(m.collection_info_extension.total, 1426);
        assert_eq!(
            m.collection_info_extension.price_tiers[0]
                .price
                .amount
                .to_string(),
            "360000"
        );
        assert_eq!(
            m.collection_info_extension.price_tiers[0].price.denom,
            "ibc/<PENUMBRA_USDT_TBD>"
        );
        assert_eq!(variables().len(), 133);
        assert_eq!(cfg.placeholders().len(), 5);
        let back: cw721_svg::InstantiateMsg =
            serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        assert_eq!(
            royalty_msg(&cfg, None)["set_royalty_fee"]["royalty_fee"],
            "0.09"
        );
    }
}
