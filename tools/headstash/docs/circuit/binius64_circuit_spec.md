# Binius64 Circuit System Specification

## Overview

This specification extends the CosmWasm ZK proof verification architecture to support Binius64, a multivariate polynomial-based proof system over binary fields. It leverages the existing generic middleware layer for circuit storage and deserialization, allowing users to upload Binius64 circuit binaries for verification in the VM.

**Key Goals:**

- Enable generic circuit support for Binius64 alongside Halo2.
- Maintain compatibility with existing middleware (zkid mapping, app state storage).
- Provide a complete pipeline: circuit compilation → serialization → upload → deserialization → verification.

## Architecture Integration

### Existing Middleware Layer

The CosmWasm VM's ZK middleware provides:

- **Circuit Storage**: `store_code_with_circuit()` uploads and stores VKs/CS in app state.
- **ZKID Mapping**: Logical circuit IDs (`zkid`) map to checksums for dynamic loading.
- **Deserialization**: VKs are loaded from storage and deserialized on-demand.
- **Verification**: Proofs verified against loaded VKs via host imports.

Binius64 integrates seamlessly by extending this layer for binary field operations.

### Binius64 Components

- **Proof System**: Multivariate polynomials over GF(2^k), optimized for 64-bit architectures.
- **Circuit Representation**: Constraints as binary polynomials, commitments via FRI.
- **Verification**: Evaluate polynomials and verify commitments in binary fields.

## Serialization Format Extension

Extend `ZK_CIRCUIT_SERIALIZATION_FORMAT.md` to support Binius64:

### Footer Metadata Update

Add `circuit_type` enum:

- 0: Halo2/Plonkish
- 1: Binius64

### Binius64 Sections

```
Halo2 Params (if hybrid) or Binius Params
Binius VK (Multivariate Commitments)
Polynomial CS (Binary Constraints)
Footer (32 bytes, circuit_type=1)
```

### Serialization Methods

Implement `write()`/`read()` for Binius structures:

- Polynomials: Serialize coefficients as binary arrays.
- Commitments: FRI proofs and roots.
- CS: Constraint polynomials.

## VM Middleware Extensions

### Host Import: `do_binius_proof_verify`

Add to `packages/vm/src/imports.rs`:

```rust
pub fn do_binius_proof_verify(
    env: FunctionEnvMut<Environment<A, S, Q>>,
    zkid: u32,
    proof_ptr: u32,
    instances_ptr: u32,
) -> VmResult<u32> {
    // Resolve zkid to checksum, load Binius VK, verify proof
}
```

### Environment: `resolve_zkid_to_system`

Extend `Environment` to detect proof system (Halo2 vs. Binius) via zkid.

### Cache: Binius VK Loading

Update `Cache` to deserialize and pin Binius VKs alongside Halo2.

## Circuit Compilation Pipeline

### From High-Level to Binius64

1. **Circuit Definition**: Write circuit in Rust using Binius traits (e.g., binary constraints).
2. **Compilation**: Use Binius compiler to generate multivariate polynomials.
3. **Serialization**: Package into VM-compatible format.
4. **Upload**: `store_code_with_circuit()` stores Binius binary.
5. **Verification**: VM deserializes and verifies proofs.

### Example Workflow

```rust
// Define binary circuit
let circuit = BinaryCircuit::new(constraints);

// Compile to Binius
let binius_circuit = BiniusCompiler::compile(circuit);

// Serialize
let serialized = serialize_binius_circuit(binius_circuit);

// Upload to VM
vm.store_code_with_circuit(serialized);
```

## App State and Storage

### Schema Extensions

- `zk_system:{zkid}` → `u8` (0=Halo2, 1=Binius)
- `zk_vk:{checksum}` → Binius VK bytes

### Upload Process

1. App layer validates Binius format.
2. Store zkid → system mapping.
3. Store checksum → VK bytes.

## Verification Flow

### Updated `do_zk_proof_verify`

```rust
fn verify_proof(zkid: u64, proof: &[u8], instances: &[u8]) -> u32 {
    let system = env.resolve_zkid_to_system(zkid);
    match system {
        0 => do_halo2_proof_verify(zkid, proof, instances),
        1 => do_binius_proof_verify(zkid, proof, instances),
        _ => 1, // Invalid
    }
}
```

### Binius Verification

- Deserialize VK from app state.
- Parse proof as multivariate evaluation.
- Verify against binary constraints.

## Dependencies and Implementation

### Crates

Add to `Cargo.toml`:

- `binius-core`
- `binius-field`
- `binius-hal`

### Code Locations

- **VM Imports**: `packages/vm/src/imports.rs` (new function)
- **Environment**: `packages/vm/src/environment.rs` (zkid resolution)
- **Cache**: `packages/vm/src/cache.rs` (Binius loading)
- **Serialization**: `packages/zk/src/cosmwasm_circuit.rs` (Binius support)

## Testing and Validation

- **Unit Tests**: Serialize/deserialize Binius circuits.
- **Integration**: Upload and verify Binius proofs in VM.
- **Compatibility**: Ensure Halo2 circuits still work.

## Performance Considerations

- Binius may offer faster verification for certain circuits.
- Memory usage: Binary fields are compact.
- Gas costs: Meter Binius operations similarly to Halo2.

This specification enables Binius64 support via the existing middleware, maintaining the VM's generic circuit handling.
