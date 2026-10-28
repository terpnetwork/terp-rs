//! Write the approvable step list for the fresh logos + fractal on codes 98/99/73.
//! Offline: never signs or broadcasts. Exits non-zero (BLOCKED) while any *_TBD remains.
use clap::Parser;
use cw_infuser_scripts::fresh_plan::*;
#[derive(Parser)]
struct A {
    #[arg(long, default_value = FRACTAL_CURATOR_TBD)] fractal_curator: String,
    #[arg(long, default_value = LOGOS_CURATOR_TBD)] logos_curator: String,
    #[arg(long, default_value = cw_infuser_scripts::paymint::PAYMENT_MINTER_TBD)] payment_minter: String,
    #[arg(long, default_value = "WHITELIST_ADDR_FROM_STEP_1")] whitelist: String,
    #[arg(long, default_value = "scripts/cw-infuser/svgs/interchain-packed")] logos_dir: std::path::PathBuf,
    #[arg(long, default_value = "/tmp/svg-fresh-plan.json")] out: std::path::PathBuf,
}
fn main() -> anyhow::Result<()> {
    let a = A::parse();
    let r = Roles { fractal_curator: a.fractal_curator, logos_curator: a.logos_curator, payment_minter: a.payment_minter, ..Default::default() };
    let s = plan(&r, &a.logos_dir, &a.whitelist, 42)?;
    std::fs::write(&a.out, serde_json::to_vec_pretty(&s)?)?;
    for x in &s { println!("{:<22} signer={} {}", x.id, x.signer, x.note); }
    println!("wrote {}", a.out.display());
    r.require_real()?;
    anyhow::ensure!(!a.whitelist.ends_with("STEP_1"), "BLOCKED: step 2 needs the step-1 whitelist address");
    println!("READY (still nothing broadcast; each step needs owner approval)");
    Ok(())
}
