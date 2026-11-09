// //! The suite consists of two tests; one for each type of signature. One of them also do printings of the values,
// //! which can be useful to you when comparing different implementations.
// //! Their setup is shared, `mod helpers` contains barely not refactored code, which is still instrumental to the tests.

// use helpers::{PlumeVersion, gen_test_scalar_sk};
// use k256::FieldBytes;
// use k256::{NonZeroScalar, ProjectivePoint, elliptic_curve::sec1::ToEncodedPoint};
// use plume_rustcrypto::{AffinePoint, PlumeSignature, PlumeSignatureV1Fields};

// use crate::helpers::test_blake_gen_signals;

// const G: ProjectivePoint = ProjectivePoint::GENERATOR;
// const M_HEADSTASH: &str = "DE4BAA02C4855872BBA5464749157D06151ED215C6FD39A07454344DE8D9A2BF";

// // `test_gen_signals` provides fixed key nullifier, secret key, and the random value for testing
// // Normally a secure enclave would generate these values, and output to a wallet implementation
// // `gen_test_scalar_sk()` provides the signer's secret key. It is only accessed within the secure enclave.
// // The user's public key goes to the `pk` field as $g^sk$.

// // Both tests finish with the signals verification, normally this would happen in ZK with only the nullifier public, which would have a zk verifier instead
// // The wallet should probably run this prior to snarkify-ing as a sanity check
// // `M` and nullifier should be public, so we can verify that they are correct

// #[test]
// fn my_plume_v2_test() {
//     let test_data = test_blake_gen_signals(M_HEADSTASH.as_bytes(), PlumeVersion::V2);
//     assert!(
//         PlumeSignature {
//             message: M_HEADSTASH.as_bytes().into(),
//             pk: (G * gen_test_scalar_sk()).into(),
//             nullifier: test_data.1.into(),
//             c: NonZeroScalar::from_repr(<[u8; 32] as Into<FieldBytes>>::into(
//                 *test_data.2.as_bytes()
//             ))
//             .unwrap(),
//             s: NonZeroScalar::new(test_data.3).unwrap(),
//             v1specific: None
//         }
//         .verify_blake()
//     );
// }

// mod helpers {
//     /* Feels like this one could/should be replaced with static/constant values. Preserved for historical reasons.
//     For the same reasons calls for internal `fn` are commented and replaced by "one-liners" adapted from current implementation. */
//     use super::*;
//     use hex_literal::hex;
//     use k256::{
//         FieldBytes, Scalar, Secp256k1,
//         elliptic_curve::{
//             PrimeField,
//             hash2curve::{ExpandMsgXmd, GroupDigest},
//         },
//         sha2::{Digest, Sha256, digest::Output},
//     };

//     use blake3::Hash;
//     #[derive(Debug)]
//     pub enum PlumeVersion {
//         V1,
//         V2,
//     }

//     // Generates a deterministic secret key for deterministic testing. Should be replaced by random oracle in production deployments.
//     pub fn gen_test_scalar_sk() -> Scalar {
//         Scalar::from_repr(
//             hex!("cf9fcc0acf2a93917e16e63a9aa28c272b68be6fd8731057c3f1a3c00ef8aae7").into(),
//         )
//         .unwrap()
//     }

//     // Generates a deterministic r for deterministic testing. Should be replaced by random oracle in production deployments.
//     fn gen_test_scalar_r() -> Scalar {
//         Scalar::from_repr(
//             hex!("5b5a890438716d9778d07d54b8529a60a82a046ee7ed316db180788c36726740").into(),
//         )
//         .unwrap()
//     }

//     // Calls the hash to curve function for secp256k1, and returns the result as a ProjectivePoint
//     pub fn hash_to_secp(s: &[u8]) -> ProjectivePoint {
//         let pt: ProjectivePoint = Secp256k1::hash_from_bytes::<ExpandMsgXmd<Sha256>>(
//             &[s],
//             //b"CURVE_XMD:SHA-256_SSWU_RO_"
//             &[plume_rustcrypto::DST],
//         )
//         .unwrap();
//         pt
//     }

//     // Modified blake_hash_to_secp function using BLAKE3, returns the result as a ProjectivePoint
//     pub fn blake_hash_to_secp(s: &[u8]) -> ProjectivePoint {
//         let pt: ProjectivePoint =
//             Secp256k1::hash_from_bytes::<Blake3Xmd>(&[s], &[DST_BLAKE3]).unwrap();
//         pt
//     }

//     use k256::ProjectivePoint;
//     use plume_rustcrypto::blake3xmd::{Blake3Xmd, DST_BLAKE3};
//     // These generate test signals as if it were passed from a secure enclave to wallet. Note that leaking these signals would leak pk, but not sk.
//     // Outputs these 6 signals, in this order
//     // g^sk																(private)
//     // hash[m, pk]^sk 													public nullifier
//     // c = hash2(g, pk, hash[m, pk], hash[m, pk]^sk, gr, hash[m, pk]^r)	(public or private)
//     // r + sk * c														(public or private)
//     // g^r																(private, optional)
//     // hash[m, pk]^r													(private, optional)
//     pub fn test_blake_gen_signals(
//         m: &[u8],
//         version: PlumeVersion,
//     ) -> (
//         ProjectivePoint,
//         ProjectivePoint,
//         Hash,
//         Scalar,
//         Option<ProjectivePoint>,
//         Option<ProjectivePoint>,
//     ) {
//         // The base point or generator of the curve.
//         let g = ProjectivePoint::GENERATOR;

//         // The signer's secret key. It is only accessed within the secure enclave.
//         let sk = gen_test_scalar_sk();

//         // A random value r. It is only accessed within the secure enclave.
//         let r = gen_test_scalar_r();

//         // The user's public key: g^sk.
//         let pk = &g * &sk;

//         // The generator exponentiated by r: g^r.
//         let g_r = &g * &r;

//         // hash[m, pk]
//         let hash_m_pk =
//             // zk_nullifier::hash_to_curve(m, &pk)
//             Secp256k1::hash_from_bytes::<Blake3Xmd>(
//                 &[[
//                     m,
//                     // &encode_pt(pk)
//                     &pk.to_encoded_point(true).to_bytes().to_vec()
//                 ].concat().as_slice()],
//                 //b"CURVE_XMD:BLAKE-3_SSWU_RO_",
//                 &[DST_BLAKE3],
//             )
//             .unwrap();

//         println!(
//             "h.x: {:?}",
//             hex::encode(hash_m_pk.to_affine().to_encoded_point(false).x().unwrap())
//         );
//         println!(
//             "h.y: {:?}",
//             hex::encode(hash_m_pk.to_affine().to_encoded_point(false).y().unwrap())
//         );

//         // hash[m, pk]^r
//         let hash_m_pk_pow_r = &hash_m_pk * &r;
//         println!(
//             "hash_m_pk_pow_r.x: {:?}",
//             hex::encode(
//                 hash_m_pk_pow_r
//                     .to_affine()
//                     .to_encoded_point(false)
//                     .x()
//                     .unwrap()
//             )
//         );
//         println!(
//             "hash_m_pk_pow_r.y: {:?}",
//             hex::encode(
//                 hash_m_pk_pow_r
//                     .to_affine()
//                     .to_encoded_point(false)
//                     .y()
//                     .unwrap()
//             )
//         );

//         // The public nullifier: hash[m, pk]^sk.
//         let nullifier = &hash_m_pk * &sk;

//         // The Fiat-Shamir type step.
//         let c = match version {
//             PlumeVersion::V1 => blake3::hash(
//                 vec![&g, &pk, &hash_m_pk, &nullifier, &g_r, &hash_m_pk_pow_r]
//                     .into_iter()
//                     .map(|x| x.to_encoded_point(true).to_bytes().to_vec())
//                     .collect::<Vec<_>>()
//                     .concat()
//                     .as_slice(),
//             ),
//             PlumeVersion::V2 => {
//                 dbg!("entering `blake3::hash` for `V2`");
//                 let result = blake3::hash(
//                     vec![&nullifier, &g_r, &hash_m_pk_pow_r]
//                         .into_iter()
//                         .map(|x| x.to_encoded_point(true).to_bytes().to_vec())
//                         .collect::<Vec<_>>()
//                         .concat()
//                         .as_slice(),
//                 );
//                 dbg!("finished `blake3::hash` for `V2`");
//                 result
//             }
//         };
//         dbg!(&c, version);

//         let c_scalar =
//             Scalar::from_repr(<[u8; 32] as Into<FieldBytes>>::into(*c.as_bytes())).unwrap();
//         // This value is part of the discrete log equivalence (DLEQ) proof.
//         let r_sk_c = r + sk * c_scalar;

//         // Return the signature.
//         (pk, nullifier, c, r_sk_c, Some(g_r), Some(hash_m_pk_pow_r))
//     }
//     /* Yes, testing the tests isn't a conventional things.
//     This should be straightened if `helpers` will be refactored. */
//     #[cfg(test)]
//     mod tests {
//         use super::*;
//         use k256::elliptic_curve::sec1::ToEncodedPoint;
//         // Test the hash-to-curve algorithm
//         #[test]
//         fn test_hash_to_curve_sha256() {
//             let h = hash_to_secp(b"abc");
//             assert_eq!(
//                 hex::encode(h.to_affine().to_encoded_point(false).x().unwrap()),
//                 "3377e01eab42db296b512293120c6cee72b6ecf9f9205760bd9ff11fb3cb2c4b"
//             );
//             assert_eq!(
//                 hex::encode(h.to_affine().to_encoded_point(false).y().unwrap()),
//                 "7f95890f33efebd1044d382a01b1bee0900fb6116f94688d487c6c7b9c8371f6"
//             );
//         }
//         #[test]
//         fn test_hash_to_curve_blake3() {
//             let h = blake_hash_to_secp(b"abc");
//             assert_eq!(
//                 hex::encode(h.to_affine().to_encoded_point(false).x().unwrap()),
//                 "345571ae56fcc327b5a2e38a3581c6f34f1019843a53039ddaafbc21327044ce"
//             );
//             assert_eq!(
//                 hex::encode(h.to_affine().to_encoded_point(false).y().unwrap()),
//                 "9d82e199ff56011f881aa8573eda5cc6745c70a1825a6f5e799dd40af6c82e67"
//             );
//         }
//     }
// }

fn main() {}
