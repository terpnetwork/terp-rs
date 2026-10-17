//! Host K=18 prove for the local chain harness.
//!
//! Writes the proof file and returns. It does not start a chain and it does
//! not treat cw-orch Mock as a broadcast.
//!
//! ```text
//! HEADSTASH_PROVE_ONLY=1 HEADSTASH_CLAIM_PROOF=/tmp/headstash-e2e/claim_proof.json \
//!   cargo test -p terp-orch --features headstash --test export_claim_proof -- --ignored --nocapture
//! ```

#![cfg(feature = "headstash")]

use std::path::PathBuf;

#[test]
#[ignore = "K=18 Halo2 keygen+prove. Writes claim_proof.json. No chain broadcast."]
fn export_claim_proof_for_local_chain() {
    let path = std::env::var("HEADSTASH_CLAIM_PROOF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("artifacts/headstash_claim_proof.json"));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // set_var is unsafe because it races other threads. This runs on the test
    // thread before run_headstash_claim starts Rayon.
    unsafe {
        std::env::set_var("HEADSTASH_PROVE_ONLY", "1");
        std::env::set_var("HEADSTASH_CLAIM_PROOF", &path);
    }
    let report = terp_orch::run_headstash_claim().expect("host prove");
    assert!(report.proof_bytes > 0, "proof file was empty");
    assert!(report.k == 18, "expected K=18");
    assert!(!report.claim_mock_verify);
    assert!(path.is_file(), "missing {}", path.display());
}
