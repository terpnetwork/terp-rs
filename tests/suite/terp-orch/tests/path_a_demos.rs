//! Mock e2e: circuit footers + Path A verify through `cw-zk-verify-demo`.
//!
//! ```text
//! cargo test -p terp-orch --test path_a_demos
//! ```

use cosmwasm_std::testing::{clear_test_circuits, register_test_circuit};
use cosmwasm_std::{Binary, VerificationError};
use cw_orch::core::circuits::circuit_interface_traits::{
    CircuitPathValidator, CircuitSummaryLoader,
};
use cw_orch::prelude::*;
use terp_orch::{
    write_demo_circuit_artifacts, FlockLigeritoCircuit, StwoSsleCircuit, TerpCircuitDemoDeployData,
    TerpCircuitDemoSuite,
};
use zk_cosmwasm::curves::verify_stwo_proof;
use zk_cosmwasm::{prove_flock, verify_flock_proof};

fn deploy(zkid: u64) -> TerpCircuitDemoSuite<Mock> {
    write_demo_circuit_artifacts().expect("write circuit artifacts");
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    TerpCircuitDemoSuite::deploy_on(mock, TerpCircuitDemoDeployData { admin, zkid })
        .expect("deploy")
}

#[test]
fn circuit_interfaces_write_and_validate_footers() {
    write_demo_circuit_artifacts().expect("artifacts");
    let info = ChainInfoOwned::default();
    assert!(StwoSsleCircuit::<Mock>::is_circuit_ready(&info));
    assert!(FlockLigeritoCircuit::<Mock>::is_circuit_ready(&info));
    let stwo = StwoSsleCircuit::<Mock>::load_summary().expect("stwo summary");
    assert_eq!(stwo.prover_id, 2);
    assert_eq!(stwo.curve_id, 5);
    let flock = FlockLigeritoCircuit::<Mock>::load_summary().expect("flock summary");
    assert_eq!(flock.prover_id, 3);
    assert_eq!(flock.curve_id, 7);
}

#[test]
fn flock_digest_stub_fails_through_demo_contract() {
    clear_test_circuits();
    register_test_circuit(
        3,
        "flock_ligerito",
        Box::new(|proof, instances| {
            verify_flock_proof(proof, instances)
                .map(|_| true)
                .map_err(|_| VerificationError::GenericErr)
        }),
    );
    let suite = deploy(3);
    let action = vec![7u8; 128];
    let stub = prove_flock(&action);
    let err = suite
        .verify
        .execute(
            &cw_zk_verify_demo::ExecuteMsg::Verify {
                proof: Binary::from(stub),
                instances: Binary::from(action),
            },
            &[],
        )
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("proof_instance_verify")
            || msg.contains("host verifier")
            || msg.contains("digest"),
        "expected fail-closed flock stub, got: {msg}"
    );
    clear_test_circuits();
}

#[test]
fn stwo_registered_host_accepts_and_dummy_dstw_rejected() {
    clear_test_circuits();
    register_test_circuit(
        5,
        "stwo_ssle",
        Box::new(|proof, instances| {
            if proof.windows(4).any(|w| w == b"DSTW") || proof.starts_with(b"DSTW") {
                return verify_stwo_proof(proof, instances)
                    .map(|_| true)
                    .map_err(|_| VerificationError::GenericErr);
            }
            Ok(proof.starts_with(b"STWO"))
        }),
    );
    let suite = deploy(5);
    suite
        .verify
        .execute(
            &cw_zk_verify_demo::ExecuteMsg::Verify {
                proof: Binary::from(b"STWO\x02\x05SSLE".to_vec()),
                instances: Binary::from(vec![]),
            },
            &[],
        )
        .expect("registered stwo host accept");
    let mut dstw = vec![0u8; 18];
    dstw[0..4].copy_from_slice(b"DSTW");
    dstw[4] = 2;
    dstw[5] = 5;
    let err = suite
        .verify
        .execute(
            &cw_zk_verify_demo::ExecuteMsg::Verify {
                proof: Binary::from(dstw),
                instances: Binary::from(vec![]),
            },
            &[],
        )
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("host verifier") || msg.contains("proof_instance_verify") || msg.contains("DSTW"),
        "expected dummy DSTW reject, got: {msg}"
    );
    clear_test_circuits();
}
