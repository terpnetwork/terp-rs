//! Host timing for one Headstash Halo2 claim — same `Proof::create` the snap
//! calls from `gen_proof_witness`.
//!
//! MetaMask Snap workers (`@metamask/snaps-execution-environments`) are one SES
//! compartment per snap. They do not set `crossOriginIsolated` or hand the snap
//! `SharedArrayBuffer`, so `wasm-bindgen-rayon` `initThreadPool` cannot add OS
//! threads inside the snap. The execution environment parallelizes **across
//! snaps** (one worker each), not inside one K=18 IPA prove.
//!
//! This bench is **native release** (`halo2_proofs/multicore`). It is not a WASM
//! timing. Full K=18 prove in snap WASM is not viable here: `host-crypto` pulls
//! libsecp C, and a single-thread WASM prove would be several times slower than
//! the native number below.

use std::time::Instant;

use cw_orch::prelude::Mock;

use crate::circuit::{Proof, ProvingKey};
use crate::suite::suite::HeadstashProofBuilder;
use crate::suite::HeadstashCircuitSuite;

#[test]
#[ignore = "K=18 keygen+prove; cargo test -p zk-headstash --release --lib snap_bench -- --ignored --nocapture"]
fn bench_one_claim_proof_native() {
    let suite = HeadstashCircuitSuite::new(Mock::new("snap-bench"));
    let t_key = Instant::now();
    let pk = ProvingKey::build();
    let keygen_secs = t_key.elapsed().as_secs_f64();

    let (circuit, instance, _anchor, _partial) = suite
        .suite_backed_claim_pair(4, 1)
        .expect("suite claim");
    let instance_for_verify = instance.clone();

    let mut rng = crate::os_rng();
    let t_prove = Instant::now();
    let proof = Proof::create(&pk, &[circuit], &[instance], &mut rng).expect("prove");
    let prove_secs = t_prove.elapsed().as_secs_f64();

    let t_ver = Instant::now();
    proof
        .verify(&pk.vk(), &[instance_for_verify.clone()])
        .expect("verify");
    let verify_secs = t_ver.elapsed().as_secs_f64();

    let vk = pk.vk();
    let flipped = {
        let mut bytes = proof.as_ref().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        Proof::new(bytes)
    };
    let t_flip = Instant::now();
    let flipped_err = flipped
        .verify(&vk, &[instance_for_verify.clone()])
        .expect_err("flipped proof must not verify");
    let flipped_secs = t_flip.elapsed().as_secs_f64();

    let mut wrong_instance = instance_for_verify.clone();
    wrong_instance.e[0] ^= 0xff;
    let t_inst = Instant::now();
    let instance_err = proof
        .verify(&vk, &[wrong_instance])
        .expect_err("proof must not verify under a different public instance");
    let wrong_instance_secs = t_inst.elapsed().as_secs_f64();

    let short = Proof::new(b"not-a-halo2-proof".to_vec());
    let t_short = Instant::now();
    let short_err = short
        .verify(&vk, &[instance_for_verify.clone()])
        .expect_err("truncated proof must not verify");
    let short_secs = t_short.elapsed().as_secs_f64();

    // A flood of distinct corrupt proofs. Each one still has to be rejected.
    const FLOOD: usize = 8;
    let t_flood = Instant::now();
    for i in 0..FLOOD {
        let mut bytes = proof.as_ref().to_vec();
        let at = i % bytes.len();
        bytes[at] ^= 0xa5;
        Proof::new(bytes)
            .verify(&vk, &[instance_for_verify.clone()])
            .expect_err("flooded proof must not verify");
    }
    let flood_secs = t_flood.elapsed().as_secs_f64();

    let report = serde_json::json!({
        "halo2": "zakura-halo2-proofs",
        "floor_planner": "V1+floor-planner-v1-legacy-pdqsort",
        "runtime": "native-release",
        "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "k": crate::circuit::K,
        "scalar_mul": "glv-128",
        "dropped": "q_orchard selector and action cv/enable gates (never enabled)",
        "keygen_secs": keygen_secs,
        "prove_secs": prove_secs,
        "verify_secs": verify_secs,
        "proof_bytes": proof.as_ref().len(),
        "invalid": {
            "flipped_byte_secs": flipped_secs,
            "flipped_byte_err": format!("{flipped_err:?}"),
            "wrong_instance_secs": wrong_instance_secs,
            "wrong_instance_err": format!("{instance_err:?}"),
            "truncated_secs": short_secs,
            "truncated_err": format!("{short_err:?}"),
            "flood_count": FLOOD,
            "flood_secs": flood_secs,
            "flood_each_secs": flood_secs / FLOOD as f64,
        },
        "wasm_snap": {
            "measured": false,
            "reason": "Snap SES worker is single-threaded; host-crypto is not a wasm32 guest. initThreadPool is a no-op unless the execution environment exposes SharedArrayBuffer, which snaps-execution-environments does not.",
        },
    });
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../artifacts/snap-proof-bench.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    std::eprintln!("{report}");
    std::eprintln!("wrote {}", path.display());
}


/// Three release proves of the current Halo2 claim.
///
/// The suite tree is 51,807 leaves. The circuit still hashes
/// `MERKLE_DEPTH_ORCHARD` (32) levels: the path is padded. This is not a
/// depth-16 circuit. Prints timings only.
#[test]
#[ignore = "K=19 keygen plus three proves of a 51807-note padded path"]
fn bench_three_opens_on_51807() {
    let suite = HeadstashCircuitSuite::new(Mock::new("notes-51807"));
    let t_key = Instant::now();
    let pk = ProvingKey::build();
    let keygen_secs = t_key.elapsed().as_secs_f64();
    let vk = pk.vk();

    let t_tree = Instant::now();
    let (circuit, instance, _anchor, _partial) = suite
        .suite_backed_claim_pair(51_807, 1_000)
        .expect("51807-note inclusion");
    let witness_secs = t_tree.elapsed().as_secs_f64();
    std::println!(
        "{{\"event\":\"setup\",\"notes\":51807,\"circuit_merkle_depth\":{},\"k\":{},\"keygen_secs\":{:.6},\"witness_secs\":{:.6},\"threads\":{}}}",
        crate::constants::MERKLE_DEPTH_ORCHARD,
        crate::circuit::K,
        keygen_secs,
        witness_secs,
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    );

    for run in 1..=3 {
        let mut rng = crate::os_rng();
        let t_prove = Instant::now();
        let proof = Proof::create(&pk, &[circuit.clone()], &[instance.clone()], &mut rng)
            .expect("prove");
        let prove_secs = t_prove.elapsed().as_secs_f64();
        let t_ver = Instant::now();
        proof.verify(&vk, &[instance.clone()]).expect("verify");
        let verify_secs = t_ver.elapsed().as_secs_f64();
        std::println!(
            "{{\"event\":\"run\",\"run\":{run},\"prove_secs\":{:.6},\"verify_secs\":{:.6},\"proof_bytes\":{}}}",
            prove_secs,
            verify_secs,
            proof.as_ref().len(),
        );
    }
    std::println!("{{\"event\":\"done\"}}");
}
