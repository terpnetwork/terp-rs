//! Bytes the sidecar signs and the contract checks.
//!
//! The Flock circuit does not see the MetaMask signature. The sidecar recovers
//! the leaf key from `personal_sign` and only then folds the payout. The chain
//! message carries the signed batch, not the Ligerito proof.

use k256::ecdsa::signature::{Signer, Verifier};
use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};

const BATCH_DOMAIN: &[u8] = b"terp-hs-batch-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchPayout {
    pub destination: String,
    pub amount: u128,
    pub nullifier: [u8; 32],
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("ethereum signature must be 65 bytes")]
    SignatureLength,
    #[error("ethereum signature did not recover")]
    Recover,
    #[error("batch signature was rejected")]
    BatchSignature,
}

/// Canonical batch the sidecar signs and the collection contract rebuilds.
pub fn canonical_batch(root: &[u8; 32], rejected: u32, payouts: &[BatchPayout]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + payouts.len() * 64);
    out.extend_from_slice(BATCH_DOMAIN);
    out.extend_from_slice(root);
    out.extend_from_slice(&rejected.to_le_bytes());
    out.extend_from_slice(&(payouts.len() as u32).to_le_bytes());
    for payout in payouts {
        let dest = payout.destination.as_bytes();
        out.extend_from_slice(&(dest.len() as u32).to_le_bytes());
        out.extend_from_slice(dest);
        out.extend_from_slice(&payout.amount.to_le_bytes());
        out.extend_from_slice(&payout.nullifier);
    }
    out
}

fn batch_key(secret: &[u8; 32]) -> Result<SigningKey, WireError> {
    SigningKey::from_bytes(secret.into()).map_err(|_| WireError::Recover)
}

/// Compressed SEC1 public key, 33 bytes.
pub fn batch_public(secret: &[u8; 32]) -> [u8; 33] {
    let key = batch_key(secret).expect("batch secret");
    let point = key.verifying_key().to_encoded_point(true);
    let mut out = [0u8; 33];
    out.copy_from_slice(point.as_bytes());
    out
}

pub fn sign_batch(secret: &[u8; 32], message: &[u8]) -> [u8; 64] {
    let key = batch_key(secret).expect("batch secret");
    let sig: Signature = key.sign(message);
    sig.to_bytes().into()
}

pub fn verify_batch(public: &[u8; 33], message: &[u8], signature: &[u8; 64]) -> Result<(), WireError> {
    let key = VerifyingKey::from_sec1_bytes(public).map_err(|_| WireError::BatchSignature)?;
    let sig = Signature::from_slice(signature).map_err(|_| WireError::BatchSignature)?;
    key.verify(message, &sig).map_err(|_| WireError::BatchSignature)
}

/// EIP-191 `personal_sign` digest.
pub fn personal_sign_hash(message: &[u8]) -> [u8; 32] {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut out = Vec::with_capacity(prefix.len() + message.len());
    out.extend_from_slice(prefix.as_bytes());
    out.extend_from_slice(message);
    keccak256(&out)
}

/// Message the holder signs to bind a Bandersnatch key to an eligible address.
pub fn registration_message(eligible_address: &str, y_hex: &str) -> Vec<u8> {
    format!(
        "headstash-register-v1:{}:{y_hex}",
        eligible_address.trim().trim_start_matches("0x").to_ascii_lowercase()
    )
    .into_bytes()
}

/// Sign `message` as MetaMask `personal_sign`. Returns `r || s || v` with `v` in {27, 28}.
pub fn sign_personal(secret: &[u8; 32], message: &[u8]) -> Result<[u8; 65], WireError> {
    let key = batch_key(secret)?;
    let hash = personal_sign_hash(message);
    let (sig, id) = key.sign_prehash_recoverable(&hash).map_err(|_| WireError::Recover)?;
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&sig.to_bytes());
    out[64] = 27 + id.to_byte();
    Ok(out)
}

fn recover_verifying_key(message: &[u8], signature: &[u8]) -> Result<VerifyingKey, WireError> {
    if signature.len() != 65 {
        return Err(WireError::SignatureLength);
    }
    let v = signature[64];
    let rec = if v >= 27 { v - 27 } else { v };
    let id = RecoveryId::try_from(rec).map_err(|_| WireError::Recover)?;
    let sig = Signature::from_slice(&signature[..64]).map_err(|_| WireError::Recover)?;
    let hash = personal_sign_hash(message);
    VerifyingKey::recover_from_prehash(&hash, &sig, id).map_err(|_| WireError::Recover)
}

/// X coordinate of the key that signed `message`. This is the leaf key check.
pub fn recover_pubkey_x(message: &[u8], signature: &[u8]) -> Result<[u8; 32], WireError> {
    let vk = recover_verifying_key(message, signature)?;
    let point = vk.to_encoded_point(false);
    let bytes = point.as_bytes();
    let mut x = [0u8; 32];
    x.copy_from_slice(&bytes[1..33]);
    Ok(x)
}

/// Last 20 bytes of Keccak(uncompressed pubkey without the 0x04 prefix).
pub fn recover_address(message: &[u8], signature: &[u8]) -> Result<[u8; 20], WireError> {
    let vk = recover_verifying_key(message, signature)?;
    let point = vk.to_encoded_point(false);
    let hash = keccak256(&point.as_bytes()[1..]);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    Ok(addr)
}

/// Keccak-256. Checked against the empty-string digest Ethereum uses.
pub fn keccak256(input: &[u8]) -> [u8; 32] {
    let mut state = [0u64; 25];
    let rate = 136;
    let mut offset = 0;
    while offset + rate <= input.len() {
        absorb_block(&mut state, &input[offset..offset + rate]);
        keccak_f(&mut state);
        offset += rate;
    }
    let mut block = [0u8; 136];
    let rest = &input[offset..];
    block[..rest.len()].copy_from_slice(rest);
    block[rest.len()] = 0x01;
    block[135] |= 0x80;
    absorb_block(&mut state, &block);
    keccak_f(&mut state);
    let mut out = [0u8; 32];
    for i in 0..4 {
        out[8 * i..8 * i + 8].copy_from_slice(&state[i].to_le_bytes());
    }
    out
}

fn absorb_block(state: &mut [u64; 25], block: &[u8]) {
    for i in 0..block.len() / 8 {
        let mut lane = [0u8; 8];
        lane.copy_from_slice(&block[8 * i..8 * i + 8]);
        state[i] ^= u64::from_le_bytes(lane);
    }
}

fn keccak_f(a: &mut [u64; 25]) {
    const RC: [u64; 24] = [
        0x0000_0000_0000_0001,
        0x0000_0000_0000_8082,
        0x8000_0000_0000_808A,
        0x8000_0000_8000_8000,
        0x0000_0000_0000_808B,
        0x0000_0000_8000_0001,
        0x8000_0000_8000_8081,
        0x8000_0000_0000_8009,
        0x0000_0000_0000_008A,
        0x0000_0000_0000_0088,
        0x0000_0000_8000_8009,
        0x0000_0000_8000_000A,
        0x0000_0000_8000_808B,
        0x8000_0000_0000_008B,
        0x8000_0000_0000_8089,
        0x8000_0000_0000_8003,
        0x8000_0000_0000_8002,
        0x8000_0000_0000_0080,
        0x0000_0000_0000_800A,
        0x8000_0000_8000_000A,
        0x8000_0000_8000_8081,
        0x8000_0000_0000_8080,
        0x0000_0000_8000_0001,
        0x8000_0000_8000_8008,
    ];
    const ROT: [u32; 24] = [
        1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
    ];
    const PIL: [usize; 24] = [
        10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1,
    ];
    for round in 0..24 {
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        let mut d = [0u64; 5];
        for x in 0..5 {
            d[x] = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
        }
        for i in 0..25 {
            a[i] ^= d[i % 5];
        }
        // Rho and pi walk one lane chain. `PIL` is the next index, not a
        // destination for lane `i + 1`.
        let mut t = a[1];
        for i in 0..24 {
            let j = PIL[i];
            let next = a[j];
            a[j] = t.rotate_left(ROT[i]);
            t = next;
        }
        let mut lane = [0u64; 5];
        for y in 0..5 {
            for x in 0..5 {
                lane[x] = a[x + 5 * y];
            }
            for x in 0..5 {
                a[x + 5 * y] = lane[x] ^ (!lane[(x + 1) % 5] & lane[(x + 2) % 5]);
            }
        }
        a[0] ^= RC[round];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keccak_empty_matches_ethereum() {
        assert_eq!(
            hex_encode(&keccak256(b"")),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn personal_sign_recovers_the_same_x_and_address() {
        let mut secret = [0u8; 32];
        secret[31] = 1;
        let message = b"headstash-claim-v1";
        let sig = sign_personal(&secret, message).unwrap();
        let addr = recover_address(message, &sig).unwrap();
        assert_eq!(
            hex_encode(&addr),
            "7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
        let x = recover_pubkey_x(message, &sig).unwrap();
        let other = sign_personal(&secret, b"other").unwrap();
        assert_ne!(recover_pubkey_x(message, &other).ok(), Some(x));
    }

    #[test]
    fn a_flipped_batch_byte_fails() {
        let secret = [3u8; 32];
        let public = batch_public(&secret);
        let payouts = [BatchPayout {
            destination: "terp1dest".into(),
            amount: 15,
            nullifier: [7u8; 32],
        }];
        let root = [1u8; 32];
        let msg = canonical_batch(&root, 0, &payouts);
        let sig = sign_batch(&secret, &msg);
        verify_batch(&public, &msg, &sig).unwrap();
        let mut bad = msg.clone();
        bad[10] ^= 1;
        assert!(verify_batch(&public, &bad, &sig).is_err());
    }

    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
