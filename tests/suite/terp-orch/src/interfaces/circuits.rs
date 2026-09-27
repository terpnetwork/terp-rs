//! cw-orch `circuit_interface` handles for Path A demos.
//!
//! Footers come from `zk-cosmwasm` lean VKs (same Path A layout as wasmvm).
//! `CircuitFooterSpec` is implemented here (macro `summary_json` parse is unused).

use cw_orch::circuit_interface;
use cw_orch::core::circuits::circuit_interface_traits::CircuitFooterSpec;
use cw_orch::core::environment::ChainState;

/// Stwo / M31 SSLE demo (`prover_id=2`, `curve_id=5`).
#[circuit_interface(id = "stwo_ssle", artifacts_dir = "artifacts")]
pub struct StwoSsleCircuit;

impl<Chain: ChainState> CircuitFooterSpec for StwoSsleCircuit<Chain> {
    fn summary_json_path() -> &'static str {
        concat!(env!("CARGO_MANIFEST_DIR"), "/artifacts/stwo_ssle_summary.json")
    }
}

/// Flock Ligerito demo (`prover_id=3`, `curve_id=7`).
#[circuit_interface(id = "flock_ligerito", artifacts_dir = "artifacts")]
pub struct FlockLigeritoCircuit;

impl<Chain: ChainState> CircuitFooterSpec for FlockLigeritoCircuit<Chain> {
    fn summary_json_path() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/artifacts/flock_ligerito_summary.json"
        )
    }
}
