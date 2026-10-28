//! Dry run only (no Daemon, no keys): check the fractal template, build the code-98
//! init for the factory, and print it. Mainnet steps come from `svg_fresh_plan`.
use clap::Parser;
use cw_infuser_scripts::fractal::*;
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(long, default_value_t = 42)] seed: u64,
    #[arg(long, default_value = cw_infuser_scripts::fresh_plan::FRACTAL_CURATOR_TBD)] curator: String,
    #[arg(long)] whitelist: Option<String>,
}
pub fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let m = permissionless_fractal_init_v2(&a.seed, &FractalRoles { curator: a.curator, withdraw: cw_infuser_scripts::fresh_plan::FOUNDATION_DAO.into(), paymint_whitelist: a.whitelist })?;
    println!("{}", serde_json::to_string_pretty(&m)?);
    eprintln!("dry run: nothing broadcast (use svg_fresh_plan for the approvable steps)");
    Ok(())
}
