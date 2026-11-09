//! Deterministic WAVS proof-of-ownership fixture for multi-test instantiate.
//!
//! Instantiate runs `WavsProofOfOwnership::verify` and the BLS12-381 PoP
//! pairing check, so the mock operator set has to carry real keys and
//! signatures. Scalars are fixed so the tests stay reproducible.

use ark_bls12_381::{Fr, G1Affine, G1Projective, G2Affine};
use ark_ec::AffineRepr;
use ark_ff::PrimeField;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use cosmwasm_std::testing::MockApi;
use cosmwasm_std::{Api, BLS12_381_G2_GENERATOR as G2, HashFunction};
use cw_headstash::wavs::{WavsAuthMetadata, WavsOpAuth, WavsProofOfOwnership};

/// Valid `count`-operator PoP set with a 2/3 + 1 threshold.
pub fn valid_wavs_proof(count: usize) -> WavsProofOfOwnership {
    assert!(count >= 1, "count must be >= 1");
    let api = MockApi::default();
    let mut poos = Vec::with_capacity(count);
    let mut agg = G1Projective::default();
    for i in 0..count {
        let sk = Fr::from_be_bytes_mod_order(&[0x42u8.wrapping_add(i as u8); 32]);
        let pk: G1Affine = (G1Affine::generator() * sk).into();
        let mut pk_bytes = vec![];
        pk.serialize_compressed(&mut pk_bytes).unwrap();
        let pop_hash = api
            .bls12_381_hash_to_g2(HashFunction::Sha256, &pk_bytes, &G2)
            .unwrap();
        let h_point = G2Affine::deserialize_compressed(&pop_hash[..]).unwrap();
        let sig: G2Affine = (h_point * sk).into();
        let mut sig_bytes = vec![];
        sig.serialize_compressed(&mut sig_bytes).unwrap();
        poos.push(WavsOpAuth {
            key: hex::encode(&pk_bytes),
            poo: hex::encode(&sig_bytes),
        });
        agg += pk;
    }
    let agg: G1Affine = agg.into();
    let mut agg_bytes = vec![];
    agg.serialize_compressed(&mut agg_bytes).unwrap();
    WavsProofOfOwnership {
        poos,
        msg: WavsAuthMetadata {
            aggregate_key: hex::encode(agg_bytes),
            threshold: count * 2 / 3 + 1,
            total_operators: count,
            nonce: 0,
        },
    }
}
