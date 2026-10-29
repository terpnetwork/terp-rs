//! k19 lab: one harness for the K=19 ECDSA+GLV claim circuit and the vk_id 3
//! note-membership circuit (K=11, depth 17). zakura-halo2 only.
//! LAB=k19|vk3 cargo test -p zk-headstash --release --lib lab_bench -- --ignored --nocapture
use std::time::Instant;
use std::{format, println, string::String, vec::Vec};

use halo2_proofs::{
    dev::CircuitCost,
    plonk::{self, Circuit, ConstraintSystem, SingleVerifier},
    poly::commitment::Params,
    transcript::{Blake2bRead, Blake2bWrite, Challenge255},
};
use pasta_curves::{pallas, vesta};

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn field(dbg: &str, name: &str) -> Option<usize> {
    let at = dbg.find(&format!("{name}: "))? + name.len() + 2;
    dbg[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

pub(crate) fn measure<C>(
    name: &str,
    k: u32,
    circuit: C,
    public: Vec<pallas::Base>,
    runs: usize,
) -> serde_json::Value
where
    C: Circuit<pallas::Base> + Clone + Sync + core::fmt::Debug,
    C::Config: Send + Sync,
{
    let mut cs = ConstraintSystem::<pallas::Base>::default();
    let _ = C::configure(&mut cs);
    let cost = format!("{:?}", CircuitCost::<vesta::Point, C>::measure(k, &circuit));
    if std::env::var("LAB_COST_ONLY").is_ok() {
        let v = serde_json::json!({"circuit": name, "k": k, "cost_only": true, "rows_max": field(&cost, "max_rows"),
            "advice_rows_max": field(&cost, "max_advice_rows"), "fixed_rows_max": field(&cost, "max_fixed_rows")});
        println!("LAB_JSON {v}");
        return v;
    }
    let t = Instant::now();
    let params = Params::<vesta::Affine>::new(k);
    let params_secs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let vk = plonk::keygen_vk(&params, &C::without_witnesses(&circuit)).expect("vk");
    let vk_secs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let pk = plonk::keygen_pk(&params, vk, &C::without_witnesses(&circuit)).expect("pk");
    let pk_secs = t.elapsed().as_secs_f64();
    let (mut pb, mut vb, mut kb) = (Vec::new(), Vec::new(), Vec::new());
    params.write(&mut pb).unwrap();
    pk.get_vk().write(&mut vb).unwrap();
    pk.write(&mut kb).unwrap();
    let mut prove = Vec::new();
    let mut verify = Vec::new();
    let mut proof = Vec::new();
    for _ in 0..runs {
        let mut rng = crate::os_rng();
        let t = Instant::now();
        let mut tr = Blake2bWrite::<_, vesta::Affine, Challenge255<_>>::init(Vec::new());
        plonk::create_proof(&params, &pk, &[circuit.clone()], &[&[&public[..]]], &mut rng, &mut tr)
            .expect("prove");
        proof = tr.finalize();
        prove.push(t.elapsed().as_secs_f64());
        let t = Instant::now();
        let mut tr = Blake2bRead::<_, vesta::Affine, Challenge255<_>>::init(&proof[..]);
        plonk::verify_proof(&params, pk.get_vk(), SingleVerifier::new(&params), &[&[&public[..]]], &mut tr)
            .expect("verify");
        verify.push(t.elapsed().as_secs_f64());
    }
    // soundness smoke: a changed public input must not verify.
    let mut bad = public.clone();
    bad[0] += <pallas::Base as ff::Field>::ONE;
    let mut tr = Blake2bRead::<_, vesta::Affine, Challenge255<_>>::init(&proof[..]);
    let wrong_public_rejected = plonk::verify_proof(&params, pk.get_vk(), SingleVerifier::new(&params), &[&[&bad[..]]], &mut tr).is_err();
    let v = serde_json::json!({
        "circuit": name, "k": k, "rows_max": field(&cost, "max_rows"),
        "advice_rows_max": field(&cost, "max_advice_rows"), "fixed_rows_max": field(&cost, "max_fixed_rows"),
        "advice_columns": cs.num_advice_columns(), "fixed_columns": cs.num_fixed_columns(),
        "instance_columns": cs.num_instance_columns(), "lookups": field(&cost, "lookups"),
        "degree": cs.degree(), "public_inputs": public.len(),
        "params_gen_secs": params_secs, "keygen_vk_secs": vk_secs, "keygen_pk_secs": pk_secs,
        "keygen_secs": vk_secs + pk_secs,
        "prove_secs": prove.clone(), "prove_median_secs": median(prove),
        "verify_secs": verify.clone(), "verify_median_secs": median(verify),
        "proof_bytes": proof.len(), "params_bytes": pb.len(), "vk_bytes": vk_bytes_len(&vb), "pk_bytes": kb.len(),
        "wrong_public_rejected": wrong_public_rejected,
        "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    });
    println!("LAB_JSON {v}");
    v
}

fn vk_bytes_len(v: &[u8]) -> usize { v.len() }

fn runs() -> usize {
    std::env::var("LAB_RUNS").ok().and_then(|s| s.parse().ok()).unwrap_or(5)
}

#[test]
#[ignore = "lab bench"]
fn lab_bench_vk3() {
    use crate::no_ecdsa::{key_commitment, pack_destination};
    use crate::note_membership::*;
    use halo2_proofs::circuit::Value;
    let secret = pallas::Base::from(0x5eed_0001_u64);
    let ctx = pallas::Base::from(0x11_u64);
    let dest = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
    let amount = DENOMS_UTERP[0];
    let path = [pallas::Base::from(7u64); NOTE_DEPTH];
    let root = note_root(note_leaf(key_commitment(secret), amount), 5, &path);
    let public = note_public(secret, ctx, amount, root, dest).unwrap();
    let (lo, hi) = pack_destination(dest).unwrap();
    let c = NoteMembershipCircuit {
        secret: Value::known(secret), ctx: Value::known(ctx), amount: Value::known(amount),
        position: Value::known(5), path: Value::known(path),
        destination_lo: Value::known(lo), destination_hi: Value::known(hi), nf_tag_witness: None,
    };
    measure("vk3_note_membership_d17", NOTE_K, c, public, runs());
}

#[cfg(all(feature = "host-crypto", feature = "interface"))]
#[test]
#[ignore = "lab bench"]
fn lab_bench_k19() {
    use crate::suite::suite::HeadstashProofBuilder;
    use crate::suite::HeadstashCircuitSuite;
    use cw_orch::prelude::Mock;
    let fb = std::env::var("LAB_ECDSA_FB").is_ok();
    crate::circuit::gadget::secp256k1_chip::ECDSA_FIXED_BASE.store(fb, core::sync::atomic::Ordering::SeqCst);
    let suite = HeadstashCircuitSuite::new(Mock::new("k19-lab"));
    let (circuit, instance, _a, _p) = suite.suite_backed_claim_pair(4, 1).expect("suite claim");
    let public = instance.to_halo2_instance()[0].to_vec();
    let k = std::env::var("LAB_K19_K").ok().and_then(|v| v.parse().ok()).unwrap_or(crate::circuit::K);
    let name = if fb { format!("k19_ecdsa_fb_u1g_at_k{k}") } else if k == crate::circuit::K { String::from("k19_ecdsa_glv") } else { format!("k19_ecdsa_glv_at_k{k}") };
    measure(&name, k, circuit, public, runs());
}

pub(crate) fn runs_pub() -> usize {
    runs()
}
