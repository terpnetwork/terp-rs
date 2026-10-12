//! Local Halo2 Pasta verifier — same statement as zk-wasmvm `verify_halo2_proof`.
//! Stock terpd without the zk host must fail-closed on-chain; this binary
//! exercises the **product** circuit, not MockBech32.

use hash_market::onboard_authz::itl_cashapp::{synthetic_received_eml, LIST_CENTS, SKU};
use hash_market::onboard_authz::itl_consume::{
    prove_issued, serialize_vk_with_footer, verify_halo2_proof, ConsumeEngine, PROVER_ID, VK_MAGIC,
};
use hash_market::onboard_authz::itl_ingress::{IngressEngine, EVENT_ID};

fn main() {
    let mut ing = IngressEngine::default();
    let o = ing.create_cash_order("cred-guest");
    let eml = synthetic_received_eml(&o.pay_ref, LIST_CENTS, "D-HOSTVERIFY");
    let issued = ing.ingest_cashapp_eml(&eml).expect("paid");
    let (proof, inst, nf) =
        prove_issued(ing.tree(), &issued.ticket_secret, SKU, EVENT_ID, issued.leaf_index)
            .expect("prove");
    assert!(verify_halo2_proof(&proof, &inst).expect("verify"));
    let vk = serialize_vk_with_footer().expect("vk");
    assert_eq!(vk[vk.len() - 10], PROVER_ID);
    let (_, _, pl, _) = hash_market::onboard_authz::itl_consume::parse_vk_footer(&vk).unwrap();
    assert_eq!(&vk[pl as usize..pl as usize + 4], VK_MAGIC);

    let mut door = ConsumeEngine::default();
    door.add_usher("terp1usher");
    door.set_guest_root(issued.guest_root);
    let eid = hash_market::onboard_authz::itl_consume::fp_to_bytes(inst.event_id);
    assert_eq!(
        door.verify_consume("terp1guest", &proof, nf, eid),
        Err("not_usher")
    );
    door.verify_consume("terp1usher", &proof, nf, eid)
        .expect("usher consume");
    assert_eq!(
        door.verify_consume("terp1usher", &proof, nf, eid),
        Err("nullifier_spent")
    );
    println!("PASS itl-consume-verify halo2 pasta poseidon usher/replay/non-usher");
}
