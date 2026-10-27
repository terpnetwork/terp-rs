//! Pack the interchain logo set into cw721-svg (code 98) InstantiateMsgs.
//!
//! Reads `svgs/interchain/<logo>/init.json` (legacy code-71 shape: `svg_template` +
//! `variables` + `template_slots`), packs template+vars with the SVP1 codec
//! (`cw721_svg::pack_svg`), unpacks again to verify a byte-exact round-trip, and writes
//! `svgs/interchain-packed/<logo>/{init.json,packed.svp}` plus `manifest.json`.
//!
//!   cargo run -p cw-infuser-scripts --bin pack_logos -- \
//!     [--src scripts/cw-infuser/svgs/interchain] [--out scripts/cw-infuser/svgs/interchain-packed] \
//!     [--creator <logos curator addr>] [--withdraw <addr>] [--dry-run]
//!
//! `creator` defaults to the placeholder `LOGOS_CURATOR_TBD` (the dedicated logos
//! curator key does not exist yet); replace before broadcasting. Mint windows are left
//! `null` (set per deploy). No chain access.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use cosmwasm_std::Binary;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

pub const FOUNDATION_DAO: &str = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w";

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "scripts/cw-infuser/svgs/interchain")]
    src: PathBuf,
    #[arg(long, default_value = "scripts/cw-infuser/svgs/interchain-packed")]
    out: PathBuf,
    #[arg(long, default_value = "LOGOS_CURATOR_TBD")]
    creator: String,
    #[arg(long, default_value = FOUNDATION_DAO)]
    withdraw: String,
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

fn main() -> Result<()> {
    let a = Args::parse();
    let mut dirs: Vec<_> = fs::read_dir(&a.src)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("init.json").is_file())
        .collect();
    dirs.sort();
    let mut manifest = vec![];
    for d in dirs {
        let logo = d.file_name().unwrap().to_string_lossy().to_string();
        let old: Value = serde_json::from_str(&fs::read_to_string(d.join("init.json"))?)
            .with_context(|| format!("{logo}/init.json"))?;
        let e = &old["collection_info_extension"];
        let tmpl = e["svg_template"].as_str().context("svg_template")?;
        let vars: Vec<cw_svg::VariableDef> = serde_json::from_value(e["variables"].clone())?;
        cw_svg::validate_variables(&vars).map_err(|x| anyhow::anyhow!("{logo}: {x:?}"))?;
        // Same check the code-98 contract runs at instantiate (no animation-only paint).
        cw_svg::validate_animated_paint(tmpl)
            .map_err(|e| anyhow::anyhow!("{logo}: {e}"))?;
        let packed = cw721_svg::pack_svg(tmpl, &vars);
        let (t2, v2) = cw721_svg::decode_packed(packed.as_slice()).map_err(|x| anyhow::anyhow!("{x}"))?;
        let roundtrip = t2 == tmpl && serde_json::to_value(&v2)? == serde_json::to_value(&vars)?;
        ensure!(roundtrip, "{logo}: SVP1 round-trip mismatch");
        ensure!(packed.len() <= cw721_svg::contract::MAX_SVG_SIZE, "{logo}: packed over MAX_SVG_SIZE");
        let msg = cw721_svg::InstantiateMsg {
            name: old["name"].as_str().unwrap_or(&logo).to_string(),
            symbol: old["symbol"].as_str().unwrap_or("SVG").to_string(),
            collection_info_extension: cw721_svg::SvgCollectionMetadata {
                seed: serde_json::from_value::<Binary>(e["seed"].clone())?,
                packed: packed.clone(),
                price_tiers: serde_json::from_value(e["price_tiers"].clone())?,
                total: e["total"].as_u64().unwrap_or(10_000),
                mint_start_time: None,
                mint_end_time: None,
                whitelist: None,
                require_auth: false,
            },
            minter: None,
            creator: Some(a.creator.clone()),
            withdraw_address: Some(a.withdraw.clone()),
        };
        let msg_json = serde_json::to_vec_pretty(&msg)?;
        let old_ext = serde_json::to_vec(&json!({"svg_template":tmpl,"variables":e["variables"],"template_slots":e["template_slots"]}))?.len();
        let row = json!({
            "logo": logo, "name": msg.name, "symbol": msg.symbol,
            "template_bytes": tmpl.len(), "variables": vars.len(),
            "legacy_ext_json_bytes": old_ext, "packed_bytes": packed.len(),
            "packed_sha256": hex::encode(Sha256::digest(packed.as_slice())),
            "init_msg_json_bytes": serde_json::to_vec(&msg)?.len(), "roundtrip": roundtrip,
        });
        println!("{row}");
        if !a.dry_run {
            let o = a.out.join(&logo);
            fs::create_dir_all(&o)?;
            fs::write(o.join("init.json"), &msg_json)?;
            fs::write(o.join("packed.svp"), packed.as_slice())?;
        }
        manifest.push(row);
    }
    if !a.dry_run {
        fs::write(a.out.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    }
    Ok(())
}
