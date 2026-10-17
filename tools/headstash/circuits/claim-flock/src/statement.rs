//! The private claim both provers have to speak.
//!
//! Public words are the allocation root, the nullifier, and a hiding payout
//! commitment. The leaf key, the denomination, the amount, the destination,
//! and the signature are witnesses. `esk` is not a witness.
//!
//! Halo2 already checks the signature inside `prove_ecdsa_verify`:
//! `R = (e·s⁻¹)·G + (r·s⁻¹)·Q` and `R.x ≡ r (mod n)`. This module locks the
//! message that `e` is the challenge of. The Ligerito frontend in this repo
//! has a Blake3 gate and does not yet have that secp256k1 gate, so
//! [`crate::build_private`] enforces the public words and the leaf binding,
//! and does not yet constrain the signature.

use headstash_wire::{recover_pubkey_x, sign_personal};

/// ASCII prefix of the `personal_sign` message. The body is the hex of the
/// three public words, so a signature on any other string does not open this
/// claim.
pub const PRIVATE_SIGN_PREFIX: &str = "headstash-private-claim-v1:";

/// Message the holder signs. MetaMask applies the EIP-191 prefix on top.
pub fn private_sign_message(root: &[u32; 8], nullifier: &[u32; 8], payout_com: &[u32; 8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(PRIVATE_SIGN_PREFIX.len() + 64 * 3);
    body.extend_from_slice(PRIVATE_SIGN_PREFIX.as_bytes());
    push_hex(&mut body, root);
    push_hex(&mut body, nullifier);
    push_hex(&mut body, payout_com);
    body
}

/// The signature recovers `leaf_x` for this message. The same bytes under a
/// different message do not.
pub fn signature_opens_leaf(message: &[u8], signature: &[u8], leaf_x: &[u8; 32]) -> bool {
    recover_pubkey_x(message, signature).ok().as_ref() == Some(leaf_x)
}

fn push_hex(out: &mut Vec<u8>, words: &[u32; 8]) {
    for word in words {
        out.extend_from_slice(format!("{word:08x}").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_bound_to_the_three_public_words() {
        let secret = [7u8; 32];
        let root = [1u32; 8];
        let nullifier = [2u32; 8];
        let payout = [3u32; 8];
        let message = private_sign_message(&root, &nullifier, &payout);
        let signature = sign_personal(&secret, &message).unwrap();
        let leaf_x = recover_pubkey_x(&message, &signature).unwrap();
        assert!(signature_opens_leaf(&message, &signature, &leaf_x));

        let other = private_sign_message(&root, &nullifier, &[4u32; 8]);
        assert!(!signature_opens_leaf(&other, &signature, &leaf_x));
    }
}
