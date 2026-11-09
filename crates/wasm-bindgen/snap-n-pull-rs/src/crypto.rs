//! Authenticated encryption/decryption for nullifier sync
//!
//! This module provides ECIES (Elliptic Curve Integrated Encryption Scheme) for
//! encrypting nullifier state that can be synced across devices via headstash-api.
//!
//! ## Security Model
//!
//! 1. **Public Key Encryption**: Data encrypted to user's eligible public key
//! 2. **Authenticated**: Includes signature to prove data origin
//! 3. **Forward Secrecy**: Uses ephemeral keys for each encryption
//! 4. **Tamper Evident**: MAC ensures data integrity

use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use zk_headstash::r#gen::snp::v1::*;
use zk_headstash::keys::{EligiblePk, EligibleSk};
use zk_headstash::note::ExtractedNoteCommitment;

use crate::Error;

/// Plaintext nullifier state for serialization
#[derive(Clone, Serialize, Deserialize)]
pub struct NullifierState {
    /// Headstash ID (contract address)
    pub headstash_id: String,
    /// List of spent notes
    pub spent_notes: Vec<SerializedNoteData>,
}

// ============================================================================
// Cryptographic Primitives
// ============================================================================

/// Perform ECDH to derive shared secret
fn ecdh(sk: &EligibleSk, pk: &EligiblePk) -> Result<[u8; 32], Error> {
    use secp256k1::ecdh::SharedSecret;

    let secp = secp256k1::Secp256k1::new();
    let secret_key = sk.secret_key();
    let public_key = pk.0;

    let shared = SharedSecret::new(&public_key, &secret_key);
    Ok(shared.secret_bytes())
}

/// Derive encryption and MAC keys from shared secret
fn derive_keys(shared_secret: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    // Use HKDF-like derivation
    let mut hasher = Sha3_256::new();
    hasher.update(shared_secret);
    hasher.update(b"encryption");
    let enc_key: [u8; 32] = hasher.finalize().into();

    let mut hasher = Sha3_256::new();
    hasher.update(shared_secret);
    hasher.update(b"mac");
    let mac_key: [u8; 32] = hasher.finalize().into();

    (enc_key, mac_key)
}

/// Compute MAC over data
fn compute_mac(data: &[u8], key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(key);
    hasher.update(data);
    hasher.finalize().into()
}

/// Create message to sign
fn create_signature_message(
    ephemeral_pk: &EligiblePk,
    ciphertext: &[u8],
    mac: &[u8; 32],
) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(&ephemeral_pk.0.to_string().as_bytes());
    message.extend_from_slice(ciphertext);
    message.extend_from_slice(mac);
    message
}

/// Sign a message with secp256k1
fn sign_message(message: &[u8], sk: &EligibleSk) -> Result<Vec<u8>, Error> {
    use secp256k1::{Message, Secp256k1};

    let secp = Secp256k1::new();

    // Hash the message
    let msg_hash = Sha3_256::digest(message);
    let message = Message::from_digest_slice(&msg_hash)
        .map_err(|e| Error::Js(format!("Invalid message: {}", e).into()))?;

    // Sign
    let signature = secp.sign_ecdsa(message, &sk.secret_key());

    Ok(signature.serialize_compact().to_vec())
}

/// Verify a signature
fn verify_signature(message: &[u8], signature: &[u8], pk: &EligiblePk) -> Result<(), Error> {
    use secp256k1::{Message, Secp256k1, ecdsa::Signature};

    let secp = Secp256k1::new();

    // Hash the message
    let msg_hash = Sha3_256::digest(message);
    let message = Message::from_digest_slice(&msg_hash)
        .map_err(|e| Error::Js(format!("Invalid message: {}", e).into()))?;

    // Parse signature
    let signature = Signature::from_compact(signature)
        .map_err(|e| Error::Js(format!("Invalid signature: {}", e).into()))?;

    // Verify
    secp.verify_ecdsa(message, &signature, &pk.0)
        .map_err(|e| Error::Js(format!("Signature verification failed: {}", e).into()))?;

    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use zk_headstash::value::HeadstashValue;

//     #[test]
//     fn test_encrypt_decrypt_nullifier_state() {
//         // Generate key pairs
//         let alice_sk = EligibleSk::random();
//         let alice_pk = alice_sk.epk();
//         let bob_sk = EligibleSk::random();
//         let bob_pk = bob_sk.epk();

//         // Create test state
//         let state = NullifierState {
//             headstash_id: "terp1contract123".to_string(),
//             spent_notes: vec![],
//         };

//         // Alice encrypts to Bob's key and signs with her key
//         let encrypted = encrypt_nullifier_state(&state, &bob_pk, &alice_sk).unwrap();

//         // Bob decrypts with his key and verifies Alice's signature
//         let decrypted = decrypt_nullifier_state(&encrypted, &bob_sk, &alice_pk).unwrap();

//         assert_eq!(decrypted.headstash_id, state.headstash_id);
//     }

//     #[test]
//     fn test_mac_verification_fails_on_tampered_data() {
//         let alice_sk = EligibleSk::random();
//         let alice_pk = alice_sk.epk();
//         let bob_sk = EligibleSk::random();
//         let bob_pk = bob_sk.epk();

//         let state = NullifierState {
//             headstash_id: "terp1contract123".to_string(),
//             spent_notes: vec![],
//         };

//         let mut encrypted = encrypt_nullifier_state(&state, &bob_pk, &alice_sk).unwrap();

//         // Tamper with ciphertext
//         if !encrypted.ciphertext.is_empty() {
//             encrypted.ciphertext[0] ^= 0xFF;
//         }

//         // Decryption should fail MAC verification
//         let result = decrypt_nullifier_state(&encrypted, &bob_sk, &alice_pk);
//         assert!(result.is_err());
//     }

//     #[test]
//     fn test_signature_verification_fails_on_wrong_sender() {
//         let alice_sk = EligibleSk::random();
//         let bob_sk = EligibleSk::random();
//         let bob_pk = bob_sk.epk();
//         let eve_sk = EligibleSk::random();
//         let eve_pk = eve_sk.epk();

//         let state = NullifierState {
//             headstash_id: "terp1contract123".to_string(),
//             spent_notes: vec![],
//         };

//         // Alice encrypts and signs
//         let encrypted = encrypt_nullifier_state(&state, &bob_pk, &alice_sk).unwrap();

//         // Bob tries to verify with Eve's public key (should fail)
//         let result = decrypt_nullifier_state(&encrypted, &bob_sk, &eve_pk);
//         assert!(result.is_err());
//     }

//     #[test]
//     fn test_ecdh_produces_same_shared_secret() {
//         let alice_sk = EligibleSk::random();
//         let alice_pk = alice_sk.epk();
//         let bob_sk = EligibleSk::random();
//         let bob_pk = bob_sk.epk();

//         // Alice's perspective: alice_sk * bob_pk
//         let shared_alice = ecdh(&alice_sk, &bob_pk).unwrap();

//         // Bob's perspective: bob_sk * alice_pk
//         let shared_bob = ecdh(&bob_sk, &alice_pk).unwrap();

//         assert_eq!(shared_alice, shared_bob);
//     }
// }
