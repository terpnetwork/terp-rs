# terp-passkey

## TODO

- implement `PasskeyPayload.other_keys`
- basic tests

## Overview

This module implements a **Passkey Authenticator** for the [Terp Account Trait](https://github.com/terp-auth/docs) and [x/smart-account module](https://github.com/terpnetwork/terp-core/blob/main/x/smart-account/README.md). It enables passwordless, phishing-resistant authentication using **WebAuthn/FIDO2 passkeys** (e.g., device-bound keys like Touch ID or YubiKeys) in CosmWasm contracts. Passkeys provide cryptographic credentials that can be verified on-chain without revealing secrets.

### Key Features

- **Stateless Verification**: Uses ECDSA secp256r1 signatures over hashed challenges, verifiable via Cosmos SDK's `secp256r1_verify`.
- **Origin Binding**: Enforces client origin (e.g., dApp domain) to prevent cross-site attacks.
- **Integration Flow**:
  - **Authenticate (`on_auth_request`)**: Deserialize credential from tx signature, validate fields/origin, verify sig with pubkey.
  - **Track (`on_auth_track`)**: Placeholder for nonce/logging (extendable).
  - **Confirm (`on_auth_confirm`)**: Post-exec checks (e.g., balance diffs via events).
  - **Added/Removed**: Basic setup/teardown; requires `authenticator_params` on add.
- **Security**:
  - Challenge from base64url to bytes for hashing.
  - Low-S normalization for signatures.
  - Owner check via `cw_ownable`.
- **Dependencies**: Relies on `saa` crate for passkey types/utils (`PasskeyCredential`, `Verifiable`, `PasskeyPayload`) and `saa_common` for JSON handling/crypto.

Passkeys are added as `CosmwasmAuthenticatorV1` with `params` containing `PasskeyPayload` (e.g., origin, challenge). Clients generate credentials off-chain (e.g., via WebAuthn API) and embed in tx signatures.

### When to Use

- Replace seed phrases with biometric/hardware keys for user-friendly, secure tx signing.
- Hybrid auth: Nest in `AnyOf` (e.g., passkey OR signature) or `AllOf` (passkey AND multisig).
- dApps: Bind to specific origins for session-like auth.

## Core Types

Import via `use crate::msg::*;` or equivalent.

### Messages

| Type | Description | Fields |
|------|-------------|--------|
| `InstantiateMsg` | Contract init: Sets owner and initial payload (e.g., origin). | `owner: Option<Addr>`, `payload: PasskeyPayload` |
| `ExecuteMsg` | Empty (pure authenticator; no user exec). | N/A |
| `QueryMsg` | Empty (extend if needed for pubkey/used challenges). | N/A |

### Auth Structs

| Struct | Description | Fields |
|--------|-------------|--------|
| `TerpAccountPasskeysAuthStruct` | Placeholder for custom auth extensions (unused; extend for params). | N/A |
| `TerpAccountPasskey` | Main impl struct for trait. | N/A |

### Passkey Types (from `saa`)

- **`PasskeyCredential`**: Core credential from client. Serialized as JSON in tx `signature`.

  | Field | Type | Description |
  |-------|------|-------------|
  | `id` | `String` | Unique passkey ID (e.g., base64 credential ID). |
  | `signature` | `Binary` | secp256r1 sig (64 bytes, compact). |
  | `authenticator_data` | `Binary` | WebAuthn auth data (≥37 bytes; includes flags, sign counter). |
  | `client_data` | `ClientData` | JSON: `{type: "webauthn.get", challenge: base64url, origin: String}`. |
  | `user_handle` | `Option<String>` | Reserved (e.g., user ID). |
  | `pubkey` | `Option<Binary>` | secp256r1 pubkey (33/65 bytes, SEC1); optional if contract-stored. |

- **`PasskeyPayload`** (from `saa::types`): Stored state (via `state::PAYLOAD`).
  - Includes `origin: Option<String>` for binding (e.g., "<https://my-dapp.com>").

- **`ClientData`** (embedded in `PasskeyCredential`): WebAuthn client JSON.

## Trait Implementation Details

The `TerpAccountPasskey` implements `TerpAccountTrait`. Key methods:

### Associated Types

| Type | Value |
|------|-------|
| `InstantiateMsg` | `InstantiateMsg` |
| `ExecuteMsg` | `ExecuteMsg` |
| `QueryMsg` | `QueryMsg` |
| `SudoMsg` | `terp_auth::AuthSudoMsg` (routes to hooks) |
| `ContractError` | `crate::error::ContractError` (e.g., `Unauthorized`) |
| `AuthMethodStructs` | `TerpAccountPasskeysAuthStruct` |
| `AuthProcessResult` | `Result<Response, ContractError>` |

### Core Methods

- **`extended_authenticate`**: TODO; for internal wrappers (e.g., pubkey gen).
- **`process_sudo_auth`**: TODO; routes `SudoMsg` to hooks (e.g., match `Authenticate { .. }` to `on_auth_request`).
- **`on_auth_added`**: Validates `req.authenticator_params` (must be `Some`); stores payload/origin.
- **`on_auth_removed`**: Basic cleanup; emits event.
- **`on_auth_request`**:
  1. Deserialize `req.signature` as `PasskeyCredential` via `from_json`.
  2. Owner check: `cw_ownable::is_owner(deps.storage, &req.account)?`.
  3. Origin match: Load `PAYLOAD.origin`; fail if mismatch.
  4. Validate & verify: `cred.validate()?; cred.verify(deps.as_ref())?`.
  5. Emits "auth_req" event.
- **`on_auth_track`**: Placeholder; extend for challenge blacklisting.
- **`on_auth_confirm`**: Placeholder; extend for post-exec (e.g., query balances pre/post via `req.events`).
- **`on_hooks`**: TODO; for epoch/rotation (e.g., refresh challenges).

### `Verifiable` Impl for `PasskeyCredential`

Implements the `saa::Verifiable` trait for credential handling:

- **`message(&self) -> Cow<[u8]>`**: Extracts challenge from `client_data` (base64url → base64 → bytes). Used as sig message.
- **`validate(&self) -> Result<(), AuthError>`**:
  - Checks non-empty: `signature`, `authenticator_data` (≥37 bytes), `client_data.challenge`, `message()`.
  - Ensures `client_data.ty == "webauthn.get"`.
  - Errors: `CredentialError::MissingData` or `InvalidProperty`.
- **`verify(&self, deps: Deps) -> Result<CredentialInfo, AuthError>`**:
  - Computes `data_hash()` (likely SHA256 of `authenticator_data || message()`; see `saa` docs).
  - Verifies via `secp256r1_verify(data_hash, signature, pubkey)`.
    - **CosmWasm**: `deps.api.secp256r1_verify` (if not `no_api_r1`).
    - **Native**: `saa_crypto::secp256r1_verify`.
  - Normalizes high-S sigs.
  - Returns `CredentialInfo` (extensions/address/HRP; here, minimal for passkey).

### Crypto Primitive: `secp256r1_verify`

- **Inputs**: `message_hash: &[u8]` (32-byte SHA256), `signature: &[u8]` (64-byte compact), `public_key: &[u8]` (SEC1, 33/65 bytes).
- **Process**:
  1. Validate/read hash, sig, pubkey.
  2. Build `Identity256` digest from hash.
  3. Normalize sig to low-S.
  4. Recover `VerifyingKey` from pubkey bytes.
  5. Verify: `public_key.verify_digest(digest, &signature)`.
- **Output**: `CryptoResult<bool>` (true if valid).
- **Notes**: Cosmos-format compat; rejects high-S by default (normalized for malleability resistance).

## Client-Side Generation (Rust/WebAuthn)

Generate `PasskeyCredential` off-chain for tx inclusion. Use browser WebAuthn API (JS) or Rust crates like `webauthn-rs` for native. Embed serialized JSON in `AuthenticationRequest.signature`.

### Dependencies (Client Crate)

```toml
[dependencies]
webauthn-rs = "0.4"  # For passkey gen/verify.
base64 = "0.21"
serde_json = "1.0"
cosmos-sdk-proto = "0.20"  # Tx building.
```

### Example: Generate Credential

```rust
use webauthn_rs::prelude::*;
use webauthn_rs::webauthn::WebAuthn;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::to_string;

// Assume dApp origin/challenge from contract query.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let origin = Url::parse("https://my-dapp.com")?;
    let webauthn = WebAuthn::new(&origin, &origin, None, None, None)?;

    // Step 1: Create challenge (from contract or random).
    let challenge = generate_challenge::<Sha256>(None)?;  // 32 bytes.
    let client_challenge = URL_SAFE_NO_PAD.encode(challenge);

    // Step 2: Request assertion (sim; in browser: navigator.credentials.get()).
    let public_key = /* Existing registered key */;
    let credential = webauthn.start_passkey_assertion(
        &public_key,
        PasskeyAuthenticationOptions::default().challenge(challenge),
    ).await?;

    // Step 3: Finish assertion (simulate response).
    let assertion = /* From device/browser */;
    let webauthn_assertion = webauthn.finish_passkey_assertion(
        &assertion,
        &credential,
        /* rp_id, origin, etc. */
    ).await?;

    // Step 4: Build Credential.
    let client_data = ClientData {
        ty: "webauthn.get".to_string(),
        challenge: client_challenge,
        origin: origin.to_string(),
    };
    let cred = PasskeyCredential {
        id: webauthn_assertion.credential_id.to_base64_url(),
        signature: webauthn_assertion.signature.clone(),  // Binary.
        authenticator_data: webauthn_assertion.authenticator_data.clone(),
        client_data,
        user_handle: None,
        pubkey: Some(webauthn_assertion.pubkey().to_bytes()),  // Optional.
    };

    // Step 5: Serialize for tx.
    let cred_json = to_string(&cred)?;
    let auth_req = AuthenticationRequest {
        message: /* encoded msg */,
        signers: vec!["cosmos1...".to_string()],
        signature: cred_json.as_bytes().to_vec(),  // JSON bytes.
        // ...
    };
    // Broadcast with selected_authenticators pointing to passkey ID.

    println!("Credential JSON: {}", cred_json);
    Ok(())
}
```

- **Browser Integration**: Use `navigator.credentials.get({ publicKey: options })` to get raw response; parse to `PasskeyCredential`.
- **Registration**: Similar flow with `create()` for initial key gen; store pubkey in contract via query or separate msg.
- **Notes**: Challenge must match contract's (query for fresh one). Origin binds to dApp.

## Adding as Authenticator

1. **Instantiate Contract**: Deploy with `InstantiateMsg { owner: Some(addr), payload: PasskeyPayload { origin: Some("https://my-dapp.com".to_string()) } }`.
2. **Add to Account** (via CLI/JS):

   ```bash
   terpd tx smart-account add-authenticator <account> CosmWasmAuthenticatorV1 '{"contract": "<passkey_addr>", "params": {"origin": "https://my-dapp.com", "challenge": "base64_challenge"}}' --from <user>
   ```

   - `params`: JSON `PasskeyPayload`; stores in `PAYLOAD`.
3. **Tx Usage**: In `TxExtension.selected_authenticators`, specify passkey's global ID (from query). Include credential JSON in `signature`.
4. **Remove**: `MsgRemoveAuthenticator` with ID; triggers `on_auth_removed`.

## Extending & Testing

- **Storage**: Use `PAYLOAD` (singleton) for origin/challenges; add `Item<'static, HashSet<String>>` for used challenges in `on_auth_track`.
- **Errors**: Handle `AuthError::Signature`, `CredentialError`.
- **Tests**: Use `cw-multi-test`; mock `Deps` with `MockApi` for `secp256r1_verify`. Generate creds with `webauthn-rs`.
- **Gas**: Sig verify ~20k gas; optimize by storing pubkeys.

For full `saa` docs, see crate. Contribute to Terp!

## Demo: 2-Step Auth - Bluetooth + FaceID
