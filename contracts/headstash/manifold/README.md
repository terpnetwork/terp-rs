# CW-Headstash Manifold Contract

A CosmWasm factory contract for creating and managing headstash contracts, following the DA0-DA0 payroll factory pattern.

## Overview

This contract allows users to create headstash contracts with customizable parameters while maintaining ownership and tracking of all instantiated contracts.

## Features

- **Headstash Code ID Management**: Stores the code ID for headstash contract instantiation
- **Dual Token Support**: Supports both native tokens and CW20 tokens for funding
- **Contract Tracking**: Tracks instantiated contracts by both instantiator and recipient addresses
- **Ownership Management**: Uses cw-ownable for contract ownership control
- **Instantiation Replies**: Properly handles instantiation replies to register contracts
- **Query Interface**: Provides comprehensive queries for listing and filtering contracts

## Messages

### Instantiate
```json
{
  "owner": "optional_owner_address",
  "headstash_code_id": 123
}
```

### Execute

#### Update Ownership
```json
{
  "update_ownership": "action"
}
```

#### Create Headstash
```json
{
  "create_headstash": {
    "recipient": "recipient_address",
    "genesis_root": "base64_encoded_root",
    "token_strategy": {
      "NewFungible": {
        "subdenom": {
          "proof": "binary_proof",
          "raw": "denom_name"
        },
        "metadata": {
          "display": "DISPLAY",
          "name": "Token Name",
          "symbol": "SYMBOL"
        },
        "initial_mint": null,
        "manager": null,
        "minters": []
      }
    },
    "wavs": {
      "poos": [],
      "msg": {
        "aggregate_key": "hex_key",
        "threshold": 2,
        "total_operators": 3,
        "nonce": 0
      }
    },
    "funding": {
      "amount": "1000000",
      "token": {
        "Native": {
          "denom": "uterp"
        }
      }
    }
  }
}
```

### Query

#### Get Ownership
```json
{
  "ownership": {}
}
```

#### Contracts by Instantiator
```json
{
  "contracts_by_instantiator": {
    "instantiator": "instantiator_address",
    "start_after": "optional_cursor",
    "limit": 10
  }
}
```

#### Contracts by Recipient
```json
{
  "contracts_by_recipient": {
    "recipient": "recipient_address",
    "start_after": "optional_cursor",
    "limit": 10
  }
}
```

#### Get Contract
```json
{
  "contract": {
    "address": "contract_address"
  }
}
```

## Architecture

- `contract.rs`: Main contract logic with instantiate, execute, reply, and query handlers
- `msg.rs`: Message type definitions for all contract interactions
- `state.rs`: Storage definitions and indexing for contract tracking
- `error.rs`: Custom error types
- `lib.rs`: Module exports

## Dependencies

- `cw-ownable`: For ownership management
- `cw-storage-plus`: For indexed storage
- `cosmwasm-std`: Core CosmWasm functionality
- `cw-headstash`: The headstash contract being instantiated

## Usage

1. Deploy the factory contract with the headstash code ID
2. Use the `create_headstash` execute message to instantiate new headstash contracts
3. Query the factory for contract listings and details
4. Transfer ownership if needed using the ownership management features

## Testing

Run tests with:
```bash
cargo test
```

## Building

Build the contract with:
```bash
cargo build --release
```

The resulting WASM file will be in `target/wasm32-unknown-unknown/release/`.