//! ```text
//! cargo test -p terp-ipa-params -- --nocapture
//! cargo test -p terp-ipa-params -- --ignored --nocapture
//! ```

use terp_ipa_params::{
    generate_k1_through_k20, generate_vesta_params, params_checksum, params_to_bytes,
    validate_pinned_checksums, write_and_validate, NoopCircuit, K_MAX, K_MIN,
};

#[test]
fn noop_circuit_binds_k() {
    let c = NoopCircuit::with_k(4);
    assert_eq!(c.k, 4);
}

#[test]
fn params_new_is_deterministic() {
    let a = params_to_bytes(&generate_vesta_params(4)).unwrap();
    let b = params_to_bytes(&generate_vesta_params(4)).unwrap();
    assert_eq!(a, b);
    assert_eq!(params_checksum(&a), params_checksum(&b));
}

#[test]
fn generate_and_validate_k1_through_k8() {
    let dir = std::env::temp_dir().join(format!("terp-ipa-params-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    for k in 1..=8 {
        write_and_validate(&dir, k).unwrap_or_else(|e| panic!("k={k}: {e}"));
    }
}

#[test]
fn pinned_sha256sums_match_existing_bins() {
    let n = validate_pinned_checksums().expect("SHA256SUMS");
    assert!(
        n > 0,
        "expected on-disk kNN_vesta.params.bin next to SHA256SUMS"
    );
}

#[test]
#[ignore = "writes k=1..=20; cargo test -p terp-ipa-params -- --ignored --nocapture"]
fn generate_and_validate_k1_through_k20() {
    assert_eq!(K_MIN, 1);
    assert_eq!(K_MAX, 20);
    let rows = generate_k1_through_k20().expect("ipa params table");
    assert_eq!(rows.len(), 20);
    assert_eq!(validate_pinned_checksums().unwrap(), 20);
}
