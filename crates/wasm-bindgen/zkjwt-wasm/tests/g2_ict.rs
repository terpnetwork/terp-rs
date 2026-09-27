//! G2: prove + export VK, then ict-rs TestEnv (subprocess) when `ICT_ZKJWT=1`.
//!
//! ```text
//! ICT_ZKJWT=1 cargo test --release --offline --test g2_ict -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Marked `ignored-without-ICT`. Does not pull ict-rs into this crate graph
//! (Axiom vs zcash halo2_proofs clash).

use std::path::PathBuf;
use std::process::Command;

use halo2_base::halo2_proofs::arithmetic::CurveAffine;
use halo2_base::halo2_proofs::halo2curves::ff::Field;
use halo2_base::halo2_proofs::halo2curves::secp256r1::{Fq, Secp256r1Affine};
use halo2_base::utils::{biguint_to_fe, fe_to_biguint, modulus};
use rand::rngs::StdRng;
use rand::SeedableRng;
use zkjwt_wasm::{
    CIRCUIT_ID, claim_from_pk, compute_msg_bind, encode_instances, fr_to_bytes, prove_passkey,
    verify_passkey, write_store_full_circuit_files, PasskeyWitness,
};

fn require_ict() {
    assert_eq!(
        std::env::var("ICT_ZKJWT").ok().as_deref(),
        Some("1"),
        "set ICT_ZKJWT=1 (ignored-without-ICT)"
    );
}

fn signed_witness(msg_bind: [u8; 32]) -> PasskeyWitness {
    let mut rng = StdRng::seed_from_u64(42);
    let sk = Fq::random(&mut rng);
    let pk = Secp256r1Affine::from(Secp256r1Affine::generator() * sk);
    let auth = b"auth-data-webauthn";
    let client = br#"{"type":"webauthn.get","challenge":"AA","origin":"https://terp.network"}"#;
    let msghash = zkjwt_wasm::webauthn_msghash(auth, client);
    let k = Fq::random(&mut rng);
    let k_inv = k.invert().unwrap();
    let r_pt = Secp256r1Affine::from(Secp256r1Affine::generator() * k);
    let xy = r_pt.coordinates().unwrap();
    let r = biguint_to_fe::<Fq>(&(fe_to_biguint(xy.x()) % modulus::<Fq>()));
    let s = k_inv * (msghash + (r * sk));
    PasskeyWitness {
        pk,
        r,
        s,
        msghash,
        msg_bind,
    }
}

fn b64(data: &[u8]) -> String {
    const CH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CH[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CH[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CH[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CH[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn prepare_artifacts() -> PathBuf {
    let dir = PathBuf::from("/tmp/zkjwt-g2-artifacts");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_store_full_circuit_files(&dir).expect("export store-full-circuit");

    // Real MsgExecuteContract-shaped sign bytes (type URL + fields).
    let sign_mode_direct = b"/cosmwasm.wasm.v1.MsgExecuteContract|sender=terp1alice|contract=terp1zkjwt|msg={\"register_claim\":{}}";
    let msg_bind = compute_msg_bind("ict-zkjwt-g2", "terp1alice", "0", 0, sign_mode_direct);
    let w = signed_witness(msg_bind);
    let inst = encode_instances(&w);
    assert_eq!(&inst[96..128], &zkjwt_wasm::fr_to_bytes(zkjwt_wasm::public_frs(&w)[3])[..]);
    let proof = prove_passkey(&w).expect("prove");
    assert!(verify_passkey(&proof, &w).unwrap());
    let mut bad = w.clone();
    bad.msg_bind[0] ^= 0xff;
    assert!(!verify_passkey(&proof, &bad).unwrap(), "wrong bind fail");

    std::fs::write(dir.join("proof.bin"), &proof).unwrap();
    std::fs::write(dir.join("instances.bin"), &inst).unwrap();
    std::fs::write(dir.join("claim.bin"), fr_to_bytes(claim_from_pk(&w.pk))).unwrap();
    std::fs::write(dir.join("msg_bind.bin"), msg_bind).unwrap();
    let payload = serde_json::json!({
        "issuer": "terp.network/passkey-zkjwt",
        "claim_commitment": b64(&fr_to_bytes(claim_from_pk(&w.pk))),
        "public_inputs": b64(&inst),
        "proof": b64(&proof),
        "circuit_id": CIRCUIT_ID,
    });
    std::fs::write(dir.join("payload.json"), payload.to_string()).unwrap();
    println!("PASS prove proof_len={}", proof.len());
    dir
}

fn spawn_ict_rs(dir: &PathBuf) {
    let ict_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../ict-rs/ict-rs/Cargo.toml");
    assert!(
        ict_manifest.exists(),
        "ict-rs Cargo.toml missing at {}",
        ict_manifest.display()
    );
    let status = Command::new("cargo")
        .args([
            "test",
            "--release",
            "--offline",
            "--manifest-path",
        ])
        .arg(&ict_manifest)
        .args([
            "--features",
            "docker,terp,testing",
            "--test",
            "zkjwt_passkey_g2",
            "--",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("ICT_ZKJWT", "1")
        .env("ZKJWT_G2_DIR", dir)
        .env("PATH", format!("/usr/local/bin:{}", std::env::var("PATH").unwrap_or_default()))
        .status()
        .expect("spawn ict-rs g2");
    assert!(status.success(), "ict-rs zkjwt_passkey_g2 failed: {status}");
}

#[test]
#[ignore = "ignored-without-ICT"]
fn g2_ict_zkjwt_passkey_p256() {
    require_ict();
    let dir = prepare_artifacts();
    spawn_ict_rs(&dir);
}

#[test]
#[ignore = "ignored-without-ICT"]
fn g2_executive_register_prove_authenticate() {
    require_ict();
    let dir = prepare_artifacts();
    spawn_ict_rs(&dir);
}
