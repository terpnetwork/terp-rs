//! Write the approvable step list for the fresh logos + fractal on codes 98/99/73.
//! Offline: never signs or broadcasts. Exits non-zero (BLOCKED) while any *_TBD remains.
use clap::Parser;
use cw_infuser_scripts::fresh_plan::*;
#[derive(Parser)]
struct A {
    #[arg(long, default_value = FRACTAL_CURATOR)] fractal_curator: String,
    #[arg(long, default_value = LOGOS_CURATOR)] logos_curator: String,
    #[arg(long, default_value = PAYMENT_MINTER)] payment_minter: String,
    #[arg(long, default_value = "WHITELIST_ADDR_FROM_STEP_1")] whitelist: String,
    #[arg(long, default_value = "scripts/cw-infuser/svgs/interchain-packed")] logos_dir: std::path::PathBuf,
    #[arg(long, default_value = "/tmp/svg-fresh-plan.json")] out: std::path::PathBuf,
    /// One msg file per step (`svg_ops --msg-file`).
    #[arg(long, default_value = "/tmp/svg-fresh-steps")] steps_dir: std::path::PathBuf,
}
fn main() -> anyhow::Result<()> {
    let a = A::parse();
    let r = Roles { fractal_curator: a.fractal_curator, logos_curator: a.logos_curator, payment_minter: a.payment_minter, ..Default::default() };
    let s = plan(&r, &a.logos_dir, &a.whitelist, 42)?;
    std::fs::write(&a.out, serde_json::to_vec_pretty(&s)?)?;
    for x in &s { println!("{:<22} signer={} {}", x.id, x.signer, x.note); }
    println!("wrote {}", a.out.display());
    std::fs::create_dir_all(&a.steps_dir)?;
    for x in &s { std::fs::write(a.steps_dir.join(format!("{}.json", x.id)), serde_json::to_vec(&x.msg)?)?; }
    println!("wrote {}/<step>.json", a.steps_dir.display());
    let t = cw_infuser_scripts::paymint::paymint_tree(&r.payment_minter, 10_000, &r.salt);
    if r.payment_minter == PAYMENT_MINTER { anyhow::ensure!(t.root == PAYMINT_ROOT && t.proof == vec![PAYMINT_PROOF.to_string()], "paymint tree drifted from the verified root"); }
    println!("paymint root {} proof {:?}", t.root, t.proof);
    println!("retired code-71 logos: no step; pause/end only if their mint is still open (creator-signed, not in plan)");
    r.require_real()?;
    anyhow::ensure!(!a.whitelist.ends_with("STEP_1"), "BLOCKED: step 2 needs the step-1 whitelist address");
    let wl: cosmrs::AccountId = a.whitelist.parse().map_err(|e| anyhow::anyhow!("BLOCKED: --whitelist {} is not a valid address: {e}", a.whitelist))?;
    anyhow::ensure!(wl.prefix() == "terp" && wl.to_bytes().len() == 32, "BLOCKED: --whitelist must be a terp1 contract address (32 bytes)");
    println!("READY (still nothing broadcast; each step needs owner approval)");
    Ok(())
}
