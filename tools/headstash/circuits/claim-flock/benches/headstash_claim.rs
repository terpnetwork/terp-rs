//! One Headstash claim: Blake3 inclusion plus the derived nullifier,
//! proved and verified as a Ligerito circuit.

use std::time::Instant;

use headstash_claim::{self, Opening, DEPTH};

fn sample() -> Opening<DEPTH> {
    let mut siblings = [[0u32; 8]; DEPTH];
    siblings[0] = [21, 22, 23, 24, 25, 26, 27, 28];
    siblings[1] = [31, 32, 33, 34, 35, 36, 37, 38];
    Opening {
        rseed: [1, 2, 3, 4, 5, 6, 7, 8],
        epk_x: [9, 10, 11, 12],
        nd: [13, 14],
        v: 15,
        fdi: 1,
        index: 1,
        siblings,
        destination: [0x1111, 0x2222, 0x3333, 0x4444, 0x5555],
    }
}

fn main() {
    flock_prover::init_perf_thread_pool();
    let opening = sample();
    let t = Instant::now();
    let report = headstash_claim::prove_and_verify(&opening);
    let total = t.elapsed().as_secs_f64();
    let json = format!(
        "{{\n  \"system\": \"flock-ligerito\",\n  \"statement\": \"blake3-merkle-depth-{depth}+nullifier\",\n  \"gates\": {gates},\n  \"build_secs\": {build},\n  \"prove_secs\": {prove},\n  \"verify_secs\": {verify},\n  \"proof_bytes\": {bytes},\n  \"total_secs\": {total}\n}}\n",
        depth = headstash_claim::DEPTH,
        gates = report.gates,
        build = report.build_secs,
        prove = report.prove_secs,
        verify = report.verify_secs,
        bytes = report.proof_bytes,
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/flock-claim-bench.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("bench dir");
    }
    std::fs::write(&path, &json).expect("write bench");
    println!("{json}wrote {}", path.display());
}
