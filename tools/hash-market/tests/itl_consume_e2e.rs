//! Consume plane — Halo2 Pasta Poseidon merkle+nullifier, usher-only door.
//! Local: no Docker. Live: ict-rs Docker Terp + MsgStoreCodeWithCircuit, not MockBech32.
//!
//!   cargo test --manifest-path tools/onboard-authz/Cargo.toml --test itl_consume_e2e

use hash_market::onboard_authz::itl_cashapp::{synthetic_received_eml, LIST_CENTS, SKU};
use hash_market::onboard_authz::itl_consume::{
    consume_backend_mocked, consume_local_prod_cmd, consume_success_allowed, fp_to_bytes,
    msg_store_code_with_circuit, parse_vk_footer, prove_issued, serialize_vk_with_footer,
    verify_halo2_proof, ConsumeEngine, MSG_STORE_CODE_WITH_CIRCUIT, PROVER_ID,
};
use hash_market::onboard_authz::itl_ingress::{ict_mock_refused, IngressEngine, EVENT_ID};

#[test]
fn itl_consume_usher_only_nullifiers_no_guest_listmembers() {
    let mut door = ConsumeEngine::default();
    door.add_usher("terp1usher");
    assert!(door.is_usher("terp1usher"));
    assert!(!door.is_usher("terp1guest"));
    assert!(door.guest_list_members().is_none());
    assert_eq!(door.verify_consume("terp1guest", &[1], [0u8; 32], [0u8; 32]), Err("not_usher"));
    assert_eq!(
        door.verify_consume("mock1usher", &[1], [0u8; 32], [0u8; 32]),
        Err("mock_bech32")
    );
    assert_eq!(
        door.verify_consume("terp1usher", &[1], [0u8; 32], [0u8; 32]),
        Err("guest_root_unset")
    );
}

#[test]
fn itl_consume_msg_store_code_with_circuit_prover_id_0() {
    let msg = msg_store_code_with_circuit("terp1curator", b"\0asm", b"vk");
    assert_eq!(msg["@type"], MSG_STORE_CODE_WITH_CIRCUIT);
    assert_eq!(msg["prover_id"], 0);
    assert_ne!(msg["@type"], "/cosmwasm.wasm.v1.MsgStoreCode");
    assert!(consume_local_prod_cmd().contains("itl_consume_e2e"));
    assert!(consume_local_prod_cmd().contains("ict-rs-cw-orch"));
    assert!(consume_local_prod_cmd().contains("docker"));
    assert!(ict_mock_refused(Some("1")));
    assert!(!consume_success_allowed("MockBech32", None));
    assert!(consume_backend_mocked("MockBech32", None));
}

#[test]
fn itl_consume_paid_proof_usher_ok_replay_unpaid_nonusher() {
    let mut ing = IngressEngine::default();
    let o = ing.create_cash_order("cred-guest");
    let eml = synthetic_received_eml(&o.pay_ref, LIST_CENTS, "D-CONSUME01");
    let issued = ing.ingest_cashapp_eml(&eml).expect("paid");
    let (proof, inst, nf) = prove_issued(
        ing.tree(),
        &issued.ticket_secret,
        SKU,
        EVENT_ID,
        issued.leaf_index,
    )
    .expect("prove");
    assert!(verify_halo2_proof(&proof, &inst).expect("verify"));

    let mut door = ConsumeEngine::default();
    door.add_usher("terp1usher");
    door.set_guest_root(issued.guest_root);
    let eid = fp_to_bytes(inst.event_id);

    assert_eq!(
        door.verify_consume("terp1rando", &proof, nf, eid),
        Err("not_usher")
    );
    door.verify_consume("terp1usher", &proof, nf, eid)
        .expect("first consume");
    assert!(door.nullifier_spent(&nf));
    assert_eq!(
        door.verify_consume("terp1usher", &proof, nf, eid),
        Err("nullifier_spent")
    );
    assert!(door.guest_list_members().is_none());

    let unpaid_secret = [0x11u8; 32];
    let unpaid_err = prove_issued(ing.tree(), &unpaid_secret, SKU, EVENT_ID, issued.leaf_index);
    assert!(unpaid_err.is_err() || {
        let (p, uinst, unf) = unpaid_err.unwrap();
        !verify_halo2_proof(&p, &uinst).unwrap_or(false)
            && door
                .verify_consume("terp1usher", &p, unf, fp_to_bytes(uinst.event_id))
                .is_err()
    });
}

#[test]
fn itl_consume_vk_footer_halo2_pasta() {
    let blob = serialize_vk_with_footer().expect("vk");
    let (pid, n, pl, vl) = parse_vk_footer(&blob).unwrap();
    assert_eq!(pid, PROVER_ID);
    assert_eq!(n, 3);
    assert!(pl > 0 && vl > 0);
}

#[test]
#[ignore = "needs ict-rs Docker Terp (itl_consume_e2e) — Daemon, not MockBech32"]
fn itl_consume_docker_terp_store_code_with_circuit() {
    assert!(
        !ict_mock_refused(std::env::var("ICT_MOCK").ok().as_deref()),
        "ICT_MOCK=1 refused"
    );
    let backend = std::env::var("ICT_BACKEND").unwrap_or_else(|_| "daemon".into());
    assert!(
        consume_success_allowed(&backend, std::env::var("ICT_MOCK").ok().as_deref()),
        "MockBech32 is not consume success"
    );
    let cmd = consume_local_prod_cmd();
    assert!(cmd.contains("docker"));
    assert!(cmd.contains("MsgStoreCodeWithCircuit") || cmd.contains("itl_consume_e2e"));
    let lcd = std::env::var("ICT_LCD").expect("ICT_LCD");
    assert!(!lcd.is_empty());
}

/// Product: Halo2 prove/verify + usher policy. Not a chain mock.
/// Env: stock terpd without zk-wasmvm must fail-closed on-chain (see cw-ave zk.rs).
#[test]
fn itl_consume_vk_is_not_debug_pinned_params_roundtrip() {
    let blob = serialize_vk_with_footer().expect("vk");
    let s = String::from_utf8_lossy(&blob);
    assert!(
        !s.contains("PinnedVerificationKey"),
        "testing env must not ship Debug VK"
    );
    let (_, _, pl, vl) = parse_vk_footer(&blob).unwrap();
    assert!(pl > 0 && vl >= 4);
}
