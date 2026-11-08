//! Product A: fresh Headstash circuit + clearnet claim via `proof_instance_verify`.
//!
//! Not mock verify. Not private-dex. Public `uterp` mint.
//!
//! ```text
//! cargo test -p terp-orch --features headstash --test headstash_claim -- --ignored --nocapture
//! ```
//!
//! K=18 keygen + prove is slow. Writes `artifacts/headstash_claim_metrics.json`.

#![cfg(feature = "headstash")]

use terp_orch::run_headstash_claim;

#[test]
#[ignore = "K=18 keygen+prove; run: cargo test -p terp-orch --features headstash --test headstash_claim -- --ignored --nocapture"]
fn claim_clearnet_proof_instance_verify() {
    let report = run_headstash_claim().expect("Product A clearnet claim");
    assert_eq!(report.k, 18);
    assert_eq!(report.public_inputs, 6);
    assert!(!report.claim_mock_verify);
    assert!(report.proof_bytes > 0);
    assert!(report.store_circuit_blob_bytes > 80);
    assert!(!report.mint_denom.is_empty());
}
