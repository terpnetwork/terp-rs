//! Bench fixture for the on-chain verify benchmark: N real vk_id 3 note proofs
//! from distinct notes (distinct secrets, one note per tree), plus the circuit
//! blob split into params and vk body for `tx wasm store-full-circuit`.
//! Test secrets only; nothing here is a campaign key.
use ff::PrimeField;
use pasta_curves::pallas;
use zk_headstash::no_ecdsa::key_commitment;
use zk_headstash::note_membership::*;

fn main() {
    let out = std::env::args().nth(1).expect("out dir");
    let n: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(40);
    std::fs::create_dir_all(&out).unwrap();
    let blob = note_circuit_blob().expect("blob");
    let footer = &blob[blob.len() - 80..];
    // param_len sits in the footer; recompute from the params writer instead.
    let mut pbuf = Vec::new();
    note_params().write(&mut pbuf).unwrap();
    assert_eq!(&blob[..pbuf.len()], &pbuf[..]);
    std::fs::write(format!("{out}/blob.bin"), &blob).unwrap();
    std::fs::write(format!("{out}/params.bin"), &pbuf).unwrap();
    std::fs::write(format!("{out}/vk_body.bin"), &blob[pbuf.len()..]).unwrap();
    eprintln!(
        "blob {} params {} vk_body {} footer {}",
        blob.len(),
        pbuf.len(),
        blob.len() - pbuf.len(),
        footer.len()
    );
    let ctx = pallas::Base::from(0x11_u64);
    let dest = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
    let path = [pallas::Base::from(0u64); NOTE_DEPTH];
    let mut rows = Vec::new();
    for i in 0..n {
        let secret = pallas::Base::from(0x5eed_0000_u64 + i as u64);
        let amount = DENOMS_UTERP[i % 4];
        let index = i as u32;
        let root = note_root(note_leaf(key_commitment(secret), amount), index, &path);
        let p = prove_note(secret, ctx, amount, index, path, dest).expect("prove");
        let mut inst = Vec::new();
        for limb in &p.public {
            inst.extend_from_slice(limb.to_repr().as_ref());
        }
        rows.push(serde_json::json!({
            "i": i, "amount": amount, "root": hex::encode(root.to_repr()),
            "proof": hex::encode(&p.proof), "instances": hex::encode(&inst),
            "prove_secs": p.prove_secs, "verify_secs": p.verify_secs,
        }));
        eprintln!("note {i} proof {} bytes", p.proof.len());
    }
    std::fs::write(
        format!("{out}/proofs.json"),
        serde_json::to_string(&rows).unwrap(),
    )
    .unwrap();
}
