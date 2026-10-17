# Merkle Tree Inclusion Test Plan

This plan outlines the changes needed to add and implement a test that fulfills merkle tree inclusion accuracy in the zk-headstash circuit. The test will verify that the circuit correctly constrains a leaf's inclusion in the genesis merkle tree by computing the root from the leaf and path, and ensuring it matches the provided root. This builds on the existing leaf hash tests in `/Users/returniflost/TERPNETWORK/zk-airdrop/genesis-sinsemilla/zk-crates/zk-headstash/src/circuit/headstash_merkle_tree.rs`.

## Prerequisites and Dependencies

- Ensure `shardtree` or a simple Sinsemilla merkle implementation is available (already in Cargo.toml).
- Import necessary types: `pasta_curves::pallas`, `halo2_gadgets::sinsemilla::HashDomain`, `OrchardHashDomains`, etc.
- The `constrain_genesis_inclusion` function must be implemented (as added previously) but is not yet used in tests.

## Sequence of Steps for Implementation

1. **Set Up Test Data Builders in test-press**: Add a new module `test_data_builders.rs` to `/Users/returniflost/TERPNETWORK/zk-airdrop/genesis-sinsemilla/test-press/src/`. Create a trait `MerkleTestDataBuilder` with methods for generating merkle trees and paths. Implement functions like `generate_merkle_tree` and `compute_merkle_path`.

2. **Implement Merkle Tree Helpers**: In the new module, define `generate_merkle_tree(leaves: Vec<pallas::Affine>) -> (Vec<Vec<pallas::Affine>>, pallas::Affine)` to build a 32-level tree using Sinsemilla `MerkleCrh` domain for hashing pairs.

3. **Add Path Computation**: Implement `compute_path(tree: &[Vec<pallas::Affine>], leaf_index: usize) -> Vec<pallas::Affine>` to extract the sibling path for inclusion proofs.

4. **Update Test Circuit Config**: Extend the test circuit's `Config` to include `MerkleConfig`. In `configure`, add `let merkle_config = MerkleChip::configure(meta, ecc_config, sinsemilla_config);`.

5. **Modify Synthesize for Inclusion**: In the circuit's `synthesize`, load `MerkleChip`, and prepare to call `constrain_genesis_inclusion` with loaded path witnesses and root instance.

6. **Add New Test Function**: Create `#[test] fn test_genesis_inclusion()` in the test file. Use `MerkleTestDataBuilder` to generate tree data, pick a leaf, compute path/root, and run the circuit.

7. **Integrate Witnesses**: Load `epk`, `nd`, `v`, `fdi` as before; additionally load path as `[pallas::Affine]` witnesses and root as `pallas::Base` instance.

8. **Call Inclusion Constraint**: In the test circuit, invoke `constrain_genesis_inclusion` with all parameters, ensuring the circuit enforces path-to-root verification.

9. **Run and Validate**: Execute `MockProver`, assert success for valid inclusion. Test edge cases like single-leaf trees or invalid paths to confirm failures.

10. **Refine and Document**: Update comments, ensure no regressions with existing tests, and document the new functions in test-press README.

## Out-of-Circuit Merkle Tree Generation Logic

- **Add Helper Module**: Created `test_data_builders.rs` in test-press with `MerkleTestDataBuilder` trait and functions for reusability.
  - Function `generate_merkle_tree(leaves: Vec<pallas::Affine>) -> (Vec<Vec<pallas::Affine>>, pallas::Affine)`: Builds a 32-level tree. For each level, hash pairs using `MerkleCrh` domain (left.x, left.y, right.x, right.y as 254-bit ranges). Current impl is placeholder; needs full Sinsemilla hashing.
  - Function `compute_path(tree: &[Vec<pallas::Affine>], leaf_index: usize) -> Vec<pallas::Affine>`: Traverses up, collecting siblings.
- **Tree Structure**: Leaves at level 0; root at level 32. Use dummy leaves (random points) for simplicity.

## Test Circuit Modifications

- **Extend Config**: Add `MerkleConfig` to the test circuit's `Config` tuple (e.g., `type Config = (LeafHashConfig, SinsemillaConfig, EccConfig, MerkleConfig)`).
- **Configure Merkle**: In `configure`, add `let merkle_config = MerkleChip::configure(meta, ecc_config, sinsemilla_config);`.
- **Synthesize Updates**: Load `MerkleChip`, pass to `constrain_genesis_inclusion`. Load path as `[pallas::Affine]` witnesses, root as `pallas::Base` instance.

## New Test Implementation

- **Add Test Function**: `#[test] fn test_genesis_inclusion()`.
  - Generate 4-8 dummy leaves (random `epk`, `nd`, `v`, `fdi` hashed to points).
  - Build tree, pick a leaf index, compute path and root.
  - Create circuit with witnesses (epk, nd, v, fdi, path, root).
  - Call `constrain_genesis_inclusion`, run `MockProver`, assert verification succeeds.
- **Edge Cases**: Test single-leaf tree, invalid path (should fail), boundary indices.

## Integration with Existing Tests

- Keep leaf hash tests separate; inclusion test builds on them by adding path/root constraints.
- Ensure circuit config supports both (shared advice columns).

## Validation and Accuracy Checks

- Verify root computation matches out-of-circuit calculation.
- Test fails for wrong root/path, ensuring constraint accuracy.
- Run with various tree sizes to confirm 32-level depth.

This plan ensures the test accurately validates merkle inclusion. Estimated changes: 100-200 lines in test file, plus 50-100 lines in test-press for helpers (partially implemented).
