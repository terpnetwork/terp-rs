//! Product A Headstash — claim onto **clearnet** (public `uterp`), not private dex.
//!
//! Fresh Halo2 keys → Path A store-circuit blob → host `Proof::create`.
//!
//! `HEADSTASH_PROVE_ONLY=1` writes `claim_proof.json` and returns before the
//! cw-orch Mock execute. That file is what the loopback harness broadcasts.
//! The Mock path, when it runs, still wraps Halo2 `Proof::verify`. It is not
//! a chain transaction. Mock `AppResponse` has no gas meter.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use cosmwasm_std::testing::{clear_test_circuits, register_test_circuit};
use cosmwasm_std::{Binary, Coin, VerificationError};
use cw_headstash::headstash::{HeadstashInstances, HeadstashNote};
use cw_headstash::msg::{ExecuteMsg, InstantiateMsg};
use cw_headstash::tokenfactory::{HeadstashTokenObject, TokenStrategy};
use cw_headstash::wavs::generate_test_wavs_proof;
use cw_orch::prelude::*;
// Same rand_core 0.10 that zk-headstash's Proof::create bound uses.
// rand 0.8 OsRng does not implement that RngCore.
use serde::Serialize;
use zk_cosmwasm::CircuitFooter;
use zk_headstash::circuit::{Instance, Proof, ProvingKey};
use zk_headstash::suite::suite::HeadstashProofBuilder;
use zk_headstash::suite::HeadstashCircuitSuite;

const HEADSTASH_K: u32 = 18;

const CID: u64 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct HeadstashClaimReport {
    pub k: u32,
    pub public_inputs: u32,
    pub params_bytes: usize,
    pub cs_bytes: usize,
    pub vk_bytes: usize,
    pub store_circuit_blob_bytes: usize,
    pub proof_bytes: usize,
    pub keygen_secs: f64,
    pub prove_secs: f64,
    pub verify_host_secs: f64,
    pub gas_wanted: Option<u64>,
    pub gas_used: Option<u64>,
    /// Honest: cw-orch Mock has no tx gas; ict-rs Daemon fills gas_used.
    pub gas_note: String,
    pub claim_mock_verify: bool,
    pub mint_denom: String,
    pub claim_value: u64,
}

pub fn artifacts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts")
}

fn instance_to_msg(bytes: &[u8], e: [u8; 32]) -> HeadstashInstances {
    assert!(bytes.len() >= 168, "instance prefix");
    HeadstashInstances {
        anchor: Binary::from(bytes[0..32].to_vec()),
        nd: Binary::from(bytes[32..64].to_vec()),
        v: u64::from_le_bytes(bytes[64..72].try_into().unwrap()),
        nf: Binary::from(bytes[72..104].to_vec()),
        recp: Binary::from(bytes[104..136].to_vec()),
        cmx: Binary::from(bytes[136..168].to_vec()),
        e: Binary::from(e.to_vec()),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Fresh `ProvingKey::build_and_write` → `artifacts/headstash_vk.bin` (Path A footer).
pub fn write_headstash_circuit() -> Result<(ProvingKey, usize, f64)> {
    let dir = artifacts_dir();
    fs::create_dir_all(&dir).context("mkdir artifacts")?;
    let path = dir.join("headstash_vk.bin");
    let t0 = Instant::now();
    // `build_headstash_keys_to` also runs zakura `AnyVerifyingKey::from_bytes` (no
    // selector matrix). Product A keys are zcash Halo2 IPA — footer+checksums still
    // apply; host Mock verify is `Proof::verify`, not zakura decode.
    let pk = ProvingKey::build_and_write(path.clone())
        .map_err(|e| anyhow::anyhow!("keygen write: {e}"))?;
    let blob_len = fs::metadata(&path)?.len() as usize;
    Ok((pk, blob_len, t0.elapsed().as_secs_f64()))
}

/// Prove a suite-backed claim and mint on Mock through `proof_instance_verify`.
pub fn run_headstash_claim() -> Result<HeadstashClaimReport> {
    let t0 = Instant::now();
    let suite = HeadstashCircuitSuite::new(Mock::new("headstash-keys"));
    let (circuit, instance, anchor, partial) = suite
        .suite_backed_claim_pair(4, 1)
        .map_err(|e| anyhow::anyhow!("suite-backed claim: {e}"))?;
    let inst_bytes = instance.to_bytes();
    anyhow::ensure!(
        inst_bytes.len() >= 168,
        "instance width {}",
        inst_bytes.len()
    );
    let digest = zk_headstash::claim_auth::keccak256(&inst_bytes[..168]);
    let e = zk_headstash::claim_auth::challenge_scalar_be(&digest);
    let genesis_root = Binary::from(anchor.to_bytes().to_vec());
    let denom = partial.token.clone();
    let token = HeadstashTokenObject::new(denom.clone());
    let claim_value = partial.value;
    let rr = Binary::from(partial.recipient.to_vec());

    eprintln!("headstash keygen K={HEADSTASH_K} (fresh Path A blob)…");
    let (pk, blob_len, keygen_secs) = write_headstash_circuit()?;
    let vk = pk.vk();
    let blob = fs::read(artifacts_dir().join("headstash_vk.bin"))?;
    anyhow::ensure!(blob.len() >= 80, "store-circuit blob");
    let footer = CircuitFooter::from_bytes(&blob[blob.len() - 80..])
        .map_err(|e| anyhow::anyhow!("footer: {e}"))?;
    let params_bytes = footer.param_len as usize;
    let cs_bytes = footer.cs_len as usize;
    let vk_bytes = footer.vk_len as usize;

    eprintln!("prove…");
    let t1 = Instant::now();
    let proof = Proof::create(&pk, &[circuit], &[instance.clone()], rand::rng())
        .map_err(|e| anyhow::anyhow!("prove: {e}"))?;
    let prove_secs = t1.elapsed().as_secs_f64();

    let t2 = Instant::now();
    proof
        .verify(&vk, &[instance.clone()])
        .map_err(|e| anyhow::anyhow!("host verify: {e}"))?;
    let verify_host_secs = t2.elapsed().as_secs_f64();

    let proof_path = std::env::var("HEADSTASH_CLAIM_PROOF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| artifacts_dir().join("headstash_claim_proof.json"));
    if let Some(dir) = proof_path.parent() {
        fs::create_dir_all(dir).context("mkdir claim proof dir")?;
    }
    let proof_doc = serde_json::json!({
        "prover": "zk_headstash::circuit::Proof::create",
        "host_verify": true,
        "k": HEADSTASH_K,
        "denom": denom,
        "value": claim_value,
        "anchor_hex": hex_encode(&inst_bytes[0..32]),
        "nd_hex": hex_encode(&inst_bytes[32..64]),
        "nf_hex": hex_encode(&inst_bytes[72..104]),
        "recp_hex": hex_encode(&inst_bytes[104..136]),
        "cmx_hex": hex_encode(&inst_bytes[136..168]),
        "e_hex": hex_encode(&e),
        "proof_hex": hex_encode(proof.as_ref()),
        "recipient_source": "HeadstashCircuitSuite::suite_backed_claim_pair",
        "vk_path": artifacts_dir().join("headstash_vk.bin").display().to_string(),
    });
    fs::write(&proof_path, serde_json::to_vec_pretty(&proof_doc)?)
        .with_context(|| format!("write {}", proof_path.display()))?;
    eprintln!("wrote {}", proof_path.display());

    if std::env::var("HEADSTASH_PROVE_ONLY").ok().as_deref() == Some("1") {
        let report = HeadstashClaimReport {
            k: HEADSTASH_K,
            public_inputs: footer.i as u32,
            params_bytes,
            cs_bytes,
            vk_bytes,
            store_circuit_blob_bytes: blob_len,
            proof_bytes: proof.as_ref().len(),
            keygen_secs,
            prove_secs,
            verify_host_secs,
            gas_wanted: None,
            gas_used: None,
            gas_note: "host prove only. The loopback harness broadcasts this proof. Mock was not run."
                .into(),
            claim_mock_verify: false,
            mint_denom: denom,
            claim_value,
        };
        let metrics = artifacts_dir().join("headstash_claim_metrics.json");
        fs::write(&metrics, serde_json::to_vec_pretty(&report)?)?;
        eprintln!("wrote {}", metrics.display());
        return Ok(report);
    }

    clear_test_circuits();
    let vk_for_host = vk;
    register_test_circuit(
        CID,
        "headstash",
        Box::new(move |proof_bytes, instance_bytes| {
            let inst = Instance::from_bytes(instance_bytes.to_vec());
            Proof::new(proof_bytes.to_vec())
                .verify(&vk_for_host, &[inst])
                .map(|_| true)
                .map_err(|_| VerificationError::GenericErr)
        }),
    );

    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    mock.add_balance(&admin, vec![Coin::new(1_000_000_000u128, denom.clone())])?;

    let hs = cw_headstash::interface::HeadstashContract::new(mock.clone());
    hs.upload().context("upload cw-headstash")?;
    hs.instantiate(
        &InstantiateMsg {
            genesis_root,
            distro_hash_domain: Default::default(),
            genesis_label: Some("product-a-clearnet-orch".into()),
            token_strategy: TokenStrategy::ExistingFungible(token),
            wavs: generate_test_wavs_proof(1),
            circuit_id: Some(CID),
        },
        Some(&admin),
        &[Coin::new(claim_value as u128, denom.clone())],
    )
    .context("instantiate headstash")?;

    mock.add_balance(
        &hs.address()?,
        vec![Coin::new(claim_value as u128, denom.clone())],
    )?;

    hs.execute(
        &ExecuteMsg::ProcessHeadstash {
            claims: vec![HeadstashNote {
                i: instance_to_msg(&inst_bytes, e),
                p: Binary::from(proof.as_ref().to_vec()),
                rr,
                root_id: 0,
            }],
        },
        &[],
    )
    .context("ProcessHeadstash proof_instance_verify")?;

    let report = HeadstashClaimReport {
        k: HEADSTASH_K,
        public_inputs: footer.i as u32,
        params_bytes,
        cs_bytes,
        vk_bytes,
        store_circuit_blob_bytes: blob_len,
        proof_bytes: proof.as_ref().len(),
        keygen_secs: if keygen_secs > 0.0 {
            keygen_secs
        } else {
            t0.elapsed().as_secs_f64()
        },
        prove_secs,
        verify_host_secs,
        gas_wanted: None,
        gas_used: None,
        gas_note: "cw-orch Mock AppResponse has no gas_used; sizes are the on-chain payload. \
                   ict-rs Daemon / live wasmd fills gas_used."
            .into(),
        claim_mock_verify: false,
        mint_denom: denom,
        claim_value,
    };

    let path = artifacts_dir().join("headstash_claim_metrics.json");
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    eprintln!("wrote {}", path.display());
    eprintln!(
        "headstash claim metrics:\n  k={} i={}\n  params={} cs={} vk={} blob={} proof={}\n  keygen={:.1}s prove={:.1}s verify={:.3}s gas={:?}",
        report.k,
        report.public_inputs,
        report.params_bytes,
        report.cs_bytes,
        report.vk_bytes,
        report.store_circuit_blob_bytes,
        report.proof_bytes,
        report.keygen_secs,
        report.prove_secs,
        report.verify_host_secs,
        report.gas_used,
    );

    clear_test_circuits();
    Ok(report)
}
