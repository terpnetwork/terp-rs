# Charcute: Trustless & Scalable Orchestration Framework

## Versioning

### Verifying What You Are Using On Your Device

### Reviewing Centralization Surface Area

### Communicating Fees

## Permissionless Specs

### Developing

### Compiling

### Deployment

### Publishing

### Interacting

### Proof Verification

```mermaid

flowchart TD
    subgraph Off-Chain [Off-Chain Setup & Proof Generation]
        direction TB
        A[Halo2 Circuit Code] --> B[Trusted Setup Ceremony<br>Performed Once]
        B -- Generates --> C[Verification Key.vkey<br>Large, ~10s of KB]
        B -- Generates --> D[Proving Key.pkey<br>Very Large, ~100s of KB]
        C -- Stored on --> E[IPFS]
        E -- Yields --> F[Content Identifier CID]
        
        G[User's Private Data] -- Creates --> H[Witness]
        D & H -- Used by --> I[Wasm-Bindgen Prover]
        I -- Generates --> J[Proof π + Public Inputs]
    end

    subgraph OnChain [On-Chain Verification]
        K[Verifier Smart Contract]
        L[Transaction with<br>Proof π + Public Inputs + CID]
        
        F -- Fetched via --> M[IPFS Gateway]
        M -- Supplies --> N[VK Data]
        
        L --> K
        N --> K
        K -- Verifies Proof --> O[Verification Result<br>True/False]
    end

    F -- Stored in --> P[Contract State<br>mapping CID => bool]
    
```