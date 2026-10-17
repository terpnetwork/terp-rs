use std::env;

use cw_orch::{
    anyhow::{self},
    daemon::Daemon,
};

/// Build No-Rick circuit keys only (no upload).
/// Prefer `deploy_norick_testnet` for full circuit+contract deploy.
///
/// ```text
/// export MNEMONIC='…'   # optional for daemon context
/// cargo run -p zk-test-press --bin cc_norick --features interface
/// ```
fn main() -> Result<(), anyhow::Error> {
    env_logger::init();
    let mnemonic = env::var("MNEMONIC").unwrap_or_default();
    let mut builder =
        Daemon::builder(cw_orch::daemon::networks::TERP_TESTNET.clone());
    if !mnemonic.is_empty() {
        builder = builder.mnemonic(&mnemonic);
    }
    let terp = builder.build()?;
    let r = zk_test_press::suites::no_rick::interface::NoRickSuite::new(terp);
    r.circuit.build_keys()?;
    Ok(())
}
