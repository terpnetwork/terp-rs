use ff::PrimeField;
use pasta_curves::arithmetic::CurveExt;
use pasta_curves::group::Curve;
use pasta_curves::pallas::{Point, Scalar};
use pasta_curves::Fq;
use std::u64;

fn string_to_fq(str_bytes: &str) -> Fq {
    let mut secret_bytes = [0u8; 8];
    let bytes: &[u8] = str_bytes.as_bytes();
    secret_bytes[..bytes.len()].copy_from_slice(bytes);
    Scalar::from(u64::from_le_bytes(secret_bytes))
}

fn create_p_and_q(p: &str, q: &str) -> Vec<u8> {
    let mut concatenated = Vec::with_capacity(p.len() + q.len());
    concatenated.extend_from_slice(p.as_bytes());
    concatenated.extend_from_slice(q.as_bytes());
    concatenated
}
//
// In the Pedersen commitment we take two large prime numbers (p & q) to create generator value g which is of the order of q
// and a subgroup of Z_p*. Then `s` becomes a secret from 0 to Z_q, and we calculate:
// h = g^s (mod p)
// The sender now creates a commitment for a message (m) with a random number (r)
// c = g^m * h^r (mod p)
// sources:
// - <https://www.youtube.com/watch?v=J9SOk9dIOCk>
// - https://asecuritysite.com/encryption/ped?v1=10&v2=100

fn main() {
    let secret1 = string_to_fq("secret");
    let secret2 = string_to_fq("garden");

    // Get the default Pedersen generators (G for value, H for blinding)
    let hasher = Point::hash_to_curve("headstash_blinding_generator");

    let pq = create_p_and_q("trailed", "dissolving");
    let g = hasher(&pq);
    let h = hasher(b"h");

    // A random blinding factors (for hiding the value)
    let r1 = Scalar::from_u128(678673809578324923489057u128);
    let r2 = Scalar::from_u128(4364326278543786489786542564u128);

    // Compute the commitment: C = value * G + blinding * H
    let commitment1 = (g * secret1 + &(h * r1)).to_affine();
    let commitment2 = (g * secret2 + &(h * r2)).to_affine();

    println!("Secret value 1: {:?}", secret1);
    println!("Secret value 2: {:?}", secret2);
    println!("Blinding factor 1: {:?}", r1);
    println!("Blinding factor 2: {:?}", r2);
    println!("Commitment 1: {:?}", commitment1);
    println!("Commitment 2: {:?}", commitment2);

    // check we can add both values (homomorphic-encryption)
    let combined_commitment = commitment2 + commitment1;

    // add secrets together
    let expected_value = &secret1 + &secret2;
    let expected_blinding = &r1 + &r2;
    let expected_commitment = g * expected_value + h * expected_blinding;
    let expected_incorrect_commitment = g * expected_value + h * r1;
    assert_ne!(combined_commitment, expected_incorrect_commitment);
    let expected_incorrect_commitment = g * expected_value + h * (r1.square());
    assert_ne!(combined_commitment, expected_incorrect_commitment);

    println!("Combined commitment (C1 + C2): {:?}", combined_commitment);
    println!("Expected value (v1 + v2): {:?}", expected_value);
    println!("Expected blinding (r1 + r2): {:?}", expected_blinding);

    assert_eq!(combined_commitment, expected_commitment);

    // opening and verifying original commitments
    let recomputed1 = g * secret1 + h * r1;
    let recomputed2 = g * secret2 + h * r2;
    assert_eq!(commitment1, recomputed1.into());
    assert_eq!(commitment2, recomputed2.into());
}
