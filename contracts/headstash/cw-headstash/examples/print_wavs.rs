//! Print one lab BLS proof-of-possession as JSON.
//!
//! The secret is sampled in this process and is not written. Only the public
//! key and the proof-of-possession bytes go to stdout. Do not pass a mnemonic.

fn main() {
    let proof = cw_headstash::wavs::generate_test_wavs_proof(1);
    println!("{}", serde_json::to_string(&proof).expect("wavs json"));
}
