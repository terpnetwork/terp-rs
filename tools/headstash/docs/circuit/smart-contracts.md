# **Cw-Headstash – on-chain manifold

> **A modular, BLS12-381 threshold authenticated smart-account**

## Core Purpose

- register & maintain up to date record of a set of actively validated service operators keys
- authentical aggregated signatures for transactions
- validate zk-proofs on-chain
- headstash instance nullifier set storage medium
- token factory middleware for escrow and distribution

---

## Architecture Overview

### 1. CosmWasm Authenticator Lifecycle (Modular Authentication)

The contract implements the full `terp_account::traits::BtsgAccountTrait` (or equivalent `CosmWasmAuthenticator` interface) and responds to these sudo callbacks:

| Callback                  | Purpose                                                                                     | Implementation Detail |
|---------------------------|---------------------------------------------------------------------------------------------|-----------------------|
| `OnAuthenticatorAdded`    | Register new BLS aggregate keyset + proof-of-possession                                     | Validates each operator's PoP, stores `WavsOperatorSet` |
| `OnAuthenticatorRemoved`  | Clean up params on removal                                                                  | Clears state |
| `Authenticate`            | **Fast path**: Verify BLS12-381 aggregate signature over tx messages                        | Uses `bls12_381_aggregate_g1/g2` + pairing check |
| `Track`                   | No-op (can be used for analytics)                                                           | Returns OK |
| `ConfirmExecution`        | **Slow path**: Execute + verify zk-proof batch (nullifier claims)                           | Calls `extended_authenticate` → verifies Halo2/Plonk proofs + nullifiers |

> This enables **dual-mode authentication**: simple txs use fast BLS, private claims use zk + BLS.

#### 2. BLS12-381 Aggregated Threshold Key Management

- Uses **non-programmable aggregation** (simple sum in G1) – sufficient for known operator sets.
- Each operator submits:  
  - `pubkey ∈ G1` (hex-encoded compressed)  
  - `proof_of_possession = sk · H(pk)` in G2
- On rotation: requires **signed message by current threshold** approving new keyset + new PoPs.
- `WavsOperatorSet` stored immutably per nonce → supports **key rotation with versioning**.

#### 3. Token Strategy: New vs Existing (Fully Enforced)

```rust
#[cw_serde]
pub enum TokenStrategy {
    NewFungible(NewTokenConfig),      // Uses TokenFactory → creates + mints
    ExistingFungible(String),         // Uses pre-existing denom → must pre-fund
}
```

**Strict Enforcement at Instantiate & Claim Time**:

| Strategy           | Instantiate Behavior                                      | Claim-Time Checks                                      |
|--------------------|-----------------------------------------------------------|---------------------------------------------------------|
| `NewFungible`      | Emits `CreateDenom` + `MintTokens` via TokenFactory msgs | No balance check needed (mints on demand)              |
| `ExistingFungible` | Requires contract pre-funded with exact denom             | `query_balance(contract, denom) >= total_claim_amount` |

→ **Fails early** if insufficient balance for `ExistingFungible` before processing any claims.

#### 4. Privacy-Preserving Distribution (Headstash Core)

Each `HeadstashNote` contains:

```rust
pub struct HeadstashNote {
    pub nullifier: Binary,      // zk-SNARK nullifier (prevents double claim)
    pub recipient: String,      // bech32 address to receive funds
    pub amount: Coin,           // amount + denom
    pub proof: Binary,          // Halo2/Plonk proof (groth16 or ultra-plonk)
    pub public_inputs: Binary // Array of instances
}
```

**Execution Flow in `ProcessHeadstash` / `ConfirmExecution`**:

1. For each claim:
   - Verify nullifier not in `NULLIFIERS` map → insert atomically
   - (Optional) Verify Merkle inclusion proof against `GENESIS_TREE_ROOT`
   - Verify zk-proof using pre-loaded verifying key (stored in contract or passed)
   - **Critical**: Use **single hashed public input** (see optimization below)
2. Aggregate all `BankMsg::Send` or `TokenFactoryMsg::MintTokens`
3. Revert entire tx on any failure (nullifier duplicate, bad proof, insufficient balance)

#### 5. On-Chain Verification Optimization (Mandatory for Mainnet)

**Problem**: Original circuit had 254+ public inputs → ~700k–1M gas per verification  
**Solution**: Hash all public inputs into **one** → reduces to ~120–180k gas

**Recommended Public Inputs Hash (inside circuit)**:

```rust
let public_inputs = [
    genesis_root,
    nullifier,
    commitment,
    recipient_commitment,
    amount,
    token_denom_hash,
    merkle_path_hint,
    // ... all other public values
];

let public_hash = poseidon_hash(public_inputs);  // or keccak256
layouter.constrain_instance(public_hash.cell(), primary, 0)?;
```

→ Contract verifies only **one** public input = hash of all values  
→ Off-chain verifier reconstructs and re-hashes to validate correctness

**75–80% gas reduction** → enables mainnet-scale private airdrops.

#### 6. Smart Account & DAO Integration

- Contract self-registers as authenticator in `instantiate()` via:

  ```rust
  MsgAddAuthenticator {
      authenticator_type: "CosmwasmAuthenticatorV1",
      data: CosmwasmAuthenticatorInitData { contract: self, params: WavsOperatorSet }
  }
  ```

- Any DAO or smart account can now use **BLS threshold signatures** instead of EOAs.
- Supports **AllOf(passkey, wallet, DAO vote)** composite authenticators via macro injection.

## Headstash Manifold Contract

The Headstash Manifold Contract provides a centralized mechanism for deploying and managing multiple headstash instances, enabling scalable privacy-preserving distribution across different campaigns and token strategies.

### Core Factory Responsibilities

- **Code ID Management**: Stores and manages the headstash contract code identifier for instantiation
- **Contract Deployment**: Handles instantiation of headstash contracts with configurable parameters
- **Instance Tracking**: Maintains indexed registry of all deployed headstash contracts
- **Administrative Control**: Implements ownership-based access control for factory operations

### Factory Contract Architecture

#### 1. Contract Instantiation Management

The factory contract maintains a stored code identifier for headstash contracts and provides controlled instantiation capabilities:

| Function | Purpose | Implementation |
|----------|---------|----------------|
| `InstantiateHeadstashContract` | Deploys new headstash instance with specified parameters | Validates ownership, instantiates with provided genesis root, token strategy, and WAVS configuration |
| `UpdateCodeId` | Updates the stored headstash code identifier | Owner-only operation for contract upgrades |
| `ReceiveCw20` | Handles CW20 token-funded headstash instantiation | Processes token transfers and validates funding amounts |

#### 2. Instance Registry and Indexing

Maintains a comprehensive registry of all deployed headstash contracts with multi-dimensional indexing:

- **Contract Address Index**: Primary key for contract lookup and metadata retrieval
- **Instantiator Index**: Tracks contracts deployed by specific addresses
- **Recipient Index**: Indexes contracts by intended distribution recipients
- **Token Strategy Index**: Categorizes contracts by their token distribution mechanism

#### 3. Administrative Operations

Implements ownership-based governance for factory management:

| Operation | Access Control | Purpose |
|-----------|----------------|---------|
| Code ID Updates | Owner Only | Enables contract upgrades and version management |
| Factory Ownership Transfer | Owner Only | Supports decentralized administration transitions |
| Contract Registry Queries | Public | Provides transparency and audit capabilities |

#### 4. Instantiation Reply Processing

Handles post-deployment operations through reply callbacks:

- **Contract Registration**: Automatically registers newly deployed contracts in the factory registry
- **Metadata Extraction**: Queries instantiated contracts for configuration details and recipient information
- **CW20 Funding Dispatch**: For token-funded deployments, automatically dispatches funding messages to populate contract balances

### Factory-Headstash Integration

The factory contract serves as the primary deployment mechanism while maintaining loose coupling with individual headstash instances:

- **Parameter Validation**: Factory validates instantiation parameters before deployment
- **State Synchronization**: Tracks deployment metadata without interfering with individual contract operations
- **Upgrade Coordination**: Enables coordinated upgrades across multiple headstash instances

### Security Considerations

- **Ownership Enforcement**: All administrative operations require factory ownership verification
- **Instantiation Authorization**: Contract deployment restricted to authorized instantiators
- **Reply Handler Security**: Reply processing validates contract addresses and prevents manipulation
- **Storage Isolation**: Factory state remains isolated from individual headstash contract storage

### Proving Keys

We store the viewing keys of a headstash circuit inside a cosmwasm contract for proof verification.
