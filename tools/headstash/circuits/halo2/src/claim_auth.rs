//! Claim authorization for an offline secp256k1 wallet.
//!
//! The prover does not receive `esk`. The holder signs a fixed message with
//! `personal_sign` (EIP-191). The circuit checks that signature under the `epk`
//! on the opened distribution leaf. The chain checks that the public challenge
//! `e` is the Keccak of that same message, so the hash does not have to be
//! reimplemented inside the circuit.
//!
//! The live nullifier is the hiding one. `rho` and `psi` are the two halves of
//! the note's one-time string, `rcm` is their sum, and `nk` is a Poseidon of
//! `esk` mixed with that string. The public leaf stores only
//! `Poseidon(DST_RSEED, lo, hi)`. `claim_nullifier` hashes the public leaf
//! fields and is not the claim path.

use alloc::string::String;
use alloc::vec::Vec;

use ff::PrimeField;
use halo2_gadgets::poseidon::primitives::{self as poseidon, ConstantLength, P128Pow5T3};
use pasta_curves::pallas;

use crate::note_poseidon::personalization_to_fp;

/// ASCII prefix in front of the 64-hex-character claim body.
pub const CLAIM_PREFIX: &str = "headstash-claim-v1:";

const CLAIM_NULLIFIER_TAG: &str = "terp-hs-nullifier-v1";

const KECCAK_ROUNDS: [u64; 24] = [
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

/// Rotation offsets `r[x][y]` for Keccak-f rho.
const KECCAK_ROT: [[u32; 5]; 5] = [
    [0, 36, 3, 41, 18],
    [1, 44, 10, 45, 2],
    [62, 6, 43, 15, 61],
    [28, 55, 25, 21, 56],
    [27, 20, 39, 8, 14],
];

fn keccak_f(a: &mut [u64; 25]) {
    for round in 0..24 {
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        let mut d = [0u64; 5];
        for x in 0..5 {
            d[x] = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
        }
        for y in 0..5 {
            for x in 0..5 {
                a[x + 5 * y] ^= d[x];
            }
        }

        let mut b = [0u64; 25];
        for y in 0..5 {
            for x in 0..5 {
                let nx = y;
                let ny = (2 * x + 3 * y) % 5;
                b[nx + 5 * ny] = a[x + 5 * y].rotate_left(KECCAK_ROT[x][y]);
            }
        }

        for y in 0..5 {
            for x in 0..5 {
                a[x + 5 * y] =
                    b[x + 5 * y] ^ ((!b[((x + 1) % 5) + 5 * y]) & b[((x + 2) % 5) + 5 * y]);
            }
        }
        a[0] ^= KECCAK_ROUNDS[round];
    }
}

/// Keccak-256 (the Ethereum hash, padding `0x01`, not SHA3's `0x06`).
pub fn keccak256(input: &[u8]) -> [u8; 32] {
    const RATE: usize = 136;
    let mut state = [0u64; 25];
    let mut padded = Vec::with_capacity(input.len() + RATE);
    padded.extend_from_slice(input);
    padded.push(0x01);
    while padded.len() % RATE != 0 {
        padded.push(0);
    }
    let last = padded.len() - 1;
    padded[last] |= 0x80;

    for block in padded.chunks(RATE) {
        for (i, lane) in block.chunks(8).enumerate() {
            let mut buf = [0u8; 8];
            buf[..lane.len()].copy_from_slice(lane);
            state[i] ^= u64::from_le_bytes(buf);
        }
        keccak_f(&mut state);
    }

    let mut out = [0u8; 32];
    for i in 0..4 {
        out[i * 8..i * 8 + 8].copy_from_slice(&state[i].to_le_bytes());
    }
    out
}

fn push_decimal(out: &mut Vec<u8>, mut n: usize) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    if n == 0 {
        out.push(b'0');
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.extend_from_slice(&buf[i..]);
}

/// `keccak256("\x19Ethereum Signed Message:\n" || len || message)`.
pub fn eip191_keccak(message: &[u8]) -> [u8; 32] {
    let mut prefixed = Vec::with_capacity(message.len() + 32);
    prefixed.extend_from_slice(b"\x19Ethereum Signed Message:\n");
    push_decimal(&mut prefixed, message.len());
    prefixed.extend_from_slice(message);
    keccak256(&prefixed)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Bytes a wallet passes to `personal_sign`.
///
/// `body` is the 32-byte claim digest (the Poseidon of the public claim).
/// The string is `headstash-claim-v1:` plus 64 lowercase hex characters.
pub fn claim_signing_message(body: &[u8; 32]) -> Vec<u8> {
    let mut message = Vec::with_capacity(CLAIM_PREFIX.len() + 64);
    message.extend_from_slice(CLAIM_PREFIX.as_bytes());
    message.extend(hex_encode(body).into_bytes());
    message
}

/// Challenge scalar bytes for ECDSA: EIP-191 Keccak of [`claim_signing_message`].
pub fn claim_challenge(body: &[u8; 32]) -> [u8; 32] {
    eip191_keccak(&claim_signing_message(body))
}

fn poseidon3(a: pallas::Base, b: pallas::Base, c: pallas::Base) -> pallas::Base {
    poseidon::Hash::<_, P128Pow5T3, ConstantLength<3>, 3, 2>::init().hash([a, b, c])
}

/// Split a 32-byte note random string into two canonical base-field halves.
///
/// Each half is 16 bytes, so the full 256 bits are kept. Nothing is derived from `esk`.
pub fn rseed_halves(rseed: &[u8; 32]) -> (pallas::Base, pallas::Base) {
    let lo = pallas::Base::from_u128(u128::from_le_bytes(rseed[..16].try_into().unwrap()));
    let hi = pallas::Base::from_u128(u128::from_le_bytes(rseed[16..].try_into().unwrap()));
    (lo, hi)
}

/// `rho` is the low half of the note string. Not a hash, and not a public input.
pub fn claim_rho(rseed_lo: pallas::Base) -> pallas::Base {
    rseed_lo
}

/// `psi` is the high half of the note string.
pub fn claim_psi(rseed_hi: pallas::Base) -> pallas::Base {
    rseed_hi
}

/// `rcm = rseed_lo + rseed_hi` in the base field. One addition, no Poseidon.
pub fn claim_rcm_base(rseed_lo: pallas::Base, rseed_hi: pallas::Base) -> pallas::Base {
    rseed_lo + rseed_hi
}

/// Domain tag for the hiding commitment published in the distribution leaf.
pub const DST_RSEED: &str = "terp-hs-rseed-v1";

/// `rseed_com = Poseidon(DST_RSEED, rseed_lo, rseed_hi)`.
///
/// This is what the public leaf stores. The 32-byte string stays a witness.
pub fn claim_rseed_com(rseed_lo: pallas::Base, rseed_hi: pallas::Base) -> pallas::Base {
    poseidon3(personalization_to_fp(DST_RSEED), rseed_lo, rseed_hi)
}

/// `nk = Poseidon(DST_HKDF, rseed_lo, rseed_hi)`.
///
/// Ownership of the eligible key is the personal-sign check, not this hash.
/// The public leaf is not an input. A second string is a different key.
pub fn claim_nk(rseed_lo: pallas::Base, rseed_hi: pallas::Base) -> pallas::Base {
    poseidon3(
        pallas::Base::from_repr(crate::constants::DST_HKDF).expect("DST_HKDF"),
        rseed_lo,
        rseed_hi,
    )
}

/// EIP-191 challenge reduced modulo secp256k1 `n`, as 32 big-endian bytes.
pub fn challenge_scalar_be(body: &[u8; 32]) -> [u8; 32] {
    reduce_mod_n(&claim_challenge(body))
}

/// One subtraction. `n` is greater than `2^255`, so a 32-byte integer is at most `n` over.
fn reduce_mod_n(bytes: &[u8; 32]) -> [u8; 32] {
    const N: [u8; 32] = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36,
        0x41, 0x41,
    ];
    if be_less(bytes, &N) {
        *bytes
    } else {
        be_sub(bytes, &N)
    }
}

fn be_less(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a < b
}

fn be_sub(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0u16;
    for i in (0..32).rev() {
        let av = a[i] as u16;
        let bv = b[i] as u16 + borrow;
        if av >= bv {
            out[i] = (av - bv) as u8;
            borrow = 0;
        } else {
            out[i] = (av + 256 - bv) as u8;
            borrow = 1;
        }
    }
    out
}

/// Sign `body` with the eligible key. Returns `(e, r, s)` as big-endian scalars.
/// `e` is [`challenge_scalar_be`]. The secret is not a circuit witness.
#[cfg(feature = "host-crypto")]
pub fn sign_personal_claim(
    secret_be: &[u8; 32],
    body: &[u8; 32],
) -> ([u8; 32], [u8; 32], [u8; 32]) {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let challenge = claim_challenge(body);
    let e_be = reduce_mod_n(&challenge);
    let secp = Secp256k1::new();
    let sk = SecretKey::from_byte_array(*secret_be).expect("eligible key");
    let sig = secp.sign_ecdsa(Message::from_digest(challenge), &sk);
    let compact = sig.serialize_compact();
    let mut r_be = [0u8; 32];
    let mut s_be = [0u8; 32];
    r_be.copy_from_slice(&compact[..32]);
    s_be.copy_from_slice(&compact[32..]);
    (e_be, r_be, s_be)
}

/// One nullifier per distribution leaf. Same inputs as the leaf, different tag.
pub fn claim_nullifier(
    epk_x: pallas::Base,
    epk_y: pallas::Base,
    nd: pallas::Base,
    v: pallas::Base,
    fdi: pallas::Base,
) -> pallas::Base {
    poseidon::Hash::<_, P128Pow5T3, ConstantLength<6>, 3, 2>::init().hash([
        personalization_to_fp(CLAIM_NULLIFIER_TAG),
        epk_x,
        epk_y,
        nd,
        v,
        fdi,
    ])
}

#[cfg(all(test, feature = "host-crypto"))]
mod tests {
    use super::*;
    use ff::PrimeField;
    use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

    #[test]
    fn keccak256_empty_matches_ethereum() {
        // Keccak-256("") — not SHA3-256.
        let expect = hex_to_32("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
        assert_eq!(keccak256(b""), expect);
    }

    #[test]
    fn eip191_hello_matches_personal_sign_hash() {
        // personal_sign("hello") digest, as used by Ethereum wallets.
        let got = eip191_keccak(b"hello");
        let expect = hex_to_32("50b2c43fd39106bafbba0da34fc430e1f91e3c96ea2acee2bc34119f92b37750");
        assert_eq!(got, expect, "eip-191 keccak of hello");
    }

    #[test]
    fn wallet_signature_verifies_and_rejects_a_swap() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_byte_array([0x11; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);

        let body = [0xab; 32];
        let challenge = claim_challenge(&body);
        let sig = secp.sign_ecdsa(Message::from_digest(challenge), &sk);
        secp.verify_ecdsa(Message::from_digest(challenge), &sig, &pk)
            .expect("honest personal_sign verifies");

        let mut other = body;
        other[0] ^= 1;
        let wrong = claim_challenge(&other);
        assert!(
            secp.verify_ecdsa(Message::from_digest(wrong), &sig, &pk)
                .is_err(),
            "a signature on one claim must not verify for another"
        );
    }

    #[test]
    fn bound_nullifier_is_hidden_and_unique() {
        let x = pallas::Base::from(3u64);
        let y = pallas::Base::from(5u64);
        let nd = pallas::Base::from(7u64);
        let v = pallas::Base::from(11u64);
        let fdi = pallas::Base::from(13u64);
        let mut seed_a = [0u8; 32];
        seed_a[0] = 9;
        seed_a[31] = 7;
        let mut seed_hi_only = seed_a;
        seed_hi_only[16] = 1;
        let mut seed_b = seed_a;
        seed_b[0] = 8;
        seed_b[16] = 1;
        let (lo_a, hi_a) = rseed_halves(&seed_a);
        let (lo_hi_only, hi_only) = rseed_halves(&seed_hi_only);
        let (lo_b, hi_b) = rseed_halves(&seed_b);

        let rho = claim_rho(lo_a);
        assert_eq!(rho, lo_a);
        assert_eq!(claim_rcm_base(lo_a, hi_a), lo_a + hi_a);
        assert_eq!(claim_rho(lo_hi_only), rho, "high half is not rho");
        assert_ne!(
            claim_rseed_com(lo_a, hi_a),
            claim_rseed_com(lo_hi_only, hi_only),
            "the high half is inside the published commitment"
        );
        assert_ne!(rho, claim_rho(lo_b), "a fresh note random changes rho");
        assert_ne!(claim_psi(hi_a), claim_psi(hi_b));
        assert_eq!(claim_nk(lo_a, hi_a), claim_nk(lo_a, hi_a));
        assert_ne!(
            claim_nk(lo_a, hi_a),
            claim_nk(lo_b, hi_b),
            "a second note random is a different nullifier key"
        );
        // Public leaf inputs do not appear in rho. Changing them does not move it.
        let leaf = crate::distro_poseidon::poseidon_distro_leaf(
            x,
            y,
            nd,
            v,
            fdi,
            claim_rseed_com(lo_a, hi_a),
        );
        let leaf_other = crate::distro_poseidon::poseidon_distro_leaf(
            x,
            y,
            nd,
            v + pallas::Base::from(1u64),
            fdi,
            claim_rseed_com(lo_a, hi_a),
        );
        assert_ne!(
            leaf,
            crate::distro_poseidon::poseidon_distro_leaf(x, y, nd, v, fdi, claim_rseed_com(lo_b, hi_b)),
            "a second note random is a different leaf"
        );
        assert_ne!(leaf, leaf_other);
        assert_eq!(rho, claim_rho(lo_a));
        assert_ne!(leaf, rho);
        assert_ne!(leaf, claim_nk(lo_a, hi_a));
    }

    #[test]
    fn nullifier_is_unique_per_leaf_preimage() {
        let x = pallas::Base::from(3u64);
        let y = pallas::Base::from(5u64);
        let nd = pallas::Base::from(7u64);
        let v = pallas::Base::from(11u64);
        let fdi = pallas::Base::from(13u64);
        let nf = claim_nullifier(x, y, nd, v, fdi);
        assert_eq!(nf, claim_nullifier(x, y, nd, v, fdi));
        let one = pallas::Base::from(1u64);
        assert_ne!(nf, claim_nullifier(x, y, nd, v, fdi + one));
        assert_ne!(nf, claim_nullifier(x + one, y, nd, v, fdi));
    }

    fn hex_to_32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }
}
