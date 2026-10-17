//! Genesis Sinsemilla leaf hash test circuit.
//!
//! This module provides a spec-compliant test circuit for verifying the genesis
//! sinsemilla merkle tree leaf hash computation with proper 8-piece message
//! decomposition and canonicity constraints.
//!
//! ## Message Decomposition (510 bits total):
//! - `a`: epk.x[0..250] (250 bits)
//! - `b`: epk.x[250..255] || epk.y[0] || nd[0..4] (10 bits)
//! - `c`: nd[4..254] (250 bits)
//! - `d`: nd[254] || v[0..9] (10 bits)
//! - `e`: v[9..59] (50 bits)
//! - `f`: v[59..64] || fdi[0..5] (10 bits)
//! - `g`: fdi[5..55] (50 bits)
//! - `h`: fdi[55..64] || padding (10 bits)
//!
//! ## Test Coverage
//!
//! Tests use `MerkleTestDataBuilder` from suite.rs to generate test data that can be
//! verified against the circuit constraints.

use pasta_curves::pallas;
use zk_headstash::{
    OrchardCommitDomains, OrchardFixedBases, OrchardHashDomains,
    circuit::{
        Secp256k1Chip, Secp256k1Config, Secp256k1Fp, assign_free_advice,
        headstash_merkle_tree::{LeafHashChip, LeafHashConfig, gadgets},
    },
    value::NoteValue,
};

use halo2_gadgets::{
    ecc::{
        CircuitVersion,
        chip::{EccChip, EccConfig},
    },
    sinsemilla::chip::{SinsemillaChip, SinsemillaConfig},
    utilities::lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig},
};
use halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner, Value},
    plonk::{Circuit, ConstraintSystem, Error},
};

/// Configuration type for the LeafHashTestCircuit.
pub type LeafHashTestConfig = (
    LeafHashConfig,
    SinsemillaConfig<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
    EccConfig<OrchardFixedBases>,
    Secp256k1Config,
);

/// Spec-compliant test circuit for genesis leaf hash computation.
///
/// This circuit tests the 8-piece message decomposition and canonicity constraints
/// for the genesis sinsemilla merkle tree leaf hash, matching the exact specification
/// used in the production circuit.
///
/// ## Usage
///
/// ```ignore
/// use zk_test_press::circuits::sinsemilla_hashdomain::LeafHashTestCircuit;
/// use halo2_proofs::{circuit::Value, dev::MockProver};
///
/// let circuit = LeafHashTestCircuit::new(
///     Value::known(epk_x),
///     Value::known(epk_y),
///     Value::known(nd),
///     Value::known(v),
///     Value::known(fdi),
/// );
///
/// let prover = MockProver::run(17, &circuit, vec![]).unwrap();
/// prover.verify().unwrap();
/// ```
#[derive(Default, Clone, Debug)]
pub struct LeafHashTestCircuit {
    /// Secp256k1 public key x-coordinate (as foreign field element with 4x64-bit CRT representation)
    pub epk_x: Value<Secp256k1Fp>,
    /// Secp256k1 public key y-coordinate (as foreign field element with 4x64-bit CRT representation)
    pub epk_y: Value<Secp256k1Fp>,
    /// Note denomination (blake3 hash with top bits cleared to fit in pallas field)
    pub nd: Value<pallas::Base>,
    /// Note value (64-bit, padded)
    pub v: Value<NoteValue>,
    /// Fixed denomination index (64-bit, padded)
    pub fdi: Value<pallas::Base>,
}

impl LeafHashTestCircuit {
    /// Create a new test circuit with the given values.
    pub fn new(
        epk_x: Value<Secp256k1Fp>,
        epk_y: Value<Secp256k1Fp>,
        nd: Value<pallas::Base>,
        v: Value<NoteValue>,
        fdi: Value<pallas::Base>,
    ) -> Self {
        Self {
            epk_x,
            epk_y,
            nd,
            v,
            fdi,
        }
    }
}

impl Circuit<pallas::Base> for LeafHashTestCircuit {
    type Config = LeafHashTestConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        let advices = [
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
        ];

        let constants = meta.fixed_column();
        meta.enable_constant(constants);

        for advice in advices.iter() {
            meta.enable_equality(*advice);
        }

        let table_idx = meta.lookup_table_column();
        let lookup = (
            table_idx,
            meta.lookup_table_column(),
            meta.lookup_table_column(),
        );
        let lagrange_coeffs = [
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
        ];

        let range_check = LookupRangeCheckConfig::configure(meta, advices[9], table_idx);

        let sinsemilla_config = SinsemillaChip::<
            OrchardHashDomains,
            OrchardCommitDomains,
            OrchardFixedBases,
        >::configure(
            meta,
            advices[..5].try_into().unwrap(),
            advices[6],
            lagrange_coeffs[0],
            lookup,
            range_check.clone(),
            false,
        );

        let leaf_hash_config = LeafHashChip::configure(meta, advices);

        let ecc_config =
            EccChip::<OrchardFixedBases>::configure(meta, advices, lagrange_coeffs, range_check);

        let secp_advices: [halo2_proofs::plonk::Column<halo2_proofs::plonk::Advice>; 9] = [
            advices[0], advices[1], advices[2], advices[3], advices[4], advices[5], advices[6],
            advices[7], advices[8],
        ];
        let secp256k1 = Secp256k1Config::configure(meta, secp_advices, range_check.clone());

        (leaf_hash_config, sinsemilla_config, ecc_config, secp256k1)
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut lo: impl Layouter<pallas::Base>,
    ) -> Result<(), Error> {
        let (leaf_hash_config, sinsemilla_config, ecc_config, secp256k1_config) = config;

        // Load the Sinsemilla chip lookup table
        SinsemillaChip::<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>::load(
            sinsemilla_config.clone(),
            &mut lo,
        )?;

        let sinsemilla_chip = SinsemillaChip::construct(sinsemilla_config);
        let ecc_chip = EccChip::construct(ecc_config, CircuitVersion::AnchoredBase);
        let lhc = LeafHashChip::construct(leaf_hash_config);
        let secp256k1 = Secp256k1Chip::construct(secp256k1_config);

        // Load secp256k1 public key coordinates as CRT integers (4x64-bit limbs)
        let epkx = secp256k1
            .fp
            .load_private(lo.namespace(|| "load pk.x"), self.epk_x)?;
        let epky = secp256k1
            .fp
            .load_private(lo.namespace(|| "load pk.y"), self.epk_y)?;

        // Witness note inputs
        let nd = assign_free_advice(
            lo.namespace(|| "witness nd"),
            lhc.config.advices[0],
            self.nd,
        )?;

        let v = assign_free_advice(lo.namespace(|| "witness v"), lhc.config.advices[0], self.v)?;

        let fdi = assign_free_advice(
            lo.namespace(|| "witness fdi"),
            lhc.config.advices[0],
            self.fdi,
        )?;

        // Compute leaf hash with proper 8-piece decomposition and canonicity constraints
        let _leaf = gadgets::hash_leaf(
            sinsemilla_chip,
            ecc_chip,
            lhc.clone(),
            lo,
            (epkx, epky),
            fdi,
            v,
            nd,
        )?;

        Ok(())
    }
}

// #[cfg(test)]
// mod tests {
//     use std::vec::Vec;

//     use zk_headstash::Note;

//     use crate::HeadstashSuite;

//     use super::*;

//     use ff::{Field, PrimeField};
//     use halo2_proofs::dev::MockProver;
//     use pasta_curves::Fp;
//     use rand::rngs::OsRng;

//     /// k parameter for the circuit (number of rows = 2^k)
//     const K: u32 = 17;

//     #[test]
//     fn test_leaf_hash_basic() {
//         let (_, _, esk, _) = Note::dummy(&mut OsRng, None);
//         let (epkx, epky) = esk.epk().xy();
//         let (epkx, epky) = (
//             Secp256k1Fp::from_bytes(&epkx).expect("valid Fp"),
//             Secp256k1Fp::from_bytes(&epky).expect("valid Fp"),
//         );

//         let circuit = LeafHashTestCircuit {
//             epk_x: Value::known(epkx),
//             epk_y: Value::known(epky),
//             nd: Value::known(pallas::Base::from(42u64)),
//             v: Value::known(NoteValue::one()),
//             fdi: Value::known(pallas::Base::from(100u64)),
//         };

//         let prover = MockProver::<pallas::Base>::run(K, &circuit, vec![]);
//         assert!(prover.is_ok(), "Prover creation should succeed");
//         assert!(prover.unwrap().verify().is_ok(), "Circuit should pass");
//     }

//     #[test]
//     fn test_leaf_hash_various_inputs() {
//         let two_pow_254 = pallas::Base::from_u128(1u128 << 127).square();

//         let (_, _, esk, _) = Note::dummy(&mut OsRng, None);
//         let (epkx, epky) = esk.epk().xy();
//         let (epkx, epky) = (
//             Secp256k1Fp::from_bytes(&epkx).expect("valid Fp"),
//             Secp256k1Fp::from_bytes(&epky).expect("valid Fp"),
//         );

//         let test_cases = vec![
//             (
//                 "minimal values",
//                 pallas::Base::one(),
//                 NoteValue::one(),
//                 pallas::Base::one(),
//             ),
//             (
//                 "max field values",
//                 -pallas::Base::one(),
//                 NoteValue::one(),
//                 -pallas::Base::one(),
//             ),
//             (
//                 "max u64 values",
//                 pallas::Base::from(u64::MAX),
//                 NoteValue::one(),
//                 pallas::Base::from(u64::MAX),
//             ),
//             (
//                 "254-bit boundary",
//                 two_pow_254 - pallas::Base::one(),
//                 NoteValue::one(),
//                 two_pow_254 - pallas::Base::one(),
//             ),
//             (
//                 "zero nd",
//                 pallas::Base::zero(),
//                 NoteValue::one(),
//                 pallas::Base::from(100u64),
//             ),
//             (
//                 "zero fdi",
//                 pallas::Base::from(200u64),
//                 NoteValue::one(),
//                 pallas::Base::zero(),
//             ),
//             (
//                 "power of 2",
//                 pallas::Base::from(1u64 << 30),
//                 NoteValue::one(),
//                 pallas::Base::from(1u64 << 32),
//             ),
//             (
//                 "alternating bits",
//                 pallas::Base::from(0xAAAAAAAAAAAAAAAAu64),
//                 NoteValue::one(),
//                 pallas::Base::from(0x5555555555555555u64),
//             ),
//         ];

//         for (name, nd, v, fdi) in test_cases.iter() {
//             std::println!("Running test case: {}", name);
//             let circuit = LeafHashTestCircuit {
//                 epk_x: Value::known(epkx),
//                 epk_y: Value::known(epky),
//                 nd: Value::known(*nd),
//                 v: Value::known(*v),
//                 fdi: Value::known(*fdi),
//             };

//             let prover = MockProver::<pallas::Base>::run(K, &circuit, vec![]);
//             assert!(
//                 prover.is_ok(),
//                 "Test case '{}' prover creation failed",
//                 name
//             );
//             assert_eq!(
//                 prover.unwrap().verify(),
//                 Ok(()),
//                 "Test case '{}' verification failed",
//                 name
//             );
//         }
//     }

//     #[test]
//     fn test_with_merkle_test_data_builder() {
//         // Use MerkleTestDataBuilder to generate test data
//         let suite = HeadstashSuite::new();

//         // Generate deterministic test leaf data
//         let addr = [42u8; 32];
//         let token = "uterp";
//         let value = 1_000_000u64;
//         let fdi_index = 0u64;

//         let leaf_data = suite.generate_leaf_data(&addr, token, value, fdi_index);

//         // Verify we can compute the leaf hash
//         let hash_result = suite.compute_leaf_from_data(&leaf_data);
//         assert!(hash_result.is_ok(), "Leaf hash computation should succeed");

//         let _leaf_hash = hash_result.unwrap();

//         // Note: Full circuit verification with MerkleTestDataBuilder requires
//         // converting the test data format to circuit witness format.
//         // The circuit uses Secp256k1Fp for epk, but MerkleTestDataBuilder
//         // computes the sum of 4x64-bit limbs as a single Fp value.
//         // This test verifies the test data generation works correctly.
//     }

//     #[test]
//     fn test_merkle_test_data_builder_integration() {
//         // Test that MerkleTestDataBuilder generates valid test data
//         let suite = HeadstashSuite::new();

//         // Generate test data for a small tree
//         let result = suite.generate_circuit_test_data(4, 0);
//         assert!(result.is_ok(), "Should generate test data successfully");

//         let test_data = result.unwrap();
//         assert!(test_data.tree_depth > 0, "Tree should have positive depth");
//         assert_eq!(
//             test_data.auth_path.leaf_index, 0,
//             "Should be leaf at index 0"
//         );

//         // Verify path leads to correct root
//         assert!(
//             suite.verify_merkle_path(&test_data.leaf_hash, &test_data.auth_path, &test_data.root),
//             "Generated path should be valid"
//         );
//     }

//     #[test]
//     fn test_leaf_hash_consistency() {
//         // Verify that leaf hashes computed by MerkleTestDataBuilder match circuit expectations
//         let suite = HeadstashSuite::new();

//         let leaves_data = suite.generate_test_leaves(4);

//         for (i, leaf_data) in leaves_data.iter().enumerate() {
//             let hash_result = suite.compute_leaf_from_data(leaf_data);
//             assert!(
//                 hash_result.is_ok(),
//                 "Leaf hash computation should succeed for leaf {}",
//                 i
//             );

//             let hash = hash_result.unwrap();
//             assert_ne!(
//                 hash,
//                 Fp::ZERO,
//                 "Leaf hash should be non-zero for leaf {}",
//                 i
//             );
//         }
//     }

//     #[test]
//     fn test_path_verification_for_all_leaves() {
//         // Test that paths are correctly computed for all leaf positions
//         let suite = HeadstashSuite::new();

//         let num_leaves = 8;
//         let leaves_data = suite.generate_test_leaves(num_leaves);

//         let leaf_hashes: Vec<Fp> = leaves_data
//             .iter()
//             .map(|d| suite.compute_leaf_from_data(d).unwrap())
//             .collect();

//         let tree = suite.generate_full_merkle_tree(leaf_hashes.clone());
//         let root = tree.root();

//         // Verify path for each leaf
//         for (i, hash) in leaf_hashes.iter().enumerate() {
//             let path = suite.compute_merkle_path(&tree, i);
//             assert!(
//                 suite.verify_merkle_path(hash, &path, &root),
//                 "Path verification should succeed for leaf at index {}",
//                 i
//             );
//         }
//     }

//     #[test]
//     fn test_invalid_path_rejected() {
//         // Test that invalid merkle paths are correctly rejected
//         let suite = HeadstashSuite::new();

//         let leaves_data = suite.generate_test_leaves(4);
//         let leaf_hashes: Vec<Fp> = leaves_data
//             .iter()
//             .map(|d| suite.compute_leaf_from_data(d).unwrap())
//             .collect();

//         let tree = suite.generate_full_merkle_tree(leaf_hashes.clone());
//         let root = tree.root();

//         // Get path for leaf 0
//         let path = suite.compute_merkle_path(&tree, 0);

//         // Try to verify with wrong leaf (leaf 1's hash)
//         let wrong_leaf = &leaf_hashes[1];
//         assert!(
//             !suite.verify_merkle_path(wrong_leaf, &path, &root),
//             "Path for leaf 0 should not verify with leaf 1's hash"
//         );

//         // Try to verify with wrong root
//         let wrong_root = Fp::from(999u64);
//         assert!(
//             !suite.verify_merkle_path(&leaf_hashes[0], &path, &wrong_root),
//             "Path should not verify with wrong root"
//         );
//     }

//     #[test]
//     fn test_deterministic_tree_generation() {
//         // Test that tree generation is deterministic for same inputs
//         let suite = HeadstashSuite::new();

//         let addr = [42u8; 32];
//         let token = "uterp";
//         let value = 1_000_000u64;
//         let fdi = 0u64;

//         let leaf1 = suite.generate_leaf_data(&addr, token, value, fdi);
//         let leaf2 = suite.generate_leaf_data(&addr, token, value, fdi);

//         let hash1 = suite.compute_leaf_from_data(&leaf1).unwrap();
//         let hash2 = suite.compute_leaf_from_data(&leaf2).unwrap();

//         assert_eq!(hash1, hash2, "Same inputs should produce same leaf hash");

//         let tree1 = suite.generate_full_merkle_tree(vec![hash1]);
//         let tree2 = suite.generate_full_merkle_tree(vec![hash2]);

//         assert_eq!(
//             tree1.root(),
//             tree2.root(),
//             "Same leaves should produce same root"
//         );
//     }

//     #[test]
//     fn test_position_encoding_correctness() {
//         // Test that position encoding correctly identifies left/right children
//         let suite = HeadstashSuite::new();

//         let leaves: Vec<Fp> = (0..8).map(|i| Fp::from(i as u64)).collect();
//         let tree = suite.generate_full_merkle_tree(leaves);

//         // Leaf 0 should be left child at all levels (position = 0)
//         let path0 = suite.compute_merkle_path(&tree, 0);
//         assert!(
//             !path0.position_bits[0],
//             "Leaf 0 should be left child at level 0"
//         );
//         assert!(
//             !path0.position_bits[1],
//             "Leaf 0 should be left child at level 1"
//         );
//         assert!(
//             !path0.position_bits[2],
//             "Leaf 0 should be left child at level 2"
//         );

//         // Leaf 7 should be right child at all levels (position = 7 = 0b111)
//         let path7 = suite.compute_merkle_path(&tree, 7);
//         assert!(
//             path7.position_bits[0],
//             "Leaf 7 should be right child at level 0"
//         );
//         assert!(
//             path7.position_bits[1],
//             "Leaf 7 should be right child at level 1"
//         );
//         assert!(
//             path7.position_bits[2],
//             "Leaf 7 should be right child at level 2"
//         );

//         // Leaf 4 should be: left at level 0, left at level 1, right at level 2 (position = 4 = 0b100)
//         let path4 = suite.compute_merkle_path(&tree, 4);
//         assert!(
//             !path4.position_bits[0],
//             "Leaf 4 should be left child at level 0"
//         );
//         assert!(
//             !path4.position_bits[1],
//             "Leaf 4 should be left child at level 1"
//         );
//         assert!(
//             path4.position_bits[2],
//             "Leaf 4 should be right child at level 2"
//         );
//     }

//     #[test]
//     #[cfg(feature = "multicore")]
//     fn test_full_tree_root_matches_existing_impl() {
//         // Verify that generate_full_merkle_tree produces same root as tree_root_from_leaves
//         let suite = HeadstashSuite::new();

//         for num_leaves in [2, 4, 8, 16] {
//             use zk_headstash::suite::suite::HeadstashSinsemillaTree;

//             let leaves: Vec<Fp> = (0..num_leaves).map(|i| Fp::from(i as u64)).collect();

//             let existing_root = suite.tree_root_from_leaves(leaves.clone())[0];
//             let full_tree = suite.generate_full_merkle_tree(leaves);

//             assert_eq!(
//                 existing_root,
//                 full_tree.root(),
//                 "Roots should match for tree with {} leaves",
//                 num_leaves
//             );
//         }
//     }

//     #[test]
//     fn test_auth_path_array_padding() {
//         // Test that auth path arrays are correctly padded with zeros
//         let suite = HeadstashSuite::new();

//         let leaves: Vec<Fp> = (0..4).map(|i| Fp::from(i as u64)).collect();
//         let tree = suite.generate_full_merkle_tree(leaves);

//         let path = suite.compute_merkle_path(&tree, 0);
//         let arr: [Fp; 32] = path.to_auth_path_array();

//         // Actual siblings should be at the beginning
//         for (i, sibling) in path.siblings.iter().enumerate() {
//             assert_eq!(arr[i], *sibling, "Sibling {} should match", i);
//         }

//         // Rest should be zeros
//         for i in path.siblings.len()..32 {
//             assert_eq!(arr[i], Fp::ZERO, "Element {} should be zero-padded", i);
//         }
//     }
// }
