//! Deterministic Vesta IPA `Params` table for wasmvm `zk_param/` pins.
//!
//! No-op circuit: `k` is chosen at `Params::new(k)`, not in the CS.
//! Blobs are a function of `(vesta, k)` via `hash_to_curve("Halo2-Parameters")`.
//!
//! Generated with **zcash `halo2_proofs`** (`crates/zcash/halo2`). Regenerating
//! against zakura pasta will not match these checksums until recurate'd.
//!
//! ```text
//! cargo test -p terp-ipa-params -- --nocapture
//! cargo test -p terp-ipa-params -- --ignored --nocapture
//! ```

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use halo2_proofs::circuit::{Layouter, SimpleFloorPlanner};
use halo2_proofs::pasta::vesta;
use halo2_proofs::plonk::{Circuit, ConstraintSystem, Error};
use halo2_proofs::poly::commitment::Params;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const K_MIN: u32 = 1;
pub const K_MAX: u32 = 20;

/// Empty circuit. `k` lives on [`Params`], not in the CS.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopCircuit {
    pub k: u32,
}

impl NoopCircuit {
    pub fn with_k(k: u32) -> Self {
        assert!(k > 0 && k < 32, "k must be in 1..32");
        Self { k }
    }
}

impl Circuit<vesta::Scalar> for NoopCircuit {
    type Config = ();
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        *self
    }

    fn configure(meta: &mut ConstraintSystem<vesta::Scalar>) -> Self::Config {
        let _ = meta.advice_column();
    }

    fn synthesize(
        &self,
        _config: Self::Config,
        _layouter: impl Layouter<vesta::Scalar>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

pub fn generate_vesta_params(k: u32) -> Params<vesta::Affine> {
    assert!(k > 0 && k < 32, "k must be in 1..32");
    Params::new(k)
}

pub fn params_to_bytes(params: &Params<vesta::Affine>) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    params.write(&mut buf)?;
    Ok(buf)
}

pub fn params_checksum(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Clone, Debug, Serialize)]
pub struct ParamArtifact {
    pub k: u32,
    pub bytes_len: usize,
    pub sha256_hex: String,
}

pub fn params_filename(k: u32) -> String {
    format!("k{k:02}_vesta.params.bin")
}

pub fn artifacts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts")
}

pub fn write_and_validate(dir: impl AsRef<Path>, k: u32) -> io::Result<ParamArtifact> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir)?;
    let bytes = params_to_bytes(&generate_vesta_params(k))?;
    let sha256 = params_checksum(&bytes);
    let path = dir.join(params_filename(k));
    std::fs::File::create(&path)?.write_all(&bytes)?;

    let mut round = Vec::new();
    std::fs::File::open(&path)?.read_to_end(&mut round)?;
    let again = params_to_bytes(&generate_vesta_params(k))?;
    if round != again {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("k={k}: disk blob != second Params::new"),
        ));
    }
    Ok(ParamArtifact {
        k,
        bytes_len: bytes.len(),
        sha256_hex: hex::encode(sha256),
    })
}

pub fn generate_k1_through_k20() -> io::Result<Vec<ParamArtifact>> {
    let dir = artifacts_dir();
    let mut rows = Vec::new();
    for k in K_MIN..=K_MAX {
        let t0 = Instant::now();
        let row = write_and_validate(&dir, k)?;
        eprintln!(
            "  k={:02}  {:>12} B  sha256={}  {:.2}s",
            row.k,
            row.bytes_len,
            row.sha256_hex,
            t0.elapsed().as_secs_f64()
        );
        rows.push(row);
    }
    let mut sums = String::new();
    for row in &rows {
        sums.push_str(&format!("{}  {}\n", row.sha256_hex, params_filename(row.k)));
    }
    std::fs::write(dir.join("SHA256SUMS"), sums)?;
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&rows).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?,
    )?;
    Ok(rows)
}

/// Check on-disk blobs against committed `SHA256SUMS` (no keygen).
pub fn validate_pinned_checksums() -> io::Result<usize> {
    let dir = artifacts_dir();
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS"))?;
    let mut n = 0;
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (hex_sum, name) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, line))?;
        let name = name.trim();
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
        let got = hex::encode(params_checksum(&bytes));
        if got != hex_sum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name}: {got} != {hex_sum}"),
            ));
        }
        n += 1;
    }
    Ok(n)
}
