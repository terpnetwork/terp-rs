# Circuit Test Fixes: Genesis Merkle Tree Inclusion

## Problem Summary

The `round_trip` test in `circuit.rs` will fail due to a fundamental mismatch between how the test computes the anchor and how the circuit verifies it.

### Current Test Behavior (Incorrect)

```rust
// In generate_circuit_instance() - line 867-868
let path = MerklePath::dummy(&mut rng);
let anchor = path.root(spent_note.commitment().into());  // Uses NOTE COMMITMENT
```

### Circuit Behavior (Correct)

```rust
// In Circuit::synthesize() - lines 504-534
let genesis_leaf = gadget::derive_leaf(..., epk_crt, fdi, v, nd)?;  // Derives GENESIS LEAF
let genesis_root = merkle_inputs.calculate_root(..., genesis_leaf...)?;
layouter.constrain_instance(genesis_root.cell(), config.primary, ANCHOR)?;
```

### The Mismatch

| Component | Test Computes | Circuit Expects |
|-----------|---------------|-----------------|
| Leaf Value | Note Commitment (from nd, v, fdi, recp, esk, rho, psi, rcm) | Genesis Leaf Hash (from epk.x, epk.y, nd, v, fdi) |
| Hash Domain | `CommitDomain` with blinding | `HashDomain` (Leaf) without blinding |
| Anchor | `path.root(note_commitment)` | `merkle_path.calculate_root(genesis_leaf)` |

**Result**: The constraint `genesis_root == anchor` will always fail because they're computed from different leaf values.

---

## Required Changes

### 1. Add `genesis_leaf_hash()` Function to `tree.rs`

**Location**: `/zk-crates/zk-headstash/src/tree.rs`

**Purpose**: Compute the genesis leaf hash out-of-circuit using the same algorithm as the in-circuit `derive_leaf()` gadget.

```rust
use crate::constants::sinsemilla::LEAF_PERSONALIZATION;

/// Compute genesis leaf hash from (epk_x, epk_y, nd, v, fdi) using Sinsemilla HashDomain.
/// This matches the in-circuit computation in `headstash_merkle_tree::derive_leaf()`.
///
/// Message structure (640 bits total):
/// - a: epk_x[0..250)     - 250 bits
/// - b: epk_x[250..255) || epk_y[0] || nd[0..4)  - 10 bits
/// - c: nd[4..254)        - 250 bits
/// - d: nd[254..255) || v[0..9)   - 10 bits
/// - e: v[9..59)          - 50 bits
/// - f: v[59..64) || fdi[0..5)    - 10 bits
/// - g: fdi[5..55)        - 50 bits
/// - h: fdi[55..64) || padding    - 10 bits
pub fn genesis_leaf_hash(
    epk_x: pallas::Base,
    epk_y: pallas::Base,
    nd: pallas::Base,
    v: u64,
    fdi: u64,
) -> MerkleHashOrchard {
    let domain = HashDomain::new(LEAF_PERSONALIZATION);

    // Convert to little-endian bit representations
    let epk_x_bits = epk_x.to_le_bits();
    let epk_y_bits = epk_y.to_le_bits();
    let nd_bits = nd.to_le_bits();
    let v_bits = (v as u64).to_le_bits();
    let fdi_bits = (fdi as u64).to_le_bits();

    // Build message matching in-circuit decomposition
    let message = iter::empty()
        // a: epk_x[0..250)
        .chain(epk_x_bits.iter().by_vals().take(250))
        // b: epk_x[250..255) || epk_y[0] || nd[0..4)
        .chain(epk_x_bits.iter().by_vals().skip(250).take(5))
        .chain(epk_y_bits.iter().by_vals().take(1))
        .chain(nd_bits.iter().by_vals().take(4))
        // c: nd[4..254)
        .chain(nd_bits.iter().by_vals().skip(4).take(250))
        // d: nd[254..255) || v[0..9)
        .chain(nd_bits.iter().by_vals().skip(254).take(1))
        .chain(v_bits.iter().by_vals().take(9))
        // e: v[9..59)
        .chain(v_bits.iter().by_vals().skip(9).take(50))
        // f: v[59..64) || fdi[0..5)
        .chain(v_bits.iter().by_vals().skip(59).take(5))
        .chain(fdi_bits.iter().by_vals().take(5))
        // g: fdi[5..55)
        .chain(fdi_bits.iter().by_vals().skip(5).take(50))
        // h: fdi[55..64) || padding (1 bit)
        .chain(fdi_bits.iter().by_vals().skip(55).take(9))
        .chain(iter::once(false));  // 1-bit padding

    MerkleHashOrchard(
        domain.hash(message).unwrap_or(pallas::Base::zero())
    )
}
```

### 2. Add `root_from_genesis_leaf()` Method to `MerklePath`

**Location**: `/zk-crates/zk-headstash/src/tree.rs` (in `impl MerklePath`)

**Purpose**: Calculate merkle root from a genesis leaf (not an `ExtractedNoteCommitment`).

```rust
impl MerklePath {
    // ... existing methods ...

    /// Calculate merkle root from a genesis leaf hash.
    /// Unlike `root()` which takes an ExtractedNoteCommitment,
    /// this takes a raw MerkleHashOrchard for genesis tree inclusion.
    pub fn root_from_genesis_leaf(&self, leaf: MerkleHashOrchard) -> Anchor {
        self.auth_path
            .iter()
            .enumerate()
            .fold(leaf, |node, (l, sibling)| {
                let l = l as u8;
                if self.position & (1 << l) == 0 {
                    MerkleHashOrchard::combine(l.into(), &node, sibling)
                } else {
                    MerkleHashOrchard::combine(l.into(), sibling, &node)
                }
            })
            .into()
    }
}
```

### 3. Update `generate_circuit_instance()` in `circuit.rs`

**Location**: `/zk-crates/zk-headstash/src/circuit.rs` (lines 840-909)

**Changes**:

```rust
fn generate_circuit_instance<R: RngCore>(mut rng: R) -> (Circuit, Instance) {
    let (_, fvk, esk, spent_note) = Note::dummy(&mut rng, None);
    let (epkx, epky) = esk.epk().xy();

    // Convert secp256k1 coordinates to pallas::Base for genesis leaf
    let epk_x_pallas = {
        // The native representation used in-circuit
        let fp = Secp256k1Fp::from_bytes(&epkx).expect("valid Fp");
        // Convert to pallas::Base - this matches how the circuit loads epk
        pallas::Base::from_repr(fp.to_repr()).unwrap_or(pallas::Base::zero())
    };
    let epk_y_pallas = {
        let fp = Secp256k1Fp::from_bytes(&epky).expect("valid Fp");
        pallas::Base::from_repr(fp.to_repr()).unwrap_or(pallas::Base::zero())
    };

    // ... rest of key setup ...

    let path = MerklePath::dummy(&mut rng);

    // FIXED: Compute anchor from genesis leaf hash (matching circuit)
    let genesis_leaf = crate::tree::genesis_leaf_hash(
        epk_x_pallas,
        epk_y_pallas,
        spent_note.nd().to_pallas(),
        spent_note.value().inner(),
        spent_note.fdi(),
    );
    let anchor = path.root_from_genesis_leaf(genesis_leaf);

    // ... rest of function unchanged ...
}
```

### 4. Add Required Import

**Location**: `/zk-crates/zk-headstash/src/circuit.rs` (top of file)

```rust
use crate::tree::genesis_leaf_hash;  // Add this import
```

---

## Implementation Notes

### EPK Representation Challenge

The `epk` in the circuit is loaded as `CrtInteger<pallas::Base>` via `Secp256k1Chip::load_private()`. This represents a secp256k1 field element using 3×88-bit limbs in the CRT representation.

For the out-of-circuit computation, we need to match how `epk.0.native` is used in-circuit. The `.native` field is the value reduced modulo the Pallas base field.

**Key insight**: When the test creates `Secp256k1Fp::from_bytes(&epkx)`, this creates a secp256k1 field element. The circuit then loads this as a CRT integer and accesses `.native` which gives the pallas::Base representation.

### Bit Decomposition Must Match Exactly

The genesis leaf hash uses a specific bit decomposition:

| Piece | Bits | Content |
|-------|------|---------|
| a | 250 | epk_x[0..250) |
| b | 10 | epk_x[250..255) \|\| epk_y[0] \|\| nd[0..4) |
| c | 250 | nd[4..254) |
| d | 10 | nd[254..255) \|\| v[0..9) |
| e | 50 | v[9..59) |
| f | 10 | v[59..64) \|\| fdi[0..5) |
| g | 50 | fdi[5..55) |
| h | 10 | fdi[55..64) \|\| padding |

**Total**: 640 bits

The out-of-circuit function MUST produce the exact same bit sequence.

### Hash Domain

The hash domain uses personalization string `"t.network:Headstash-Sinsemilla-leaf"` defined in:

- `/zk-crates/zk-headstash/src/constants/sinsemilla.rs` as `LEAF_PERSONALIZATION`

This maps to `OrchardHashDomains::Leaf` in-circuit.

---

## Verification Checklist

After implementing these changes:

- [ ] `genesis_leaf_hash()` produces same output as in-circuit `derive_leaf()` for identical inputs
- [ ] `root_from_genesis_leaf()` computes same root as `MerklePath::calculate_root()` in circuit
- [ ] `round_trip` test passes with `MockProver::verify() == Ok(())`
- [ ] Full proof generation and verification succeeds

---

## Testing the Fix

```bash
# Run the circuit tests
cd /Users/returniflost/TERPNETWORK/zk-airdrop/genesis-sinsemilla
cargo test -p zk-headstash round_trip -- --nocapture

# Run all circuit-related tests
cargo test -p zk-headstash --lib circuit:: -- --nocapture
```

---

## Related Files

| File | Purpose |
|------|---------|
| `src/tree.rs` | Add `genesis_leaf_hash()` and `root_from_genesis_leaf()` |
| `src/circuit.rs` | Update `generate_circuit_instance()` to use genesis leaf |
| `src/circuit/headstash_merkle_tree.rs` | In-circuit leaf derivation (reference for bit decomposition) |
| `src/constants/sinsemilla.rs` | `LEAF_PERSONALIZATION` constant |
