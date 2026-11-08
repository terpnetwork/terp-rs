use cw_orch::mock::Mock;
use zk_headstash::suite::{HeadstashCircuitSuite, suite::HeadstashSinsemillaTree};
use zk_test_press::BoxError;

/// ## `create_headstash_notes`
/// Craft partial notes + **Poseidon-v1** public inclusion tree (ADR-POSEIDON-DISTRO-TREE).
///
/// ```bash
/// cargo run --bin gen_headstash_notes -- ./data/holders.json 0x…eligibility_sk
/// ```
///
/// Writes leaf-annotated input JSON and `artifacts/merkle_output.json` with
/// `distro_hash_domain: poseidon-v1`. Publish the root as contract `genesis_root`.
fn main() -> Result<(), BoxError> {
    let s = HeadstashCircuitSuite::new(Mock::new("sender"));
    s.create_headstash_notes()?;
    let output_path = std::path::Path::new("./artifacts").join("merkle_output.json");
    let root_hex =
        HeadstashCircuitSuite::new(Mock::new("sender")).gen_headstash_tree(output_path)?;
    println!("🌳 Poseidon-v1 Merkle Root: {}", root_hex);
    println!("   distro_hash_domain=poseidon-v1 (default for new Headstashes)");
    Ok(())
}
