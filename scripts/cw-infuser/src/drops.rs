//! Initial cw721-svg collections for the market deploy.
//!
//! Marain Orbs uses the scribe-slot shield. Kara & Gippy is the first Latent
//! Benders frame. Yi Jing Harmony uses the packed dao-harmony template, priced
//! in uterp at the same 0.36 figure as the USDT spec.

use anyhow::{anyhow, Result};
use cosmwasm_std::{Binary, coin};
use cw721_svg::InstantiateMsg;

use crate::yijing::{self, YiJingConfig};

pub const MARAIN_ORBS: &str = "marain-orbs";
pub const KARA_GIPPY: &str = "kara-gippy";
pub const YIJING_HARMONY: &str = "yijing-harmony";

pub struct Drop {
    pub name: &'static str,
    pub msg: InstantiateMsg,
}

/// Creator (cw721 creator owner, withdraw address and, via the factory, wasm admin) per drop.
pub struct Creators<'a> {
    /// Withdraw (proceeds) + royalty recipient for every drop.
    pub withdraw: &'a str,
    pub marain: &'a str,
    pub benders: &'a str,
    pub yijing: &'a str,
}

pub fn initial_collections(creator: &str, denom: &str) -> Result<Vec<Drop>> {
    initial_collections_with(&Creators { withdraw: creator, marain: creator, benders: creator, yijing: creator }, denom)
}

pub fn initial_collections_with(c: &Creators, denom: &str) -> Result<Vec<Drop>> {
    Ok(vec![
        Drop {
            name: MARAIN_ORBS,
            msg: marain_orbs(c.marain, c.withdraw)?,
        },
        Drop {
            name: KARA_GIPPY,
            msg: kara_gippy(c.benders, c.withdraw)?,
        },
        Drop {
            name: YIJING_HARMONY,
            msg: yijing_harmony(c.yijing, c.withdraw, denom)?,
        },
    ])
}

fn seed(tag: &str) -> Binary {
    blake3::hash(tag.as_bytes()).as_bytes().into()
}

fn pack(template: &str, vars: &[cw_svg::VariableDef]) -> Result<Binary> {
    let (packed, _) = svg_collections::common::check_and_pack(template, vars)
        .map_err(|err| anyhow!("pack: {err}"))?;
    Ok(Binary::from(packed))
}

fn marain_orbs(creator: &str, withdraw: &str) -> Result<InstantiateMsg> {
    let themes = svg_collections::marain::themes();
    let vars = svg_collections::marain::variables(&themes);
    let template = svg_collections::marain::template();
    Ok(InstantiateMsg {
        name: "Marain Orbs".into(),
        symbol: "MARAIN".into(),
        collection_info_extension: cw721_svg::SvgCollectionMetadata {
            seed: seed("marain-orbs/initial"),
            packed: pack(&template, &vars)?,
            price_tiers: vec![],
            total: 1_000,
            mint_start_time: None,
            mint_end_time: None,
            whitelist: None,
            require_auth: false,
        },
        minter: None,
        creator: Some(creator.to_string()),
        withdraw_address: Some(withdraw.to_string()),
    })
}

fn kara_gippy(creator: &str, withdraw: &str) -> Result<InstantiateMsg> {
    let frame = svg_collections::ascii::sanitize(
        "Kara holds the room\nGippy walks the day\n################\n  ##    ##    ##\n ####  ####  ####\n",
    )
    .map_err(|err| anyhow!("kara frame: {err}"))?;
    let cid = svg_collections::cid::cid_v1_raw(b"kara-and-gippy/initial")
        .map_err(|err| anyhow!("kara cid: {err}"))?;
    let cids = vec![cid];
    let vars = svg_collections::ascii::variables(&cids);
    let template =
        svg_collections::ascii::template(&[frame], &cids, "Kara & Gippy", Some("1;.7;1"));
    Ok(InstantiateMsg {
        name: "Kara & Gippy".into(),
        symbol: "KGIPPY".into(),
        collection_info_extension: cw721_svg::SvgCollectionMetadata {
            seed: seed("kara-and-gippy/initial"),
            packed: pack(&template, &vars)?,
            price_tiers: vec![],
            total: 100,
            mint_start_time: None,
            mint_end_time: None,
            whitelist: None,
            require_auth: false,
        },
        minter: None,
        creator: Some(creator.to_string()),
        withdraw_address: Some(withdraw.to_string()),
    })
}

fn yijing_harmony(creator: &str, withdraw: &str, denom: &str) -> Result<InstantiateMsg> {
    let mut cfg = YiJingConfig::default();
    cfg.denom = denom.to_string();
    cfg.usdt_decimals = 6;
    cfg.minter = None;
    cfg.creator = Some(creator.to_string());
    cfg.withdraw_address = Some(withdraw.to_string());
    cfg.royalty_recipient = Some(withdraw.to_string());
    let mut msg = yijing::yijing_harmony_init(&cfg)?;
    msg.collection_info_extension.price_tiers = vec![cw721_svg::PriceTier {
        until_count: yijing::SUPPLY,
        price: coin(cfg.price_amount(), denom),
    }];
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_collections_pack() {
        let drops = initial_collections("terp19rl4cm2hmr8afy4kldpxz3fka4jguq0am2y34x", "uthiol").unwrap();
        assert_eq!(
            drops.iter().map(|drop| drop.name).collect::<Vec<_>>(),
            vec![MARAIN_ORBS, KARA_GIPPY, YIJING_HARMONY]
        );
        for drop in &drops {
            assert!(!drop.msg.collection_info_extension.packed.is_empty());
            let back: InstantiateMsg =
                serde_json::from_value(serde_json::to_value(&drop.msg).unwrap()).unwrap();
            assert_eq!(back.name, drop.msg.name);
        }
        assert_eq!(drops[2].msg.collection_info_extension.total, 1_426);
        assert_eq!(
            drops[2].msg.collection_info_extension.price_tiers[0]
                .price
                .amount
                .to_string(),
            "360000"
        );
        assert_eq!(
            drops[2].msg.collection_info_extension.price_tiers[0]
                .price
                .denom,
            "uthiol"
        );
    }
}
