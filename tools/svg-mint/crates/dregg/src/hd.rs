//! Fresh receive keys per checkout.
//!
//! BTC/ETH: BIP32-hardened HD from `DREGG_HD_SEED` (or generated seed).
//! ZEC: diversifier index `i` (Orchard diversified UA via zcash-devtool when present).

use hmac::{Hmac, Mac};
use rand::RngCore;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::SecretKey;
use ripemd::Ripemd160;
use sha2::{Digest, Sha256, Sha512};
use tiny_keccak::{Hasher, Keccak};

type HmacSha512 = Hmac<Sha512>;

const HARDEN: u32 = 0x8000_0000;

#[derive(Clone, Debug)]
pub struct HdEngine {
    seed: Vec<u8>,
}

impl HdEngine {
    pub fn from_env() -> Self {
        if let Ok(h) = std::env::var("DREGG_HD_SEED") {
            if let Ok(b) = hex::decode(h.trim()) {
                if b.len() >= 16 {
                    return Self { seed: b };
                }
            }
        }
        let seed_path = std::env::var("DREGG_HD_SEED_FILE").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            format!("{home}/.terp/dregg-hd.seed")
        });
        if let Ok(h) = std::fs::read_to_string(&seed_path) {
            if let Ok(b) = hex::decode(h.trim()) {
                if b.len() >= 16 {
                    return Self { seed: b };
                }
            }
        }
        let mut seed = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        eprintln!("svg-mint-fed generated DREGG_HD_SEED (not printed); persist {seed_path}");
        let _ = std::fs::write(&seed_path, hex::encode(&seed));
        Self { seed }
    }

    /// CLOSE Linus 10: all-hardened lab path `m/84'/coin'/0'/0'/i'` (matches `derive()`,
    /// not BIP84 unhardened change/index).
    pub fn btc_p2wpkh(&self, index: u32, mainnet: bool) -> Result<String, String> {
        let coin = if mainnet { 0 } else { 1 };
        let sk = self.derive(&[84 | HARDEN, coin | HARDEN, HARDEN, HARDEN, index | HARDEN])?;
        let pk = sk.public_key().to_encoded_point(true);
        let h160 = hash160(pk.as_bytes());
        let hrp = if mainnet { "bc" } else { "bcrt" };
        encode_segwit(hrp, 0, &h160)
    }

    /// CLOSE Linus 10: all-hardened lab path `m/44'/60'/0'/0'/i'` (matches `derive()`,
    /// not BIP44 unhardened change/index). Same 0x on Base (shared, not a second index).
    pub fn eth_addr(&self, index: u32) -> Result<String, String> {
        let sk = self.derive(&[44 | HARDEN, 60 | HARDEN, HARDEN, HARDEN, index | HARDEN])?;
        let uncompressed = sk.public_key().to_encoded_point(false);
        let bytes = uncompressed.as_bytes();
        // 0x04 || X || Y
        let mut keccak = Keccak::v256();
        keccak.update(&bytes[1..]);
        let mut out = [0u8; 32];
        keccak.finalize(&mut out);
        Ok(format!("0x{}", hex::encode(&out[12..])))
    }

    /// Orchard-style 11-byte diversifier from seed||index (wallet maps this to a UA).
    pub fn zec_diversifier(&self, index: u32) -> [u8; 11] {
        let h = blake3::hash(
            &[
                b"svg-mint-zec-div-v1".as_slice(),
                self.seed.as_slice(),
                &index.to_le_bytes(),
            ]
            .concat(),
        );
        let mut d = [0u8; 11];
        d.copy_from_slice(&h.as_bytes()[..11]);
        d
    }

    /// Cosmos BIP44 `m/44'/118'/0'/0/0` → `terp1…` (do not require the user to type the addr).
    pub fn terp1_from_mnemonic(phrase: &str) -> Result<String, String> {
        let m = bip39::Mnemonic::parse_normalized(phrase.trim())
            .map_err(|e| format!("mnemonic: {e}"))?;
        let seed = m.to_seed("");
        let hd = Self { seed: seed.to_vec() };
        let sk = hd.derive_cosmos(&[
            44 | HARDEN,
            118 | HARDEN,
            HARDEN,
            0,
            0,
        ])?;
        let pk = sk.public_key().to_encoded_point(true);
        let h160 = hash160(pk.as_bytes());
        encode_cosmos("terp", &h160)
    }

    /// CLOSE Linus 10: BTC/ETH children are all-hardened (`all_hardened = true`).
    fn derive(&self, path: &[u32]) -> Result<SecretKey, String> {
        self.derive_path(path, true)
    }

    /// Last two cosmos path components are normally unhardened (`0/0`).
    fn derive_cosmos(&self, path: &[u32]) -> Result<SecretKey, String> {
        self.derive_path(path, false)
    }

    fn derive_path(&self, path: &[u32], all_hardened: bool) -> Result<SecretKey, String> {
        let mut mac = HmacSha512::new_from_slice(b"Bitcoin seed").map_err(|e| e.to_string())?;
        mac.update(&self.seed);
        let i = mac.finalize().into_bytes();
        let mut k = [0u8; 32];
        let mut c = [0u8; 32];
        k.copy_from_slice(&i[..32]);
        c.copy_from_slice(&i[32..]);
        for n in path {
            let hardened = all_hardened || (*n & HARDEN) != 0;
            let mut mac = HmacSha512::new_from_slice(&c).map_err(|e| e.to_string())?;
            if hardened {
                mac.update(&[0u8]);
                mac.update(&k);
            } else {
                let sk = SecretKey::from_slice(&k).map_err(|e| e.to_string())?;
                let pk = sk.public_key().to_encoded_point(true);
                mac.update(pk.as_bytes());
            }
            mac.update(&n.to_be_bytes());
            let o = mac.finalize().into_bytes();
            let il = SecretKey::from_slice(&o[..32]).map_err(|e| e.to_string())?;
            let parent = SecretKey::from_slice(&k).map_err(|e| e.to_string())?;
            k = add_sk(&parent, &il)?;
            c.copy_from_slice(&o[32..]);
        }
        SecretKey::from_slice(&k).map_err(|e| e.to_string())
    }
}

fn add_sk(a: &SecretKey, b: &SecretKey) -> Result<[u8; 32], String> {
    use k256::elliptic_curve::PrimeField;
    let sa = k256::Scalar::from_repr(a.to_bytes()).into_option().ok_or("parent scalar")?;
    let sb = k256::Scalar::from_repr(b.to_bytes()).into_option().ok_or("il scalar")?;
    let sum = sa + sb;
    Ok(sum.to_bytes().into())
}

fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let r = Ripemd160::digest(sha);
    let mut o = [0u8; 20];
    o.copy_from_slice(&r);
    o
}

fn encode_cosmos(hrp: &str, prog: &[u8]) -> Result<String, String> {
    use bech32::ToBase32;
    bech32::encode(hrp, prog.to_base32(), bech32::Variant::Bech32).map_err(|e| e.to_string())
}

fn terp_home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".terp")
}

fn read_mnemonic_path(p: &std::path::Path) -> Result<String, String> {
    let s = std::fs::read_to_string(p).map_err(|_| format!("missing {} (chmod 600)", p.display()))?;
    let t = s.trim().to_string();
    if t.is_empty() {
        return Err(format!("empty mnemonic file {}", p.display()));
    }
    Ok(t)
}

/// DREGG **mint** key. Never the contract-admin migrate key.
pub fn load_dregg_runtime_mnemonic() -> Result<String, String> {
    if let Ok(m) = std::env::var("DREGG_RUNTIME_MNEMONIC") {
        let t = m.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    read_mnemonic_path(&terp_home().join("dregg-svg-runtime.mnemonic"))
}

/// morocco-1 **cw-svg-minter admin** (terp1fxcc…). Disjoint from DREGG runtime.
pub fn load_minter_admin_mnemonic() -> Result<String, String> {
    if let Ok(m) = std::env::var("SVG_MINTER_ADMIN_MNEMONIC") {
        let t = m.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    read_mnemonic_path(&terp_home().join("svg-minter-admin.mnemonic"))
}

pub fn addr_from_mnemonic_file(kind: &str) -> Result<String, String> {
    let m = match kind {
        "admin" => load_minter_admin_mnemonic()?,
        _ => load_dregg_runtime_mnemonic()?,
    };
    HdEngine::terp1_from_mnemonic(&m)
}

// CLOSE Linus 11: convert_bits / bech32_polymod / hrp_expand deleted (unused; encode_segwit uses bech32).

/// BIP173 Bech32 (not Bech32m). Crate 0.9 matches bitcoin/bitcoin:29 for witness v0.
fn encode_segwit(hrp: &str, witver: u8, prog: &[u8]) -> Result<String, String> {
    use bech32::{u5, ToBase32, Variant};
    if witver > 16 {
        return Err("witver".into());
    }
    let mut data = vec![u5::try_from_u8(witver).map_err(|e| e.to_string())?];
    data.extend(prog.to_base32());
    bech32::encode(hrp, data, Variant::Bech32).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_children() {
        let hd = HdEngine {
            seed: vec![7u8; 32],
        };
        let a = hd.eth_addr(0).unwrap();
        let b = hd.eth_addr(1).unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with("0x"));
        let x = hd.btc_p2wpkh(0, false).unwrap();
        let y = hd.btc_p2wpkh(1, false).unwrap();
        assert_ne!(x, y);
        assert!(x.starts_with("bcrt1q"));
        assert_eq!(x.len(), 44, "p2wpkh bcrt1 is 44 chars");
        let prog = hex::decode("751e76e8199196d454941c45d1b3a323f1433bd6").unwrap();
        // BIP173 vector (bitcoin/bitcoin:29 validateaddress isvalid).
        assert_eq!(
            encode_segwit("bc", 0, &prog).unwrap(),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        );
        assert_ne!(hd.zec_diversifier(0), hd.zec_diversifier(1));
    }
}
