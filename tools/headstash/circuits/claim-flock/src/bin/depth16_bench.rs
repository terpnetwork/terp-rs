//! Release timings for one claim on a 51,807-note depth-16 tree.
//!
//! Prints timings and sizes only. Does not print rseed, keys, or proof bytes.

use std::time::Instant;

use headstash_claim::tree::{self, SecretNote};
use headstash_claim::{prove_claim, verify_claim, Opening, DEPTH};

const NOTES: usize = 51_807;
const INDEX: usize = 1_000;

fn note(i: u32) -> SecretNote {
    let mut rseed = [0u32; 8];
    rseed[0] = i.wrapping_mul(0x9E37_79B9).wrapping_add(1);
    rseed[1] = i ^ 0xA5A5_A5A5;
    SecretNote {
        rseed,
        epk_x: [0u8; 32],
        epk_y: [0u8; 32],
        epk_words: [i, i.wrapping_add(1), 0, 0],
        nd: [1, 0],
        v: 15,
        fdi: 1,
    }
}

fn main() {
    flock_prover::init_perf_thread_pool();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    assert_eq!(tree::depth_for(NOTES), DEPTH as u32, "production depth");
    let t_tree = Instant::now();
    let notes: Vec<SecretNote> = (0..NOTES as u32).map(note).collect();
    let (tree, path) = tree::siblings(&notes, INDEX);
    let tree_secs = t_tree.elapsed().as_secs_f64();
    assert_eq!(tree.depth, DEPTH as u32);
    assert_eq!(tree.leaves.len(), NOTES);
    assert_eq!(path.len(), DEPTH);
    let mut siblings = [[0u32; 8]; DEPTH];
    for (i, sib) in path.iter().enumerate() {
        siblings[i] = *sib;
    }
    let leaf = &notes[INDEX];
    let opening = Opening::<DEPTH> {
        rseed: leaf.rseed,
        epk_x: leaf.epk_words,
        nd: leaf.nd,
        v: leaf.v,
        fdi: leaf.fdi,
        index: INDEX as u32,
        siblings,
        destination: [0x1111, 0x2222, 0x3333, 0x4444, 0x5555],
    };
    println!(
        "{{\"event\":\"setup\",\"notes\":{NOTES},\"depth\":{DEPTH},\"index\":{INDEX},\"tree_build_secs\":{tree_secs:.6},\"threads\":{threads}}}"
    );
    for run in 1..=3 {
        let t_prove = Instant::now();
        let proved = prove_claim(&opening);
        let prove_secs = t_prove.elapsed().as_secs_f64();
        let proof_bytes = bincode::serialize(&proved.proof).expect("proof").len();
        let t_verify = Instant::now();
        let claim = verify_claim::<DEPTH>(opening.index, &proved).expect("sidecar verify");
        let verify_secs = t_verify.elapsed().as_secs_f64();
        assert_eq!(claim.root, tree.root);
        assert_eq!(claim.amount, leaf.v);
        println!(
            "{{\"event\":\"run\",\"run\":{run},\"prove_secs\":{prove_secs:.6},\"sidecar_verify_secs\":{verify_secs:.6},\"proof_bytes\":{proof_bytes}}}"
        );
    }
    for run in 1..=3 {
        let report = headstash_claim::prove_and_verify(&opening);
        println!(
            "{{\"event\":\"split\",\"run\":{run},\"gates\":{gates},\"build_secs\":{build:.6},\"ligerito_prove_secs\":{prove:.6},\"verify_built_secs\":{verify:.6},\"proof_bytes\":{bytes}}}",
            gates = report.gates,
            build = report.build_secs,
            prove = report.prove_secs,
            verify = report.verify_secs,
            bytes = report.proof_bytes,
        );
    }
    println!("{{\"event\":\"done\"}}");
}
