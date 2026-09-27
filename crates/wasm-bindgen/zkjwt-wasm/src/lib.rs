#![feature(generic_const_exprs)]
#![allow(incomplete_features)]
//! zkjwt.passkey.p256.v1 — P-256 ECDSA inside Halo2 (k >= 17).
//!
//! Public instances (one column, 4 × 32B = 128B) match terp-zkjwt Authenticate:
//!   [0] nullifier
//!   [1] claim = field(SHA256(b"terp-zkjwt/pk/v1" || SEC1(pk)))
//!   [2] inclusion_set_root  (constrained 0)
//!   [3] msg_bind = compute_msg_bind(...) 32B as field
//!
//! Private: P-256 pk, (r,s), authData, canonical clientDataJSON.
//! ECDSA is verified in-circuit; skipping it fails `ecdsa_must_run`.

#![allow(clippy::too_many_arguments)]

use halo2_base::halo2_proofs::arithmetic::CurveAffine;
use halo2_base::halo2_proofs::halo2curves::ff::{Field, PrimeField};
use halo2_base::gates::circuit::{builder::BaseCircuitBuilder, CircuitBuilderStage};
use halo2_base::gates::RangeChip;
use halo2_base::halo2_proofs::halo2curves::bn256::{Bn256, Fr, G1Affine};
use halo2_base::halo2_proofs::halo2curves::secp256r1::{Fp, Fq, Secp256r1Affine};
use halo2_base::halo2_proofs::plonk::{
    create_proof, keygen_pk, keygen_vk, verify_proof, ProvingKey, VerifyingKey,
};
use halo2_base::halo2_proofs::poly::commitment::ParamsProver;
use halo2_base::halo2_proofs::poly::kzg::{
    commitment::{KZGCommitmentScheme, ParamsKZG},
    multiopen::{ProverSHPLONK, VerifierSHPLONK},
    strategy::SingleStrategy,
};
use halo2_base::halo2_proofs::transcript::{
    Blake2bRead, Blake2bWrite, Challenge255, TranscriptReadBuffer, TranscriptWriterBuffer,
};
use halo2_base::utils::{biguint_to_fe, fe_to_biguint, modulus};
use halo2_base::{AssignedValue, Context};
use halo2_ecc::ecc::ecdsa::ecdsa_verify_no_pubkey_check;
use halo2_ecc::ecc::with_weierstrass_a;
use halo2_ecc::ecc::EccChip;
use halo2_ecc::fields::FieldChip;
use halo2_ecc::secp256r1::{FpChip, FqChip};
use num_bigint::BigUint;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use wasm_bindgen::prelude::*;

pub const K: u32 = 17;
pub const LOOKUP_BITS: usize = 16;
pub const LIMB_BITS: usize = 88;
pub const NUM_LIMBS: usize = 3;
pub const INSTANCE_COUNT: usize = 4;
pub const CIRCUIT_ID: &str = "zkjwt.passkey.p256.v1";
pub const CLAIM_DOMAIN: &[u8] = b"terp-zkjwt/pk/v1";
pub const NF_DOMAIN: &[u8] = b"terp-zkjwt/nf/v1";
pub const MSG_BIND_DOMAIN: &[u8] = b"terp-auth/zkjwt/v1";


fn p256_a_le() -> [u8; 32] {
    let a = -Fp::from(3u64);
    a.to_bytes()
}

struct Keys {
    params: ParamsKZG<Bn256>,
    pk: ProvingKey<G1Affine>,
    break_points: halo2_base::gates::flex_gate::MultiPhaseThreadBreakPoints,
    config_params: halo2_base::gates::circuit::BaseCircuitParams,
}

static KEYS: OnceLock<Result<Keys, String>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct PasskeyWitness {
    pub pk: Secp256r1Affine,
    pub r: Fq,
    pub s: Fq,
    pub msghash: Fq,
    pub msg_bind: [u8; 32],
}

pub fn compute_msg_bind(
    chain_id: &str,
    account: &str,
    authenticator_id: &str,
    msg_index: u64,
    sign_mode_direct: &[u8],
) -> [u8; 32] {
    let msg_hash = Sha256::digest(sign_mode_direct);
    let mut h = Sha256::new();
    h.update(MSG_BIND_DOMAIN);
    h.update(chain_id.as_bytes());
    h.update([0u8]);
    h.update(account.as_bytes());
    h.update([0u8]);
    h.update(authenticator_id.as_bytes());
    h.update([0u8]);
    h.update(msg_index.to_le_bytes());
    h.update(msg_hash);
    h.finalize().into()
}

pub fn sec1_uncompressed(pk: &Secp256r1Affine) -> [u8; 65] {
    let mut out = [0u8; 65];
    out[0] = 0x04;
    out[1..33].copy_from_slice(&pk.x.to_bytes());
    out[33..65].copy_from_slice(&pk.y.to_bytes());
    out
}

pub fn sha256_to_fr(bytes: &[u8]) -> Fr {
    let d = Sha256::digest(bytes);
    let n = BigUint::from_bytes_be(&d) % modulus::<Fr>();
    biguint_to_fe(&n)
}

pub fn claim_from_pk(pk: &Secp256r1Affine) -> Fr {
    let mut buf = Vec::with_capacity(CLAIM_DOMAIN.len() + 65);
    buf.extend_from_slice(CLAIM_DOMAIN);
    buf.extend_from_slice(&sec1_uncompressed(pk));
    sha256_to_fr(&buf)
}

pub fn nullifier_from_pk(pk: &Secp256r1Affine, msg_bind: &[u8; 32]) -> Fr {
    let mut buf = Vec::with_capacity(NF_DOMAIN.len() + 65 + 32);
    buf.extend_from_slice(NF_DOMAIN);
    buf.extend_from_slice(&sec1_uncompressed(pk));
    buf.extend_from_slice(msg_bind);
    sha256_to_fr(&buf)
}

pub fn fr_to_bytes(f: Fr) -> [u8; 32] {
    f.to_bytes()
}

pub fn bytes_to_fr(b: &[u8; 32]) -> Fr {
    Option::from(Fr::from_bytes(b)).unwrap_or(Fr::ZERO)
}

pub fn webauthn_msghash(auth_data: &[u8], client_data_json: &[u8]) -> Fq {
    let cd_hash = Sha256::digest(client_data_json);
    let mut m = Vec::with_capacity(auth_data.len() + 32);
    m.extend_from_slice(auth_data);
    m.extend_from_slice(&cd_hash);
    let digest = Sha256::digest(&m);
    let n = BigUint::from_bytes_be(&digest) % modulus::<Fq>();
    biguint_to_fe(&n)
}

pub fn public_frs(w: &PasskeyWitness) -> [Fr; INSTANCE_COUNT] {
    [
        nullifier_from_pk(&w.pk, &w.msg_bind),
        claim_from_pk(&w.pk),
        Fr::ZERO,
        {
            let n = BigUint::from_bytes_be(&w.msg_bind) % modulus::<Fr>();
            biguint_to_fe(&n)
        },
    ]
}

pub fn encode_instances(w: &PasskeyWitness) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    for f in public_frs(w) {
        out.extend_from_slice(&fr_to_bytes(f));
    }
    out
}

fn assign_passkey(
    ctx: &mut Context<Fr>,
    range: &RangeChip<Fr>,
    w: &PasskeyWitness,
) -> AssignedValue<Fr> {
    let fp_chip = FpChip::<Fr>::new(range, LIMB_BITS, NUM_LIMBS);
    let fq_chip = FqChip::<Fr>::new(range, LIMB_BITS, NUM_LIMBS);
    let m = fq_chip.load_private(ctx, w.msghash);
    let r = fq_chip.load_private(ctx, w.r);
    let s = fq_chip.load_private(ctx, w.s);
    let ecc = EccChip::<Fr, FpChip<Fr>>::new(&fp_chip);
    let pk = ecc.load_private_unchecked(ctx, (w.pk.x, w.pk.y));
    ecdsa_verify_no_pubkey_check::<Fr, Fp, Fq, Secp256r1Affine>(&ecc, ctx, pk, r, s, m, 4, 4)
}

fn populate_builder(
    builder: &mut BaseCircuitBuilder<Fr>,
    w: &PasskeyWitness,
) {
    with_weierstrass_a(-3, || {
    populate_builder_inner(builder, w);
    });
}

fn populate_builder_inner(
    builder: &mut BaseCircuitBuilder<Fr>,
    w: &PasskeyWitness,
) {
    builder.set_lookup_bits(LOOKUP_BITS);
    builder.set_instance_columns(1);
    let range = RangeChip::new(LOOKUP_BITS, builder.lookup_manager().clone());
    let ok = {
        let ctx = builder.main(0);
        assign_passkey(ctx, &range, w)
    };
    {
        let ctx = builder.main(0);
        let one = ctx.load_constant(Fr::ONE);
        ctx.constrain_equal(&ok, &one);
    }

    let pubs = public_frs(w);
    let mut cells = Vec::new();
    {
        let ctx = builder.main(0);
        for p in pubs {
            cells.push(ctx.load_witness(p));
        }
        let zero = ctx.load_constant(Fr::ZERO);
        ctx.constrain_equal(&cells[2], &zero);
    }
    builder.assigned_instances[0].extend(cells);
}

/// Deterministic valid ECDSA witness for keygen/bench (not a WebAuthn fixture).
pub fn dummy_witness() -> PasskeyWitness {
    use halo2_base::halo2_proofs::arithmetic::CurveAffine;
    use halo2_base::halo2_proofs::halo2curves::ff::Field;
    let sk = Fq::from(3);
    let pk = Secp256r1Affine::from(Secp256r1Affine::generator() * sk);
    let msghash = Fq::from(7);
    let k = Fq::from(5);
    let k_inv = k.invert().unwrap();
    let r_pt = Secp256r1Affine::from(Secp256r1Affine::generator() * k);
    let r = biguint_to_fe::<Fq>(&(fe_to_biguint(&r_pt.x) % modulus::<Fq>()));
    let s = k_inv * (msghash + (r * sk));
    PasskeyWitness {
        pk,
        r,
        s,
        msghash,
        msg_bind: [1u8; 32],
    }
}

fn keys() -> Result<&'static Keys, String> {
    KEYS.get_or_init(|| {
        let mut builder =
            BaseCircuitBuilder::from_stage(CircuitBuilderStage::Keygen).use_k(K as usize);
        populate_builder(&mut builder, &dummy_witness());
        let config_params = builder.calculate_params(Some(20));
        let params = ParamsKZG::<Bn256>::setup(K, OsRng);
        let vk = keygen_vk(&params, &builder).map_err(|e| format!("keygen_vk: {e:?}"))?;
        let pk = keygen_pk(&params, vk, &builder).map_err(|e| format!("keygen_pk: {e:?}"))?;
        let break_points = builder.break_points();
        Ok(Keys {
            params,
            pk,
            break_points,
            config_params,
        })
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// Force one-time params + PK generation (cached in process).
pub fn warmup_keys() -> Result<(), String> {
    keys().map(|_| ())
}

/// CosmWasm Path A footer (`curve_id=6` Halo2-axiom KZG BN256).
pub const STORE_CURVE_ID: u8 = 6;
pub const STORE_PROVER_ID: u8 = 0; // CircuitType::Plonkish
pub const COSMWASM_FOOTER_LEN: usize = 80;

/// Params + vk_body for `terpd tx wasm store-full-circuit`.
///
/// `vk_body` = `[BaseCircuitParams JSON | VK | 80-byte CircuitFooter]`.
/// Host concatenates `[params | vk_body]` and `check_circuit` reads the footer.
pub fn export_store_full_circuit() -> Result<(Vec<u8>, Vec<u8>), String> {
    use halo2_base::halo2_proofs::SerdeFormat;
    let keys = keys()?;
    let mut params_body = Vec::new();
    keys.params
        .write_custom(&mut params_body, SerdeFormat::RawBytesUnchecked)
        .map_err(|e| format!("params.write: {e:?}"))?;
    let cs = serde_json::to_vec(&keys.config_params).map_err(|e| format!("config json: {e}"))?;
    let mut vk_only = Vec::new();
    keys.pk
        .get_vk()
        .write(&mut vk_only, SerdeFormat::RawBytesUnchecked)
        .map_err(|e| format!("vk.write: {e:?}"))?;
    let mut vk_region = Vec::with_capacity(cs.len() + vk_only.len());
    vk_region.extend_from_slice(&cs);
    vk_region.extend_from_slice(&vk_only);
    let param_hash = Sha256::digest(&params_body);
    let vk_hash = Sha256::digest(&vk_region);
    let mut footer = [0u8; COSMWASM_FOOTER_LEN];
    footer[0] = STORE_PROVER_ID;
    footer[1] = STORE_CURVE_ID;
    footer[2] = K as u8;
    footer[3] = INSTANCE_COUNT as u8;
    footer[4..8].copy_from_slice(&(params_body.len() as u32).to_le_bytes());
    footer[8..12].copy_from_slice(&(cs.len() as u32).to_le_bytes());
    footer[12..16].copy_from_slice(&(vk_only.len() as u32).to_le_bytes());
    footer[16..48].copy_from_slice(&param_hash);
    footer[48..80].copy_from_slice(&vk_hash);
    let mut vk_body = vk_region;
    vk_body.extend_from_slice(&footer);
    Ok((params_body, vk_body))
}

pub fn write_store_full_circuit_files(dir: &std::path::Path) -> Result<(), String> {
    let (params, vk) = export_store_full_circuit()?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("params.bin"), &params).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("vk_body.bin"), &vk).map_err(|e| e.to_string())?;
    Ok(())
}

/// Halo2 flex-gate config after keygen (for bench notes).
pub fn circuit_config_debug() -> Result<String, String> {
    let keys = keys()?;
    Ok(format!("{:?}", keys.config_params))
}

pub fn prove_passkey(w: &PasskeyWitness) -> Result<Vec<u8>, String> {
    let keys = keys()?;
    let mut builder =
        BaseCircuitBuilder::prover(keys.config_params.clone(), keys.break_points.clone());
    populate_builder(&mut builder, w);
    let instances = public_frs(w);
    let inst_ref: &[&[Fr]] = &[&instances];
    let mut transcript = Blake2bWrite::<_, G1Affine, Challenge255<_>>::init(vec![]);
    create_proof::<
        KZGCommitmentScheme<Bn256>,
        ProverSHPLONK<'_, Bn256>,
        Challenge255<_>,
        _,
        Blake2bWrite<Vec<u8>, G1Affine, Challenge255<G1Affine>>,
        _,
    >(&keys.params, &keys.pk, &[builder], &[inst_ref], OsRng, &mut transcript)
    .map_err(|e| format!("create_proof: {e:?}"))?;
    Ok(transcript.finalize())
}

pub fn verify_passkey(proof: &[u8], w: &PasskeyWitness) -> Result<bool, String> {
    let keys = keys()?;
    let instances = public_frs(w);
    let inst_ref: &[&[Fr]] = &[&instances];
    let strategy = SingleStrategy::new(&keys.params);
    let mut transcript = Blake2bRead::<_, G1Affine, Challenge255<_>>::init(proof);
    match verify_proof::<
        KZGCommitmentScheme<Bn256>,
        VerifierSHPLONK<'_, Bn256>,
        Challenge255<G1Affine>,
        Blake2bRead<&[u8], G1Affine, Challenge255<G1Affine>>,
        SingleStrategy<'_, Bn256>,
    >(
        keys.params.verifier_params(),
        keys.pk.get_vk(),
        strategy,
        &[inst_ref],
        &mut transcript,
    ) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn b64_encode(data: &[u8]) -> String {
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

fn parse_hex32(s: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(s.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    if raw.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", raw.len()));
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&raw);
    Ok(a)
}

fn fq_from_be(bytes: &[u8; 32]) -> Fq {
    Fq::from_bytes(bytes).unwrap_or(Fq::ZERO)
}

fn pk_from_sec1(sec1: &[u8]) -> Result<Secp256r1Affine, String> {
    if sec1.len() != 65 || sec1[0] != 0x04 {
        return Err("SEC1 uncompressed 65B required".into());
    }
    let mut xb = [0u8; 32];
    let mut yb = [0u8; 32];
    xb.copy_from_slice(&sec1[1..33]);
    yb.copy_from_slice(&sec1[33..65]);
    let x = Option::<Fp>::from(Fp::from_bytes(&xb)).ok_or("bad pk.x")?;
    let y = Option::<Fp>::from(Fp::from_bytes(&yb)).ok_or("bad pk.y")?;
    Option::from(Secp256r1Affine::from_xy(x, y)).ok_or_else(|| "pk not on curve".into())
}

/// Browser entry: prove a WebAuthn P-256 assertion.
/// `msg_bind_hex` is 32-byte compute_msg_bind; `r_hex`/`s_hex`/`pk_sec1_hex` are hex.
#[wasm_bindgen]
pub fn generate_proof(
    pk_sec1_hex: &str,
    r_hex: &str,
    s_hex: &str,
    auth_data_hex: &str,
    client_data_json: &str,
    msg_bind_hex: &str,
) -> Result<String, JsValue> {
    let pk_bytes = hex::decode(pk_sec1_hex.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    let pk = pk_from_sec1(&pk_bytes).map_err(|e| JsValue::from_str(&e))?;
    let r = fq_from_be(&parse_hex32(r_hex).map_err(|e| JsValue::from_str(&e))?);
    let s = fq_from_be(&parse_hex32(s_hex).map_err(|e| JsValue::from_str(&e))?);
    let auth = hex::decode(auth_data_hex.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    let msg_bind = parse_hex32(msg_bind_hex).map_err(|e| JsValue::from_str(&e))?;
    let w = PasskeyWitness {
        pk,
        r,
        s,
        msghash: webauthn_msghash(&auth, client_data_json.as_bytes()),
        msg_bind,
    };
    let proof = prove_passkey(&w).map_err(|e| JsValue::from_str(&e))?;
    Ok(b64_encode(&proof))
}

#[wasm_bindgen]
pub fn encode_instances_js(pk_sec1_hex: &str, msg_bind_hex: &str) -> Result<Vec<u8>, JsValue> {
    let pk_bytes = hex::decode(pk_sec1_hex.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    let pk = pk_from_sec1(&pk_bytes).map_err(|e| JsValue::from_str(&e))?;
    let msg_bind = parse_hex32(msg_bind_hex).map_err(|e| JsValue::from_str(&e))?;
    let w = PasskeyWitness {
        pk,
        r: Fq::ONE,
        s: Fq::ONE,
        msghash: Fq::ONE,
        msg_bind,
    };
    Ok(encode_instances(&w))
}

#[wasm_bindgen]
pub fn encode_register_claim(pk_sec1_hex: &str, issuer: &str) -> Result<String, JsValue> {
    let pk_bytes = hex::decode(pk_sec1_hex.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    let pk = pk_from_sec1(&pk_bytes).map_err(|e| JsValue::from_str(&e))?;
    let c = claim_from_pk(&pk);
    Ok(serde_json::json!({
        "register_claim": {
            "claim_commitment": b64_encode(&fr_to_bytes(c)),
            "issuer": issuer,
        }
    })
    .to_string())
}

#[wasm_bindgen]
pub fn encode_auth_payload(
    pk_sec1_hex: &str,
    msg_bind_hex: &str,
    issuer: &str,
    proof_b64: &str,
) -> Result<String, JsValue> {
    let pk_bytes = hex::decode(pk_sec1_hex.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    let pk = pk_from_sec1(&pk_bytes).map_err(|e| JsValue::from_str(&e))?;
    let msg_bind = parse_hex32(msg_bind_hex).map_err(|e| JsValue::from_str(&e))?;
    let w = PasskeyWitness {
        pk,
        r: Fq::ONE,
        s: Fq::ONE,
        msghash: Fq::ONE,
        msg_bind,
    };
    let inst = encode_instances(&w);
    let c = claim_from_pk(&pk);
    Ok(serde_json::json!({
        "issuer": issuer,
        "claim_commitment": b64_encode(&fr_to_bytes(c)),
        "public_inputs": b64_encode(&inst),
        "proof": proof_b64,
        "circuit_id": CIRCUIT_ID,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use halo2_base::halo2_proofs::arithmetic::CurveAffine;
    use halo2_base::halo2_proofs::halo2curves::ff::Field;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn signed_witness(flip_bind: bool, bad_s: bool) -> PasskeyWitness {
        let mut rng = StdRng::seed_from_u64(42);
        let sk = Fq::random(&mut rng);
        let pk = Secp256r1Affine::from(Secp256r1Affine::generator() * sk);
        let auth = b"auth-data-webauthn";
        let client = br#"{"type":"webauthn.get","challenge":"AA","origin":"https://terp.network"}"#;
        let msghash = webauthn_msghash(auth, client);
        let k = Fq::random(&mut rng);
        let k_inv = k.invert().unwrap();
        let r_pt = Secp256r1Affine::from(Secp256r1Affine::generator() * k);
        let xy = r_pt.coordinates().unwrap();
    let x = xy.x();
        let r = biguint_to_fe::<Fq>(&(fe_to_biguint(x) % modulus::<Fq>()));
        let mut s = k_inv * (msghash + (r * sk));
        if bad_s {
            s += Fq::ONE;
        }
        let mut msg_bind = compute_msg_bind("terp-test", "terp1alice", "zkjwt", 0, b"sign-bytes");
        if flip_bind {
            msg_bind[0] ^= 0xff;
        }
        PasskeyWitness {
            pk,
            r,
            s,
            msghash,
            msg_bind,
        }
    }

    #[test]
    fn k_is_at_least_17() {
        assert!(K >= 17, "passkey P-256 requires k>=17, got {K}");
        assert_ne!(CIRCUIT_ID, "zkjwt.membership.v1");
    }

    #[test]
    fn instances_are_128_bytes() {
        let w = signed_witness(false, false);
        let b = encode_instances(&w);
        assert_eq!(b.len(), 128);
        assert_eq!(&b[64..96], &[0u8; 32]);
    }

    #[test]
    fn claim_is_sha256_of_sec1() {
        let w = signed_witness(false, false);
        let pubs = public_frs(&w);
        assert_eq!(pubs[1], claim_from_pk(&w.pk));
        assert_eq!(fr_to_bytes(pubs[2]), [0u8; 32]);
        let mb = {
            let n = BigUint::from_bytes_be(&w.msg_bind) % modulus::<Fr>();
            biguint_to_fe::<Fr>(&n)
        };
        assert_eq!(pubs[3], mb);
    }

    #[test]
    fn msg_bind_matches_compute() {
        let mb = compute_msg_bind("c", "a", "id", 1, b"tx");
        assert_eq!(mb.len(), 32);
        let w = signed_witness(false, false);
        assert_eq!(w.msg_bind, compute_msg_bind("terp-test", "terp1alice", "zkjwt", 0, b"sign-bytes"));
    }

    #[test]
    fn prove_verify_roundtrip() {
        let w = signed_witness(false, false);
        let proof = prove_passkey(&w).expect("prove");
        assert!(!proof.is_empty());
        assert!(verify_passkey(&proof, &w).unwrap());
    }

    #[test]
    fn msg_bind_flipped_fails() {
        let w = signed_witness(false, false);
        let proof = prove_passkey(&w).expect("prove");
        let mut bad = w.clone();
        bad.msg_bind[0] ^= 0xff;
        assert!(!verify_passkey(&proof, &bad).unwrap());
    }

    #[test]
    fn ecdsa_must_run() {
        // Invalid (r,s) cannot satisfy the ECDSA gate: a produced proof must not verify.
        let w = signed_witness(false, true);
        match prove_passkey(&w) {
            Err(_) => {}
            Ok(proof) => {
                assert!(
                    !verify_passkey(&proof, &w).unwrap(),
                    "invalid ECDSA must not verify"
                );
            }
        }
    }

    #[test]
    fn payload_has_no_secret_or_credential_id() {
        let w = signed_witness(false, false);
        let sec1 = hex::encode(sec1_uncompressed(&w.pk));
        let j = encode_register_claim(&sec1, "terp.network/passkey-zkjwt").unwrap();
        assert!(!j.contains("credential_id"));
        assert_eq!(CIRCUIT_ID, "zkjwt.passkey.p256.v1");
    }
}
