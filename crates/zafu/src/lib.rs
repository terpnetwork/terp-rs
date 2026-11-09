//! Offline consensus signer.
//!
//! The Ed25519 key is created here and does not leave. A laptop KMS submits
//! canonical sign bytes over a Bluetooth Noise session and receives a
//! signature. The validator link in front of that KMS can be SecretConnection
//! or Noise; this crate does not care which.

#[allow(dead_code)]
mod noise;

use ed25519_zebra::{SigningKey, VerificationKey};

pub use noise::{advertisement, handshake_message1, HIDDEN_VECTOR_MSG1};

/// Last consensus message this device signed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hwm {
    pub height: u64,
    pub round: u32,
    pub step: u8,
    set: bool,
}

impl Hwm {
    /// A request is allowed when it is strictly after the last signature.
    pub fn allows(self, height: u64, round: u32, step: u8) -> bool {
        if !self.set {
            return true;
        }
        if height != self.height {
            return height > self.height;
        }
        if round != self.round {
            return round > self.round;
        }
        step > self.step
    }
}

/// Cold signer. Same rules as an air-gapped consensus key: monotonic votes
/// and proposals only, no export.
pub struct ColdSigner {
    key: SigningKey,
    hwm: Hwm,
}

impl ColdSigner {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            key: SigningKey::from(seed),
            hwm: Hwm::default(),
        }
    }

    pub fn pubkey(&self) -> [u8; 32] {
        let vk = VerificationKey::from(&self.key);
        vk.into()
    }

    pub fn sign(
        &mut self,
        height: u64,
        round: u32,
        step: u8,
        msg: &[u8],
    ) -> Result<[u8; 64], &'static str> {
        if msg.is_empty() {
            return Err("empty sign bytes");
        }
        if !self.hwm.allows(height, round, step) {
            return Err("refusing to sign at or below the high-water mark");
        }
        let sig = self.key.sign(msg);
        self.hwm.height = height;
        self.hwm.round = round;
        self.hwm.step = step;
        self.hwm.set = true;
        Ok(sig.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_regression_and_signs_the_message() {
        let mut s = ColdSigner::from_seed([9u8; 32]);
        let sig = s.sign(2, 0, 1, b"a").unwrap();
        assert!(s.sign(2, 0, 1, b"b").is_err());
        assert!(s.sign(1, 9, 9, b"c").is_err());
        let next = s.sign(3, 0, 1, b"d").unwrap();
        assert_ne!(sig, next);
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn advertisement_and_first_handshake_hide_the_static_key() {
        let static_seed = [0x11u8; 32];
        let eph_seed = [0x22u8; 32];
        let ad = advertisement(&[0x33u8; 16]);
        let static_pub = noise::public_from_seed(static_seed);
        assert_eq!(ad.len(), 16);
        assert!(!ad.windows(16).any(|w| w == &static_pub[..16]));
        let msg1 = handshake_message1(static_seed, eph_seed);
        assert_eq!(msg1.len(), 32);
        assert_ne!(msg1, static_pub);
        assert!(!msg1.windows(32).any(|w| w == static_pub));
    }
}
