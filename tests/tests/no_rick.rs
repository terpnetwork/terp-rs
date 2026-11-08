//! Norick WASM-NR e2e: `SuiteProfile::Norick` + host prove/verify.
//!
//! The guest crate is `wasm-bindgen` (`cdylib`). This file exercises the
//! host-equivalent (`rlib`) path so CI does not need a JS runtime. The same
//! functions wrap the WASM exports.

use std::path::PathBuf;
use std::process::Command;

use terp_scripts::suite::config::SuiteProfile;

fn norick_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/wasm-bindgen/norick-wasm/Cargo.toml")
}

#[test]
fn suite_profile_norick_is_sidecar_free() {
    let fleet = SuiteProfile::Norick.fleet("wasm-nr");
    assert!(fleet.hashmerchant.is_none());
    assert!(fleet.merkle_server.is_none());
}

#[test]
fn host_prove_verify_workflow() {
    let manifest = norick_manifest();
    assert!(
        manifest.is_file(),
        "norick-wasm crate missing at {}",
        manifest.display()
    );

    let mut cmd = Command::new(env!("CARGO"));
    cmd.args([
        "test",
        "--manifest-path",
        manifest.to_str().unwrap(),
        "--offline",
        "--lib",
        "--",
        "host_e2e",
        "--test-threads=1",
    ]);
    let out = cmd.output().expect("spawn cargo test -p norick-wasm");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "norick-wasm host e2e failed\nstatus={:?}\nstdout={stdout}\nstderr={stderr}",
        out.status
    );
}
