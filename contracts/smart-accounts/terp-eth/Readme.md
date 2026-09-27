# terp-eth

authentication for eth based signatures of off-chain actions.

 eth personal signatures : <https://eips.ethereum.org/EIPS/eip-191>

# Ethereum Personal Signature Authenticator for Terp Accounts

## Overview

This module implements an **Ethereum Personal Signature Authenticator** for the [Terp Account Trait](https://github.com/terp-auth/docs) and [x/smart-account module](https://github.com/terpnetwork/terp-core/blob/main/x/smart-account/README.md). It enables authentication of Cosmos transactions using **EVM-compatible personal signatures** (EIP-191 style), allowing Ethereum wallets (e.g., MetaMask, Ledger) to sign Cosmos txs without native Cosmos key management. This bridges EVM and Cosmos ecosystems for seamless cross-chain UX.

### Key Features
- **EIP-191 Signing**: Signs the tx data with the Ethereum personal message prefix (`\x19Ethereum Signed Message:\n<len>` + msg), hashed via Keccak256, then ECDSA secp256k1.
- **Pubkey Storage**: Stores the Ethereum pubkey (hex string) on instantiate/add; used for on-chain recovery/verification.
- **Stateless Verification**: In `on_auth_request`, constructs `EthPersonalSign` from tx data (`sign_mode_direct`), signature, and pubkey; verifies via `saa::Verifiable`.
- **Integration Flow**:
  - **Authenticate (`on_auth_request`)**: Deserialize sig, load pubkey, verify against tx bytes. Fails tx if invalid.
  - **Track/Confirm**: Placeholders; extend for nonces or post-exec checks (e.g., gas limits).
  - **Added/Removed**: Basic setup/teardown; TODOs for DAO membership/RBAM filters (e.g., via [cw-jsonfilter](https://github.com/DA0-DA0/dao-contracts/blob/development/packages/cw-jsonfilter/README.md)).
- **Security**:
  - Pubkey tied to account owner.
  - No state mutation in auth (stateless).
  - Supports composites: Nest in `AnyOf` (EVM OR Cosmos sig) or `AllOf` (EVM AND multisig).
- **Dependencies**: `saa` crate for `EthPersonalSign` and `Verifiable` trait; `terp_auth` for hooks.

### When to Use
- EVM users signing Cosmos txs (e.g., IBC transfers from Ethereum wallets).
- DAO governance: Verify EVM sigs for membership (extend `on_auth_added`).
- Hybrid wallets: Fallback to Ethereum signing for Cosmos dApps.

## Core Types

Import via `use crate::msg::*;` or equivalent.

### Messages
| Type | Description | Fields |
|------|-------------|--------|
| `InstantiateMsg` | Contract init: Sets owner and Ethereum pubkey (uncompressed hex, e.g., "0x04..."). | `owner: Option<Addr>`, `pubkey: String` |
| `ExecuteMsg` | Empty (pure authenticator). | N/A |
| `QueryMsg` | Empty (extend for pubkey retrieval). | N/A |

### Auth Structs
| Struct | Description | Fields |
|--------|-------------|--------|
| `TerpAccountEthStructs` | Placeholder for extensions (e.g., chain ID). | N/A |
| `TerpAccountEth` | Main trait impl. | N/A |

### Eth Types (from `saa`)
- **`EthPersonalSign`**: Signing structure.
  | Field | Type | Description |
  |-------|------|-------------|
  | `message` | `Binary` | Raw tx data bytes (`sign_mode_direct`). |
  | `signature` | `Binary` | 65-byte sig (r,s,v; DER or compact). |
  | `signer` | `String` or `Binary` | Pubkey (hex) or recovered address. |

- **Verification**: Uses `secp256k1` (similar to passkey); recovers signer from sig, matches stored pubkey/address.

## Trait Implementation Details

`TerpAccountEth` implements `TerpAccountTrait`. Key methods:

### Associated Types
| Type | Value |
|------|-------|
| `InstantiateMsg` | `InstantiateMsg` |
| `ExecuteMsg` | `ExecuteMsg` |
| `QueryMsg` | `QueryMsg` |
| `SudoMsg` | `terp_auth::AuthSudoMsg` |
| `ContractError` | `ContractError` |
| `AuthMethodStructs` | `TerpAccountEthStructs` |
| `AuthProcessResult` | `Result<Response, ContractError>` |

### Core Methods
- **`process_sudo_auth`**: Routes to hooks via match on `SudoMsg` variants (e.g., `Authenticate` → `on_auth_request`).
- **`on_auth_added`**: TODO: Check DAO membership, register JSON filters. Currently `Ok(Response::new())`.
- **`on_auth_removed`**: TODO: Cleanup data. Currently `Ok(Response::new())`.
- **`on_auth_request`**:
  1. Load pubkey from `state::PUBLIC_KEY`.
  2. Build `EthPersonalSign { message: req.sign_mode_tx_data.sign_mode_direct, signature: req.signature, signer: pubkey }`.
  3. Verify: `cred.verify(deps.as_ref())?` (EIP-191 hash + ECDSA).
  4. Emits "auth_req" event.
- **`on_auth_track` / `on_auth_confirm` / `on_hooks`**: Placeholders; emit empty responses.
- **`extended_authenticate`**: TODO; for internal use.

### Verification Process (via `saa::Verifiable`)
- **Hash**: `\x19Ethereum Signed Message:\n${message.len()}${message}` → Keccak256.
- **Sig Format**: 65 bytes (r:32, s:32, v:1; v=27/28 for recovery).
- **Recovery**: Recover pubkey/address from sig + hash; match against stored `pubkey`.
- **Crypto**: `secp256k1` ECDSA; low-S normalized. Uses Cosmos API (`deps.api.secp256k1_recover_pubkey` or similar in `saa`).

## Client-Side Generation in Rust

Generate the EIP-191 personal signature off-chain over the Cosmos tx bytes. Use for wallets integrating Ethereum signing (e.g., via `ethers-rs` for MetaMask compat). Embed the 65-byte sig (hex or bytes) in `AuthenticationRequest.signature`.

### Dependencies (Client Crate)
```toml
[dependencies]
ethers = "2.0"  # EVM signing/Keccak.
secp256k1 = { version = "0.28", features = ["rand-std", "recovery"] }
hex = "0.4"
cosmos-sdk-proto = "0.20"  # Tx serialization.
serde_json = "1.0"
rand = "0.8"
```

### Steps
1. **Serialize Tx Data**: Get raw `sign_mode_direct` bytes (full tx without sigs).
2. **Personal Hash**: Prefix message, Keccak256.
3. **Sign**: ECDSA with private key (EVM wallet).
4. **Encode Sig**: Compact 65 bytes (r,s,v).
5. **Build Tx**: Include sig in `signature`; set `sign_mode: SIGN_MODE_DIRECT`.

### Example Code: `src/main.rs`
```rust
use ethers::prelude::*;
use ethers::signers::{LocalWallet, Signer};
use ethers::types::H160;
use hex::encode as hex_encode;
use secp256k1::{Secp256k1, Message, SecretKey};
use std::str::FromStr;

// Assume tx_bytes: Vec<u8> from cosmos-sdk-proto (sign_mode_direct).
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load EVM private key (e.g., from env or wallet).
    let private_key_hex = "0x...";  // Your EVM privkey.
    let wallet = LocalWallet::from_str(private_key_hex)?;

    // 2. Get tx bytes (pseudo; serialize via cosmos-sdk-proto).
    let tx_bytes: Vec<u8> = vec![/* raw sign_mode_direct tx */];  // e.g., from TxRaw.

    // 3. Compute EIP-191 personal hash.
    let message = eth_personal_hash(&tx_bytes);
    let hash = H256::from_slice(&message);

    // 4. Sign with ethers (handles prefix internally? No; use raw).
    let sig: Signature = wallet.sign_message(PersonalMessage::new(&tx_bytes)).await?;

    // Alternative: Raw secp256k1 for Cosmos compat.
    // let secp = Secp256k1::new();
    // let secret_key = SecretKey::from_slice(&wallet.signing_key().to_bytes())?;
    // let msg = Message::from_slice(&hash[..])?;
    // let (sig, rec_id) = secp.sign_ecdsa_recoverable(&msg, &secret_key);
    // let mut sig_bytes = sig.serialize_compact().to_vec();
    // sig_bytes.push((rec_id.to_i32() as u8 + 27 + 35) % 256);  // v adjustment for EVM.
    // let sig_hex = format!("0x{}", hex_encode(&sig_bytes));

    // 5. Encode sig (65 bytes hex or bytes).
    let sig_bytes = sig.to_vec();  // r (32) + s (32) + v (1).
    let sig_hex = format!("0x{}", hex_encode(&sig_bytes));

    // 6. Build Cosmos AuthenticationRequest.
    let auth_req = terp_auth::AuthenticationRequest {
        account: "cosmos1...".to_string(),
        message: tx_bytes.clone().into(),  // Or encoded.
        signers: vec!["cosmos1...".to_string()],
        signature: sig_hex.as_bytes().to_vec(),  // Or Binary::from(sig_bytes).
        sign_mode_tx_data: terp_auth::SignModeTxData {
            sign_mode_direct: tx_bytes.into(),
        },
        // ...
    };

    // 7. Broadcast: Use cosmos-sdk client; set selected_authenticators to Eth ID.
    println!("Sig hex: {}", sig_hex);
    println!("Personal hash: {:?}", hash);

    Ok(())
}

/// Compute EIP-191 hash: Keccak256("\x19Ethereum Signed Message:\n<len>" + msg).
fn eth_personal_hash(message: &[u8]) -> Vec<u8> {
    use sha3::{Digest, Keccak256};
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message);
    hasher.finalize().to_vec()
}

// Verify client-side (optional, for testing).
fn verify_eth_personal_sig(tx_bytes: &[u8], sig_hex: &str, expected_pubkey_hex: &str) -> bool {
    use secp256k1::{Secp256k1, Message, PublicKey, SecretKey};
    use sha3::Keccak256;
    use sha3::Digest;

    let hash = eth_personal_hash(tx_bytes);
    let msg = Message::from_slice(&hash).unwrap();

    let sig_bytes = hex::decode(&sig_hex.strip_prefix("0x").unwrap_or(sig_hex)).unwrap();
    if sig_bytes.len() != 65 { return false; }
    let v = sig_bytes[64];
    let mut sig_compact = sig_bytes[0..64].to_vec();
    let rec_id = secp256k1::RecoveryId::from_i32(v as i32 - 27).unwrap();  // EVM v=27/28.

    let secp = Secp256k1::new();
    let sig = secp256k1::ecdsa::Signature::from_compact(&sig_compact).unwrap();
    let pubkey = secp.recover_ecdsa(&msg, &sig, rec_id).unwrap();

    let recovered_hex = format!("{:?}", pubkey);  // Or to hex.
    let expected_pubkey = hex::decode(expected_pubkey_hex.strip_prefix("0x").unwrap_or(expected_pubkey_hex)).unwrap();
    let expected_pk = PublicKey::from_slice(&expected_pubkey).is_ok();  // Simplified match.

    pubkey == PublicKey::from_slice(&expected_pubkey).unwrap()  // Full match.
}
```

### Client Notes
- **Wallet Integration**: Use `ethers` for EVM wallets; `cosmos-sdk-proto` for tx serialization (e.g., `prost` build).
- **v Adjustment**: EVM v=27/28; Cosmos may need +35 for chain ID (check `saa` impl).
- **Error Handling**: Catch invalid keys/hashes; retry on nonce issues.
- **Testing**: Mock tx_bytes; verify with `verify_eth_personal_sig`.
- **Browser/JS**: Use `ethers.js`: `wallet.signMessage(ethers.utils.arrayify(txBytes))`.

## Signature Verification Details

On-chain verification (in `on_auth_request` via `EthPersonalSign::verify` from `saa::Verifiable`):

### Process
1. **Load Pubkey**: From `PUBLIC_KEY` singleton (hex → bytes).
2. **Hash Message**: EIP-191 personal hash of `sign_mode_direct` bytes.
3. **Recover Signer**: From sig (65 bytes) + hash using `secp256k1_recover_pubkey` (Cosmos API).
4. **Match**: Recovered pubkey/address == stored pubkey.
5. **Normalize**: Low-S sig for malleability.
6. **Error**: `ContractError` if mismatch (e.g., invalid sig).

### Pseudo-Code (from `saa`)
```rust
impl Verifiable for EthPersonalSign {
    fn verify(&self, deps: Deps) -> Result<(), Error> {
        let hash = eth_personal_hash(&self.message);  // Keccak256(prefix + msg).
        let recovered = deps.api.secp256k1_recover_pubkey(&hash, &self.signature)?;
        if recovered != self.signer { return Err(Error::InvalidSig); }
        Ok(())
    }
}
```

- **Sig Format**: Compact 65 bytes (r:32, s:32, v:1); v adjusted for recovery.
- **Pubkey**: Uncompressed (65 bytes, 0x04 prefix) or compressed (33 bytes); match full bytes.
- **Gas**: ~30k for recovery; efficient for ante handler.

## Adding as Authenticator

1. **Instantiate**: Deploy with `InstantiateMsg { owner: Some(addr), pubkey: "0x04..." }`. Stores in `PUBLIC_KEY`.
2. **Add to Account**:
   ```bash
   terpd tx smart-account add-authenticator <account> CosmWasmAuthenticatorV1 '{"contract": "<eth_addr>", "params": {"pubkey": "0x04..."}}' --from <user>
   ```
   - `params`: Optional; overrides instantiate if provided.
3. **Tx Usage**: Serialize tx as `SIGN_MODE_DIRECT`; sign personal hash; include sig hex/bytes in `signature`. Select Eth ID in `TxExtension`.
4. **Remove**: `MsgRemoveAuthenticator`; cleans up.

## Extending & Testing
- **TODOs**: Add DAO checks in `on_auth_added` (query membership); RBAM filters for msg types.
- **Storage**: Extend `PUBLIC_KEY` for nonces (blacklist used hashes in `on_auth_track`).
- **Tests**: `cw-multi-test` with `MockApi` for `secp256k1_recover_pubkey`. Generate sigs with example.
- **Limitations**: No chain ID replay protection (extend hash); EVM-only (secp256k1).

For `saa` details, see crate. Contribute to Terp!