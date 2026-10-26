//! Noise_XX_25519_ChaChaPoly_SHA256, matching crates/kms/signing/bluetooth.
//!
//! Message 1 is the initiator ephemeral key and nothing else. Static keys
//! travel only inside later ciphertext.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

const PROTOCOL: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

pub const HIDDEN_VECTOR_MSG1: &str =
    "5dfedd3b6bd47f6fa28ee15d969d5bb0ea53774d488bdaf9df1c6e0124b3ef22";

pub fn advertisement(random: &[u8; 16]) -> [u8; 16] {
    *random
}

pub fn public_from_seed(seed: [u8; 32]) -> [u8; 32] {
    let secret = StaticSecret::from(seed);
    PublicKey::from(&secret).to_bytes()
}

pub fn handshake_message1(static_seed: [u8; 32], ephemeral_seed: [u8; 32]) -> [u8; 32] {
    let _static = StaticSecret::from(static_seed);
    let eph = StaticSecret::from(ephemeral_seed);
    PublicKey::from(&eph).to_bytes()
}

struct Sym {
    k: Option<[u8; 32]>,
    n: u64,
}

impl Sym {
    fn encrypt(&mut self, ad: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let Some(k) = self.k else {
            return plaintext.to_vec();
        };
        let cipher = ChaCha20Poly1305::new((&k).into());
        let nonce = nonce(self.n);
        self.n += 1;
        cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: ad,
                },
            )
            .expect("encrypt")
    }

    fn decrypt(&mut self, ad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, &'static str> {
        let Some(k) = self.k else {
            return Ok(ciphertext.to_vec());
        };
        let cipher = ChaCha20Poly1305::new((&k).into());
        let nonce = nonce(self.n);
        self.n += 1;
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext,
                    aad: ad,
                },
            )
            .map_err(|_| "decrypt")
    }
}

fn nonce(n: u64) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[4..].copy_from_slice(&n.to_le_bytes());
    out
}

struct Hs {
    ck: Vec<u8>,
    h: Vec<u8>,
    cs: Sym,
    s: StaticSecret,
    s_pub: [u8; 32],
    e: Option<StaticSecret>,
    e_pub: [u8; 32],
    rs: Option<[u8; 32]>,
    re: Option<[u8; 32]>,
}

fn new_hs(static_seed: [u8; 32]) -> Hs {
    let sum = Sha256::digest(PROTOCOL.as_bytes());
    let s = StaticSecret::from(static_seed);
    let s_pub = PublicKey::from(&s).to_bytes();
    Hs {
        ck: sum.to_vec(),
        h: sum.to_vec(),
        cs: Sym { k: None, n: 0 },
        s,
        s_pub,
        e: None,
        e_pub: [0; 32],
        rs: None,
        re: None,
    }
}

impl Hs {
    fn mix_hash(&mut self, data: &[u8]) {
        let mut mac = Sha256::new();
        mac.update(&self.h);
        mac.update(data);
        self.h = mac.finalize().to_vec();
    }

    fn mix_key(&mut self, ikm: &[u8]) {
        let (ck, temp) = hkdf2(&self.ck, ikm);
        self.ck = ck;
        self.cs.k = Some(temp);
        self.cs.n = 0;
    }

    fn encrypt_and_hash(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let ct = self.cs.encrypt(&self.h, plaintext);
        self.mix_hash(&ct);
        ct
    }

    fn decrypt_and_hash(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, &'static str> {
        let ad = self.h.clone();
        let pt = self.cs.decrypt(&ad, ciphertext)?;
        self.mix_hash(ciphertext);
        Ok(pt)
    }
}

fn hkdf2(ck: &[u8], ikm: &[u8]) -> (Vec<u8>, [u8; 32]) {
    let temp = hmac_sha(ck, ikm);
    let one = hmac_sha(&temp, &[0x01]);
    let mut two_in = one.clone();
    two_in.push(0x02);
    let two = hmac_sha(&temp, &two_in);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&two);
    (one, arr)
}

fn hmac_sha(key: &[u8], data: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    let mut mac = sha2::Sha256::new();
    // HMAC
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let d = Sha256::digest(key);
        k[..32].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    mac.update(ipad);
    mac.update(data);
    let inner = mac.finalize();
    let mut mac = Sha256::new();
    mac.update(opad);
    mac.update(inner);
    mac.finalize().to_vec()
}

fn dh(secret: &StaticSecret, pub_bytes: &[u8; 32]) -> [u8; 32] {
    let pk = PublicKey::from(*pub_bytes);
    secret.diffie_hellman(&pk).to_bytes()
}

pub fn vector_messages() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let i_static = [0x01u8; 32];
    let r_static = [0x02u8; 32];
    let i_eph = [0x03u8; 32];
    let r_eph = [0x04u8; 32];

    let mut init = new_hs(i_static);
    let e = StaticSecret::from(i_eph);
    init.e_pub = PublicKey::from(&e).to_bytes();
    init.e = Some(e);
    let e_pub = init.e_pub;
    init.mix_hash(&e_pub);
    let msg1 = e_pub.to_vec();

    let mut resp = new_hs(r_static);
    let re: [u8; 32] = msg1.clone().try_into().unwrap();
    resp.re = Some(re);
    resp.mix_hash(&msg1);

    let reph = StaticSecret::from(r_eph);
    resp.e_pub = PublicKey::from(&reph).to_bytes();
    resp.e = Some(reph);
    let resp_e_pub = resp.e_pub;
    let mut msg2 = resp_e_pub.to_vec();
    resp.mix_hash(&resp_e_pub);
    let ee = dh(resp.e.as_ref().unwrap(), &re);
    resp.mix_key(&ee);
    let s_pub = resp.s_pub;
    let enc_s = resp.encrypt_and_hash(&s_pub);
    msg2.extend_from_slice(&enc_s);
    let es = dh(&resp.s, &re);
    resp.mix_key(&es);

    let ire: [u8; 32] = msg2[..32].try_into().unwrap();
    init.re = Some(ire);
    init.mix_hash(&ire);
    let ee = dh(init.e.as_ref().unwrap(), &ire);
    init.mix_key(&ee);
    let static_r = init.decrypt_and_hash(&msg2[32..]).unwrap();
    let mut rs = [0u8; 32];
    rs.copy_from_slice(&static_r);
    init.rs = Some(rs);
    let es = dh(init.e.as_ref().unwrap(), &rs);
    init.mix_key(&es);
    let s_pub = init.s_pub;
    let msg3 = init.encrypt_and_hash(&s_pub);
    let se = dh(&init.s, &ire);
    init.mix_key(&se);

    (msg1, msg2, msg3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_kms_handshake_vectors() {
        let (m1, m2, m3) = vector_messages();
        assert_eq!(hex(&m1), HIDDEN_VECTOR_MSG1);
        assert_eq!(
            hex(&m2),
            "ac01b2209e86354fb853237b5de0f4fab13c7fcbf433a61c019369617fecf10b0301d5221716fa2be9baa96416f4d480bd33b2af1f61bf699ff92b480b35f8603b07d5505ca312ace28713717b14f279"
        );
        assert_eq!(
            hex(&m3),
            "e4993edffbf9facd0797dfc1c2b500eef7f017292f184e032d0a5fac1e3214013703e7bf6768fd4936de656c673fdf98"
        );
        assert!(!m2.windows(32).any(|w| w == public_from_seed([0x02u8; 32])));
    }

    fn hex(b: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(b.len() * 2);
        for byte in b {
            s.push(HEX[(byte >> 4) as usize] as char);
            s.push(HEX[(byte & 0xf) as usize] as char);
        }
        s
    }
}
