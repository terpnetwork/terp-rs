# Zk-Crates: Private Headstash

A fork of [zcash orchard protocol](https://github.com/zcash/orchard)

A zk-proof circuit dedicated to public allocations, private distributions (headstshes). To learn about the specification, check out the [documentation](../../docs/zk-headstash/spec)

## WARNING: THIS IS A WORK IN PROGRESS. SUPER EXPERIEMENTAL TYPE BEAT. USE AT YOUR DISCRETION

## Feature Flags

The `zk-headstash` crate uses feature flags to minimize binary size and compilation time by making optional components conditional. This allows downstream crates to import only what they need.

| Feature | Description | Dependencies Enabled | Modules Enabled |
|---------|-------------|---------------------|-----------------|
| `circuit` | Core ZK circuit implementation including proving/verifying key generation | `halo2_gadgets`, `halo2_proofs` | `circuit`, `example_circuits` |
| `interface` | High-level suite interface for headstash operations (tree generation, proof creation, key management) | `circuit`, `headstash-randomness`, `rayon` | `deploy` |
| `std` | Standard library support (required for file I/O operations) | Enables `std` on dependencies | - |
| `multicore` | Enable multi-threaded proving | `halo2_proofs/multicore` | - |
| `dev-graph` | Development tools for circuit visualization | `image`, `plotters`, `halo2_proofs/dev-graph` | - |
| `rpc` | RPC interface support (reserved for future use) | - | - |

### Default Features

```toml
default = ["circuit", "multicore", "std", "interface"]
# Usage Examples
#
# **Minimal build (core types only, no circuit or suite):**
# [dependencies]
# zk-headstash = { version = "*", default-features = false }
#
#
# **Circuit support only (no suite interface):**
# [dependencies]
# zk-headstash = { version = "*", default-features = false, features = ["circuit", "std"] }
#
# **Full functionality (default):**
#
# [dependencies]
# zk-headstash = { version = "*" }
```

### Binary Targets

All binary targets require specific features to compile:

| Binary | Required Features | Purpose |
|--------|------------------|---------|
| `gen_headstash_keys` | `interface` | Generate circuit proving and verifying keys |
| `create_headstash_proof` | `interface` | Create a headstash proof from note parameters |
| `gen_headstash_notes` | `interface` | Generate notes and merkle tree from genesis allocation |
| `create_test_circuit_data` | `circuit` | Generate test circuit data for examples |
| `gen_randomness` | `interface` | Generate secure randomness for headstash operations |

## Building Circuit

## Testing

## Releasing Libary
