//! Write footer-only VK blobs + `CircuitSummary` JSON for demo circuits.
//!
//! Uses the same `CircuitFooter` / `CircuitSummary` path as other cw-orch
//! circuit suites (`CircuitFooterSpec` + `CircuitSummaryLoader`).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use zk_cosmwasm::curves::FlockVerifyingKey;
use zk_cosmwasm::{CircuitFooter, StwoVerifyingKey};

use cw_orch::core::circuits::circuit_interface_traits::CircuitSummary;

fn artifacts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts")
}

fn summary_from_footer(name: &str, footer: &CircuitFooter) -> CircuitSummary {
    CircuitSummary {
        circuit_name: name.into(),
        k: footer.k,
        instance_count: footer.i,
        curve_id: footer.curve_id,
        prover_id: footer.prover_id,
        num_fixed_columns: 0,
        num_advice_columns: 0,
        num_instance_columns: 0,
        degree: 0,
        has_lookups: false,
        param_len: footer.param_len,
        vk_len: footer.vk_len,
        cs_len: footer.cs_len,
        num_selectors: 0,
        num_gates: 0,
        param_checksum: footer.param_checksum,
        vk_checksum: footer.vk_checksum,
    }
}

fn write_one(name: &str, blob: &[u8], footer: &CircuitFooter) -> Result<()> {
    let dir = artifacts_dir();
    fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let vk_path = dir.join(format!("{name}_vk.bin"));
    fs::write(&vk_path, blob).with_context(|| format!("write {}", vk_path.display()))?;
    let summary = summary_from_footer(name, footer);
    let json_path = dir.join(format!("{name}_summary.json"));
    fs::write(&json_path, serde_json::to_vec_pretty(&summary)?)
        .with_context(|| format!("write {}", json_path.display()))?;
    Ok(())
}

/// Materialize Stwo + Flock lean VKs for `circuit_interface` path validation.
pub fn write_demo_circuit_artifacts() -> Result<()> {
    let stwo = StwoVerifyingKey::lean_default();
    write_one("stwo_ssle", &stwo.to_blob(), &stwo.footer)?;
    let flock = FlockVerifyingKey::lean_default();
    write_one("flock_ligerito", &flock.to_blob(), &flock.footer)?;
    Ok(())
}
