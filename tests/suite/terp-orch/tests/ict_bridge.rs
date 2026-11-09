//! Compile-gate for ict-rs → cw-orch Daemon glue.
//! Live Docker: `ICT_ZKJWT=1 cargo test -p terp-orch --features ict --test ict_bridge -- --ignored`

#![cfg(feature = "ict")]

#[test]
fn daemon_builder_symbol_exists() {
    let _ = terp_orch::ict::daemon_builder;
}
