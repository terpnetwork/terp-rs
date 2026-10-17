//! Unit tests for foreign field arithmetic - Secp256k1 key pairing
//!
//! These tests verify that we can correctly represent secp256k1 field elements
//! as 4x64-bit limbs in pallas::Base and perform elliptic curve pairing checks.

use std::println;

use super::secp256k1_chip::*;
use ff::{Field, PrimeField};
use halo2_base::halo2_proofs::halo2curves::secp256k1::{Fp as Secp256k1Fp, Fq as Secp256k1Fq};
use halo2_gadgets::utilities::lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig};
use halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner, Value},
    dev::MockProver,
    plonk::{Circuit, ConstraintSystem, Error as PlonkError, TableColumn},
};
use num_traits::Zero;
use pasta_curves::pallas;
use secp256k1::constants::{GENERATOR_X, GENERATOR_Y};

// ============================================================================
// Test Circuit for Secp256k1 Key Pairing
// ============================================================================

#[derive(Clone, Debug)]
struct Secp256k1TestConfig {
    secp_config: Secp256k1Config,
    table_idx: TableColumn,
}

impl Secp256k1TestConfig {
    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self {
        // Allocate 9 advice columns (shared between Fp and Fq)
        let advices: [_; 9] = core::array::from_fn(|_| meta.advice_column());

        // Fixed column for constants
        let constants = meta.fixed_column();
        meta.enable_constant(constants);

        // Create lookup table for range checking (K=10)
        let table_idx = meta.lookup_table_column();
        let range_check = LookupRangeCheckConfig::configure(meta, advices[0], table_idx);

        // Configure Secp256k1 chip with 9 shared columns
        let secp_config = Secp256k1Config::configure(meta, advices, range_check);

        Self {
            secp_config,
            table_idx,
        }
    }
}

/// Circuit that proves: pk = sk * G (secp256k1 key pairing)
#[derive(Default)]
struct KeyPairingTestCircuit {
    sk: Secp256k1Fq,
    pk_x: Secp256k1Fp,
    pk_y: Secp256k1Fp,
}

impl Circuit<pallas::Base> for KeyPairingTestCircuit {
    type Config = Secp256k1TestConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        Secp256k1TestConfig::configure(meta)
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), PlonkError> {
        // Load the lookup range check table
        layouter.assign_table(
            || "range_check_table",
            |mut table| {
                for index in 0..(1 << 10) {
                    table.assign_cell(
                        || "table_idx",
                        config.table_idx,
                        index,
                        || Value::known(pallas::Base::from(index as u64)),
                    )?;
                }
                Ok(())
            },
        )?;

        // Construct the Secp256k1Chip
        let secp_chip = Secp256k1Chip::construct(config.secp_config.clone());

        // Prove key pairing: pk = sk * G
        let (_sk_assigned, (_pk_x_assigned, _pk_y_assigned)) = secp_chip.prove_key_pairing(
            layouter.namespace(|| "secp256k1 key pairing"),
            Value::known(self.sk),
            Value::known(self.pk_x),
            Value::known(self.pk_y),
        )?;

        Ok(())
    }
}

/// Proves a wallet ECDSA signature under `pk` for challenge `e`.
struct EcdsaVerifyCircuit {
    e: Secp256k1Fq,
    r: Secp256k1Fq,
    s: Secp256k1Fq,
    pk_x: Secp256k1Fp,
    pk_y: Secp256k1Fp,
}

impl Circuit<pallas::Base> for EcdsaVerifyCircuit {
    type Config = Secp256k1TestConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self {
            e: Secp256k1Fq::from(1u64),
            r: Secp256k1Fq::from(1u64),
            s: Secp256k1Fq::from(1u64),
            pk_x: Secp256k1Fp::from(1u64),
            pk_y: Secp256k1Fp::from(1u64),
        }
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        Secp256k1TestConfig::configure(meta)
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), PlonkError> {
        layouter.assign_table(
            || "range_check_table",
            |mut table| {
                for index in 0..(1 << 10) {
                    table.assign_cell(
                        || "table_idx",
                        config.table_idx,
                        index,
                        || Value::known(pallas::Base::from(index as u64)),
                    )?;
                }
                Ok(())
            },
        )?;
        let chip = Secp256k1Chip::construct(config.secp_config);
        chip.prove_ecdsa_verify(
            layouter.namespace(|| "ecdsa"),
            Value::known(self.e),
            Value::known(self.r),
            Value::known(self.s),
            Value::known(self.pk_x),
            Value::known(self.pk_y),
        )?;
        Ok(())
    }
}

fn fq_from_be(bytes: &[u8; 32]) -> Secp256k1Fq {
    let mut le = *bytes;
    le.reverse();
    secp_fq_from_le(le)
}

fn reduce_mod_n(bytes: &[u8; 32]) -> [u8; 32] {
    use num_bigint::BigUint;
    let n = BigUint::parse_bytes(
        b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
        16,
    )
    .unwrap();
    let reduced = BigUint::from_bytes_be(bytes) % n;
    let be = reduced.to_bytes_be();
    let mut out = [0u8; 32];
    if !be.is_empty() {
        out[32 - be.len()..].copy_from_slice(&be);
    }
    out
}

#[test]
fn test_ecdsa_verify_accepts_wallet_signature() {
    use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

    use crate::claim_auth::claim_challenge;

    let secp = Secp256k1::new();
    let sk = SecretKey::from_byte_array([0x11; 32]).unwrap();
    let pk = PublicKey::from_secret_key(&secp, &sk);
    let body = [0xab; 32];
    let challenge = claim_challenge(&body);
    let sig = secp.sign_ecdsa(Message::from_digest(challenge), &sk);
    let compact = sig.serialize_compact();
    let mut r_bytes = [0u8; 32];
    let mut s_bytes = [0u8; 32];
    r_bytes.copy_from_slice(&compact[..32]);
    s_bytes.copy_from_slice(&compact[32..]);
    let raw = pk.serialize_uncompressed();
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x.copy_from_slice(&raw[1..33]);
    y.copy_from_slice(&raw[33..65]);

    let circuit = EcdsaVerifyCircuit {
        e: fq_from_be(&reduce_mod_n(&challenge)),
        r: fq_from_be(&r_bytes),
        s: fq_from_be(&s_bytes),
        pk_x: secp_fp_from_coord_be(&x),
        pk_y: secp_fp_from_coord_be(&y),
    };
    // Two GLV multiplications. K=19 leaves room for the variable-base offset doubles.
    let prover = MockProver::run(19, &circuit, vec![]).expect("synthesize");
    assert_eq!(prover.verify(), Ok(()));
}

// ============================================================================
// Test: Valid Secp256k1 Key Pair (Should Pass)
// ============================================================================

#[test]
fn test_secp256k1_key_pairing_valid() {
    use secp256k1::{Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let sk_bytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ];

    let sk_secp = SecretKey::from_byte_array(sk_bytes).expect("valid secret key");
    let pk_secp = sk_secp.public_key(&secp);

    let pk_bytes = pk_secp.serialize_uncompressed();
    assert_eq!(pk_bytes[0], 0x04, "First byte should be 0x04");

    let (pk_x_bytes, pk_y_bytes): ([u8; 32], [u8; 32]) = (
        pk_bytes[1..33].try_into().unwrap(),
        pk_bytes[33..65].try_into().unwrap(),
    );

    let mut sk_le = sk_bytes;
    sk_le.reverse();
    let sk = secp_fq_from_le(sk_le);
    let mut pk_x_le = pk_x_bytes;
    pk_x_le.reverse();
    let pk_x = secp_fp_from_le(pk_x_le);
    let mut pk_y_le = pk_y_bytes;
    pk_y_le.reverse();
    let pk_y = secp_fp_from_le(pk_y_le);

    println!("Testing VALID key pair:");
    println!("  sk:   {:?}", hex::encode(sk_bytes));
    println!("  pk.x: {:?}", hex::encode(pk_x_bytes));
    println!("  pk.y: {:?}", hex::encode(pk_y_bytes));

    let circuit = KeyPairingTestCircuit { sk, pk_x, pk_y };

    let prover = MockProver::run(18, &circuit, vec![]).expect("prover should run");

    assert_eq!(
        prover.verify(),
        Ok(()),
        "Valid key pair should verify successfully"
    );
}

#[test]
fn test_reconstruct_xy_from_limbs() {
    use halo2_base::utils::fe_to_biguint;
    use num_bigint::BigUint;

    let gen_x_fp = Secp256k1Fp::from_bytes(&GENERATOR_X).unwrap();
    let gen_y_fp = Secp256k1Fp::from_bytes(&GENERATOR_Y).unwrap();

    let gen_x_big = fe_to_biguint(&gen_x_fp);
    let gen_y_big = fe_to_biguint(&gen_y_fp);

    // Decompose into 4x64-bit limbs
    let x_limbs = crate::spec::decompose_biguint_simple(&gen_x_big, 4, 64);
    let y_limbs = crate::spec::decompose_biguint_simple(&gen_y_big, 4, 64);

    println!("x decomposed into 4x64-bit limbs:");
    for (i, limb) in x_limbs.iter().enumerate() {
        let limb_big = crate::spec::fe_to_biguint_simple(limb);
        println!("  limb[{}]: {} ({} bits)", i, limb_big, limb_big.bits());
    }

    // Reconstruct x by summing: limb[0] + limb[1]*2^64 + limb[2]*2^128 + limb[3]*2^192
    let reconstructed_x = x_limbs
        .iter()
        .enumerate()
        .fold(BigUint::zero(), |acc, (i, limb)| {
            let limb_big = crate::spec::fe_to_biguint_simple(limb);
            acc + (limb_big << (64 * i))
        });

    let reconstructed_y = y_limbs
        .iter()
        .enumerate()
        .fold(BigUint::zero(), |acc, (i, limb)| {
            let limb_big = crate::spec::fe_to_biguint_simple(limb);
            acc + (limb_big << (64 * i))
        });

    assert_eq!(reconstructed_x, gen_x_big, "x reconstruction failed");
    assert_eq!(reconstructed_y, gen_y_big, "y reconstruction failed");
}

// ============================================================================
// Test: Mismatched Secp256k1 Keys (Should Fail)
// ============================================================================

#[test]
fn test_secp256k1_key_pairing_invalid() {
    use secp256k1::{PublicKey, Secp256k1, SecretKey};

    let secp = Secp256k1::new();

    let sk_bytes = [0x42; 32];
    let sk_secp = SecretKey::from_byte_array(sk_bytes).unwrap();

    let wrong_sk_bytes = [0x43; 32];
    let wrong_sk_secp = SecretKey::from_byte_array(wrong_sk_bytes).unwrap();
    let wrong_pk_secp = PublicKey::from_secret_key(&secp, &wrong_sk_secp);

    let wrong_pk_bytes = wrong_pk_secp.serialize_uncompressed();
    let pk_x_bytes: [u8; 32] = wrong_pk_bytes[1..33].try_into().unwrap();
    let pk_y_bytes: [u8; 32] = wrong_pk_bytes[33..65].try_into().unwrap();

    let mut sk_le = sk_bytes;
    sk_le.reverse();
    let sk = secp_fq_from_le(sk_le);
    let mut pk_x_le = pk_x_bytes;
    pk_x_le.reverse();
    let pk_x = secp_fp_from_le(pk_x_le);
    let mut pk_y_le = pk_y_bytes;
    pk_y_le.reverse();
    let pk_y = secp_fp_from_le(pk_y_le);

    let circuit = KeyPairingTestCircuit { sk, pk_x, pk_y };
    let prover = MockProver::run(18, &circuit, vec![]).expect("prover should run");

    assert!(
        prover.verify().is_err(),
        "Mismatched key pair should fail verification"
    );
}

// ============================================================================
// Helper Test: Verify Foreign Field Decomposition (4x64-bit)
// ============================================================================

#[test]
fn test_foreign_field_limb_decomposition() {
    use num_bigint::BigUint;

    let test_value_bytes = [
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99,
    ];

    let v = secp_fp_from_le(test_value_bytes);
    let v_biguint = halo2_base::utils::fe_to_biguint(&v);

    // Decompose into 4x64-bit limbs
    let limbs = crate::spec::decompose_biguint_simple(&v_biguint, 4, 64);

    // Verify each limb fits in 64 bits
    for (i, limb) in limbs.iter().enumerate() {
        let limb_big = crate::spec::fe_to_biguint_simple(limb);
        let max_64_bit = BigUint::from(1u64) << 64;
        assert!(
            limb_big < max_64_bit,
            "Limb {} exceeds 64 bits: has {} bits",
            i,
            limb_big.bits()
        );
    }

    // Verify reconstruction
    let reconstructed = limbs
        .iter()
        .enumerate()
        .fold(BigUint::zero(), |acc, (i, limb)| {
            let limb_big = crate::spec::fe_to_biguint_simple(limb);
            acc + (limb_big << (64 * i))
        });
    assert_eq!(
        reconstructed, v_biguint,
        "Reconstruction should match original value"
    );
}

// ============================================================================
// Test: Secp256k1 SK to Pallas Base Conversion
// ============================================================================
#[test]
fn test_secp256k1_pk_to_pallas_crt_conversion() {
    use halo2_base::utils::fe_to_biguint;
    use num_bigint::BigUint;
    use secp256k1::{PublicKey, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let sk_bytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ];
    let sk = SecretKey::from_byte_array(sk_bytes).expect("valid secret key");
    let pk = PublicKey::from_secret_key(&secp, &sk);

    let pk_bytes = pk.serialize_uncompressed();
    let pk_x_bytes: [u8; 32] = pk_bytes[1..33].try_into().unwrap();
    let pk_y_bytes: [u8; 32] = pk_bytes[33..65].try_into().unwrap();

    let pk_x_fp = secp_fp_from_le(pk_x_bytes);
    let pk_y_fp = secp_fp_from_le(pk_y_bytes);
    let pk_x_big = fe_to_biguint(&pk_x_fp);
    let pk_y_big = fe_to_biguint(&pk_y_fp);

    // Decompose into 4x64-bit limbs
    let x_limbs = crate::spec::decompose_biguint_simple(&pk_x_big, 4, 64);
    let y_limbs = crate::spec::decompose_biguint_simple(&pk_y_big, 4, 64);

    // Reconstruct using limb bases
    let base_64 = crate::spec::biguint_to_fe_simple(&(BigUint::from(1u64) << 64));
    let base_128 = crate::spec::biguint_to_fe_simple(&(BigUint::from(1u64) << 128));
    let base_192 = crate::spec::biguint_to_fe_simple(&(BigUint::from(1u64) << 192));

    let pk_x_pallas =
        x_limbs[0] + x_limbs[1] * base_64 + x_limbs[2] * base_128 + x_limbs[3] * base_192;
    let pk_y_pallas =
        y_limbs[0] + y_limbs[1] * base_64 + y_limbs[2] * base_128 + y_limbs[3] * base_192;

    let pallas_modulus = crate::spec::fe_to_biguint_simple(&(-pallas::Base::ONE)) + 1u64;
    let pk_x_expected = &pk_x_big % &pallas_modulus;
    let pk_y_expected = &pk_y_big % &pallas_modulus;

    let x_reconstructed_big = crate::spec::fe_to_biguint_simple(&pk_x_pallas);
    let y_reconstructed_big = crate::spec::fe_to_biguint_simple(&pk_y_pallas);

    assert_eq!(x_reconstructed_big, pk_x_expected);
    assert_eq!(y_reconstructed_big, pk_y_expected);
}

#[test]
fn test_secp256k1_sk_to_pallas_base_conversion() {
    use halo2_base::utils::fe_to_biguint;
    use num_bigint::BigUint;

    let sk_bytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ];

    let sk_fq = secp_fq_from_le(sk_bytes);
    let sk_big = fe_to_biguint(&sk_fq);

    // Decompose into 4x64-bit limbs
    let limbs = crate::spec::decompose_biguint_simple(&sk_big, 4, 64);

    for (i, limb) in limbs.iter().enumerate() {
        let limb_big = crate::spec::fe_to_biguint_simple(limb);
        let max_64_bit = BigUint::from(1u64) << 64;
        assert!(limb_big < max_64_bit, "Limb {} exceeds 64 bits", i);
    }

    // Reconstruct
    let base_64 = crate::spec::biguint_to_fe_simple(&(BigUint::from(1u64) << 64));
    let base_128 = crate::spec::biguint_to_fe_simple(&(BigUint::from(1u64) << 128));
    let base_192 = crate::spec::biguint_to_fe_simple(&(BigUint::from(1u64) << 192));

    let sk_pallas = limbs[0] + limbs[1] * base_64 + limbs[2] * base_128 + limbs[3] * base_192;
    let sk_pallas_direct = pallas::Base::from_repr(sk_bytes).unwrap_or(pallas::Base::zero());

    let reconstructed_big = crate::spec::fe_to_biguint_simple(&sk_pallas);
    let direct_big = crate::spec::fe_to_biguint_simple(&sk_pallas_direct);
    let pallas_modulus = crate::spec::fe_to_biguint_simple(&(-pallas::Base::ONE)) + 1u64;

    assert_eq!(
        reconstructed_big % pallas_modulus.clone(),
        direct_big % pallas_modulus,
    );
}

#[test]
fn test_non_paired_keys_detection() {
    use secp256k1::{PublicKey, Secp256k1, SecretKey};

    let secp = Secp256k1::new();

    let sk1_bytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ];
    let sk1 = SecretKey::from_byte_array(sk1_bytes).expect("valid sk1");
    let _pk1 = PublicKey::from_secret_key(&secp, &sk1);

    let sk2_bytes = [0x42; 32];
    let sk2 = SecretKey::from_byte_array(sk2_bytes).expect("valid sk2");
    let pk2 = PublicKey::from_secret_key(&secp, &sk2);

    let pk2_bytes = pk2.serialize_uncompressed();
    let _pk2_x_bytes: [u8; 32] = pk2_bytes[1..33].try_into().unwrap();
    let _pk2_y_bytes: [u8; 32] = pk2_bytes[33..65].try_into().unwrap();

    let computed_pk_from_sk1 = PublicKey::from_secret_key(&secp, &sk1);
    assert_ne!(computed_pk_from_sk1, pk2, "sk1 should NOT pair with pk2");
}

#[test]
fn test_eth_key_pairing_with_crt() {
    use halo2_base::utils::fe_to_biguint;
    use num_bigint::BigUint;
    use secp256k1::{PublicKey, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let sk = SecretKey::from_byte_array([
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ])
    .expect("valid secret key");
    let pk = PublicKey::from_secret_key(&secp, &sk);

    let pk_bytes = pk.serialize_uncompressed();
    let pk_x_bytes: [u8; 32] = pk_bytes[1..33].try_into().unwrap();
    let pk_y_bytes: [u8; 32] = pk_bytes[33..65].try_into().unwrap();

    let sk_fq = secp_fq_from_le(sk.secret_bytes());
    let pk_x_fp = secp_fp_from_le(pk_x_bytes);
    let pk_y_fp = secp_fp_from_le(pk_y_bytes);

    // Verify CRT decomposition with 4x64-bit limbs
    let sk_limbs = crate::spec::decompose_biguint_simple(&fe_to_biguint(&sk_fq), 4, 64);
    for (i, limb) in sk_limbs.iter().enumerate() {
        let limb_big = crate::spec::fe_to_biguint_simple(limb);
        assert!(limb_big.bits() <= 64, "Limb {} exceeds 64 bits", i);
    }

    // Verify reconstruction
    let reconstructed_sk =
        sk_limbs
            .iter()
            .enumerate()
            .fold(num_bigint::BigUint::zero(), |acc, (i, limb)| {
                let limb_big = crate::spec::fe_to_biguint_simple(limb);
                acc + (limb_big << (64 * i))
            });
    assert_eq!(reconstructed_sk, fe_to_biguint(&sk_fq));

    // Verify curve equation for pk
    let p = BigUint::parse_bytes(
        b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F",
        16,
    )
    .unwrap();
    let x = BigUint::from_bytes_be(&pk_x_bytes);
    let y = BigUint::from_bytes_be(&pk_y_bytes);
    let y_squared = (&y * &y) % &p;
    let x_cubed = (&x * &x * &x) % &p;
    let rhs = (x_cubed + BigUint::from(7u32)) % &p;
    assert_eq!(y_squared, rhs, "Public key must be on curve");
}

#[test]
fn test_document_foreign_field_flow() {
    println!("\n=== Foreign Field Arithmetic Flow (4x64-bit) ===\n");

    println!("1. Input: secp256k1 secret key (256 bits)");
    let sk_bytes = [0x42; 32];

    println!("\n2. Convert to secp256k1::Fq field element");
    let _sk_fq = secp_fq_from_le(sk_bytes);

    println!("\n3. In circuit: Load as CRT integer");
    println!("   - Decompose 256 bits -> 4 limbs x 64 bits");
    println!("   - Each limb stored as pallas::Base (fits in 254-bit field)");

    println!("\n4. Range check each limb:");
    println!("   - Each 64-bit limb -> 7 chunks x 10 bits (K=10 lookup)");

    println!("\n5. Custom gates enforce arithmetic:");
    println!("   - foreign_mul: x*y = z (mod q) with quotient witness");
    println!("   - normalize: reduce limbs to canonical form");

    println!("\n6. Montgomery ladder scalar multiplication");
    println!("   - 256 iterations of add + double + select");
    println!("   - All constrained via custom gates");

    println!("\n=== End Flow ===\n");
}
