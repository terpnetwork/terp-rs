//! Native vk_id 3 note prover for the local e2e: reads the same witness JSON the
//! browser worker takes (file path or /dev/stdin) and prints the same result
//! shape as `prove_note_witness_browser`. The witness is never echoed.
use std::io::Read;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/stdin".into());
    let mut text = String::new();
    std::fs::File::open(&path)
        .and_then(|mut f| f.read_to_string(&mut text))
        .expect("witness");
    match zk_headstash::note_membership::prove_note_witness_json(&text) {
        Ok(p) => println!(
            "{}",
            serde_json::json!({
                "vk_id": zk_headstash::note_membership::NOTE_VK_ID,
                "k": p.k,
                "proof_hex": hex::encode(&p.proof),
                "proof_bytes": p.proof.len(),
                "pk_bytes": p.pk_bytes,
                "body": p.body,
            })
        ),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2)
        }
    }
}
