//! Product A clearnet claim: fresh Halo2 keys + prove + `proof_instance_verify`.
//!
//! Not mock verify. Not private-dex. Public `uterp` BankMsg::Send.
//!
//! ```text
//! cd crates/headstash
//! just demo-claim-clearnet
//! # or:
//! cargo test -p zk-test-press --test claim_clearnet --features "interface,zk" -- --ignored --nocapture
//! ```
//!
//! K=18 keygen + prove is slow (tens of minutes). Writes
//! `artifacts/headstash_claim_metrics.json`.

#![cfg(all(feature = "interface", feature = "zk"))]

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use cosmwasm_std::testing::{clear_test_circuits, register_test_circuit};
use cosmwasm_std::{Binary, Coin, StdError};
use cw_headstash::headstash::{HeadstashInstances, HeadstashNote};
use cw_headstash::msg::{ExecuteMsg, InstantiateMsg};
use cw_headstash::tokenfactory::{HeadstashTokenObject, TokenStrategy};
use cw_headstash::wavs::{WavsAuthMetadata, WavsProofOfOwnership};
use cw_orch::prelude::*;
use rand::rngs::OsRng;
use serde::Serialize;
use zk_headstash::circuit::{Instance, Proof, ProvingKey};
use zk_headstash::suite::suite::HeadstashProofBuilder;
use zk_headstash::suite::HeadstashCircuitSuite;

const CID: u64 = 1;
const DENOM: &str = "uterp";

#[derive(Serialize)]
struct ClaimMetrics {
    k: u32,
    public_inputs: u32,
    params_bytes: usize,
    cs_bytes: usize,
    vk_bytes: usize,
    store_circuit_blob_bytes: usize,
    proof_bytes: usize,
    keygen_secs: f64,
    prove_secs: f64,
    verify_host_secs: f64,
    gas_wanted: Option<u64>,
    gas_used: Option<u64>,
}

fn artifacts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../artifacts")
}

fn instance_to_msg(bytes: &[u8]) -> HeadstashInstances {
    assert_eq!(bytes.len(), 200);
    HeadstashInstances {
        anchor: Binary::from(bytes[0..32].to_vec()),
        nd: Binary::from(bytes[32..64].to_vec()),
        v: u64::from_le_bytes(bytes[64..72].try_into().unwrap()),
        nf: Binary::from(bytes[72..104].to_vec()),
        recp: Binary::from(bytes[104..136].to_vec()),
        cmx: Binary::from(bytes[136..168].to_vec()),
        e: Binary::from(bytes[168..200].to_vec()),
    }
}

#[test]
#[ignore = "K=18 keygen+prove; run: just demo-claim-clearnet"]
fn claim_clearnet_proof_instance_verify() {
    let t0 = Instant::now();
    let suite = HeadstashCircuitSuite;
    let (circuit, instance, anchor, _partial) = suite
        .suite_backed_claim_pair(4, 1)
        .expect("suite-backed claim");
    let inst_bytes = instance.to_bytes();
    assert_eq!(inst_bytes.len(), 200);
    let genesis_root = Binary::from(anchor.to_bytes().to_vec());
    let nd = Binary::from(inst_bytes[32..64].to_vec());
    let claim_value = u64::from_le_bytes(inst_bytes[64..72].try_into().unwrap());
    let recp = Binary::from(inst_bytes[104..136].to_vec());

    const K: u32 = 18;
    eprintln!("keygen K={K} (fresh)…");
    let pk = ProvingKey::build();
    let keygen_secs = t0.elapsed().as_secs_f64();
    let vk = pk.vk();

    let mut params_buf = Vec::new();
    pk.params().write(&mut params_buf).expect("params");
    let mut cs = halo2_proofs::plonk::ConstraintSystem::<pasta_curves::pallas::Base>::default();
    let _ = <zk_headstash::circuit::Circuit as halo2_proofs::plonk::Circuit<_>>::configure(&mut cs);
    let mut cs_buf = Vec::new();
    cs.write(&mut cs_buf).expect("cs");
    let mut vk_buf = Vec::new();
    vk.vk.write(&mut vk_buf).expect("vk");

    eprintln!("prove…");
    let t1 = Instant::now();
    let proof = Proof::create(&pk, &[circuit], &[instance.clone()], OsRng).expect("prove");
    let prove_secs = t1.elapsed().as_secs_f64();

    let t2 = Instant::now();
    proof.verify(&vk, &[instance.clone()]).expect("host verify");
    let verify_host_secs = t2.elapsed().as_secs_f64();

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
                .map_err(|e| StdError::msg(format!("{e:?}")))
        }),
    );

    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    mock.add_balance(&admin, vec![Coin::new(1_000_000_000u128, DENOM)])
        .expect("fund admin");

    let hs = cw_headstash::interface::HeadstashContract::new(mock.clone());
    hs.upload().expect("upload");
    hs.instantiate(
        &InstantiateMsg {
            genesis_root,
            distro_hash_domain: Default::default(),
            genesis_label: Some("product-a-clearnet".into()),
            token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject {
                proof: nd,
                raw: DENOM.into(),
            }),
            wavs: WavsProofOfOwnership {
                poos: vec![],
                msg: WavsAuthMetadata {
                    aggregate_key: String::new(),
                    threshold: 0,
                    total_operators: 0,
                    nonce: 0,
                },
            },
            circuit_id: Some(CID),
        },
        Some(&admin),
        &[],
    )
    .expect("instantiate");

    mock.add_balance(
        &hs.address().expect("addr"),
        vec![Coin::new(claim_value as u128, DENOM)],
    )
    .expect("escrow");

    let _exec = hs
        .execute(
            &ExecuteMsg::ProcessHeadstash {
                claims: vec![HeadstashNote {
                    i: instance_to_msg(&inst_bytes),
                    p: Binary::from(proof.as_ref().to_vec()),
                    rr: recp,
                    root_id: 0,
                }],
            },
            &[],
        )
        .expect("ProcessHeadstash proof_instance_verify");
    let (gas_wanted, gas_used): (Option<u64>, Option<u64>) = (None, None);

    let blob = params_buf.len() + cs_buf.len() + vk_buf.len() + 80;
    let metrics = ClaimMetrics {
        k: K,
        public_inputs: 6,
        params_bytes: params_buf.len(),
        cs_bytes: cs_buf.len(),
        vk_bytes: vk_buf.len(),
        store_circuit_blob_bytes: blob,
        proof_bytes: proof.as_ref().len(),
        keygen_secs,
        prove_secs,
        verify_host_secs,
        gas_wanted,
        gas_used,
    };
    eprintln!(
        "headstash claim metrics:\n  k={} i={}\n  params={} cs={} vk={} blob~{} proof={}\n  keygen={:.1}s prove={:.1}s verify={:.3}s",
        metrics.k,
        metrics.public_inputs,
        metrics.params_bytes,
        metrics.cs_bytes,
        metrics.vk_bytes,
        metrics.store_circuit_blob_bytes,
        metrics.proof_bytes,
        metrics.keygen_secs,
        metrics.prove_secs,
        metrics.verify_host_secs,
    );

    let dir = artifacts_dir();
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("headstash_claim_metrics.json");
    fs::write(&path, serde_json::to_vec_pretty(&metrics).unwrap()).expect("metrics");
    eprintln!("wrote {}", path.display());

    clear_test_circuits();
}
