use clap::Parser;
use cw_infuser_scripts::fractal::*;
use cw_orch::daemon::networks::TERP_MAINNET;
// use cw_infuser_scripts::MOROCCO_1;
use cw_orch::daemon::{Daemon, TxSender};
use cw_orch::prelude::CwOrchInstantiate;

use cw721_svg::*;
use cw_svg::*;
use std::path::PathBuf;

/// Deploy fractal attractor SVG NFTs using on-chain Range variables.
///
/// Fractal params (a–h, base_range) are generated at mint time via
/// `VariableKind::Range` — no pre-generated pool needed. The zooms
/// variable still uses `VariableKind::Options` since it's a complex
/// JSON structure.
///
/// These params feed an off-chain fractal visualizer on permissionless.money.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Initial seed for deterministic RNG (used for contract seed + zoom generation)
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Number of tokens
    #[arg(long, default_value_t = 256)]
    supply: u64,

    /// Number of unique zoom sets to pre-generate
    #[arg(long, default_value_t = 64)]
    zoom_variants: u64,

    /// Output directory for JSON files
    #[arg(long, default_value = "scripts/json")]
    out_dir: PathBuf,
}

pub fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let terp = Daemon::builder(TERP_MAINNET).build()?;
    let svg = Cw721SvgContractSuite::new(terp.clone());
    let _res = svg.instantiate(
        &default_permissionless_fractal_init(&args.seed, &terp.sender().pub_addr_str()),
        Some(&terp.sender().address()),
        &[],
    )?;
    Ok(())
}
