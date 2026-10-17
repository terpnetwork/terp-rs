
 

// use ff::PrimeField;
// use group::cofactor::CofactorCurveAffine;
// use group::Group;
// use group::GroupEncoding;
// use halo2_gadgets::ecc::chip::find_zs_and_us;
// use jubjub::Fq;
// use jubjub::Fr;
// use jubjub::SubgroupPoint;
// use jubjub::{AffinePoint, ExtendedPoint};
// use pasta_curves::arithmetic::CurveAffine;
// use pasta_curves::pallas;
// use pasta_curves::Fp;
// use subtle::Choice;

// use crate::constants::fixed_bases::NUM_WINDOWS;

// /// (u,v) coordinates
// pub const GENERATOR: ([u8; 32], [u8; 32]) = (
//     [
//         55, 67, 102, 147, 119, 59, 206, 59, 78, 116, 3, 175, 65, 218, 216, 209, 180, 4, 128, 213,
//         106, 130, 246, 127, 252, 28, 0, 15, 250, 244, 0, 104,
//     ],
//     [
//         139, 106, 11, 56, 185, 250, 174, 60, 59, 128, 59, 71, 176, 241, 70, 173, 80, 171, 34, 30,
//         110, 42, 251, 230, 219, 222, 69, 203, 169, 211, 129, 109,
//     ],
// );

// // ... existing imports ...
// /// The byte-encoding of the basepoint for `BindingSig`.
// // Extracted ad-hoc from librustzcash
// // XXX add tests for this value.
// pub const BINDINGSIG_BASEPOINT_BYTES: [u8; 32] = [
//     139, 106, 11, 56, 185, 250, 174, 60, 59, 128, 59, 71, 176, 241, 70, 173, 80, 171, 34, 30, 110,
//     42, 251, 230, 219, 222, 69, 203, 169, 211, 129, 237,
// ];

// #[test]
// pub fn test_conversion() {
//     use jubjub::{AffinePoint, ExtendedPoint, Fr, SubgroupPoint};
//     use rand::thread_rng;
//     use redjubjub::*;
//     // Get the **redjubjub** generator point in the prime‑order subgroup
//     // (the base point used for signing keys)
//     let jj_base_point = ExtendedPoint::from_bytes(&BINDINGSIG_BASEPOINT_BYTES).unwrap();
//     let gen_affine = AffinePoint::from(jj_base_point);

//     println!(
//         "BINDINGSIG_BASEPOINT_BYTES: {:?}",
//         BINDINGSIG_BASEPOINT_BYTES
//     );

//     // The coordinates `u` and `v` are already `pasta_curves::Fp` values.
//     let u: Fq = gen_affine.get_u();
//     let v = gen_affine.get_v();

//     // Verify that they are canonical field elements by round‑tripping
//     // through `to_repr` / `from_repr`.
//     assert_eq!(Fp::from_repr(u.to_repr()).is_some().unwrap_u8(), 1);
//     assert_eq!(Fp::from_repr(v.to_repr()).is_some().unwrap_u8(), 1);

//     // Print generator coordinates for reference (optional)
//     println!("Base point u: {:?}", u);
//     println!("Base point v: {:?}", v);

//     println!("jub_u_affine: {:?}", u);
//     println!("jub_v_affine: {:?}", v);

//     // Generate a secret key and sign a message
//     let sk = SigningKey::<Binding>::new(thread_rng());
//     let msg = b"Its rainin' mah fuckas!";
//     let sig = sk.sign(thread_rng(), msg);

//     // Get the verification key from the secret key
//     let vk = VerificationKey::from(&sk);
//     let vk_bytes: [u8; 32] = vk.into(); // Compressed public key bytes

//     // ---- Fixed part: obtain the scalar that `sk` wraps ----
//     let sk_scalar: Fr = Fr::from_repr(sk.into()).unwrap(); // extracts the secret scalar
//     let sk_fp: Fp = Fp::from_repr(sk.into()).unwrap(); // extracts the secret scalar

//     // impossible to derive the jubjub generator point to a field-point for the pallas curve
//     // (jubjub_G -> pallas::Fp(jubjub_G))
//     // needed for
//     // ---------------------------------------------------------
//     assert_eq!(
//         Fp::from_repr(jj_base_point.to_bytes())
//             .is_some()
//             .unwrap_u8(),
//         0
//     );

//     // Manually compute the public key: pubkey = [sk_scalar] * base_point
//     let manual_pubkey_point = jj_base_point * sk_scalar; // Scalar multiplication
//     let manual_pubkey_affine = AffinePoint::from(manual_pubkey_point);
//     let manual_vk_bytes = manual_pubkey_affine.to_bytes(); // Compressed bytes

//     // Assert that the manually computed public key matches the one from VerificationKey
//     assert_eq!(
//         manual_vk_bytes, vk_bytes,
//         "Public key derivation does not match"
//     );
//     // Verify the signature for additional correctness

//     let sig: Signature<Binding> = sig.into();
//     assert!(
//         VerificationKey::try_from(manual_vk_bytes)
//             .and_then(|pk| pk.verify(msg, &sig))
//             .is_ok(),
//         "Signature verification failed"
//     );
// }
