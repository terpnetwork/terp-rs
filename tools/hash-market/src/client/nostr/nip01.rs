//! NIP-01 event identity + BIP-340 Schnorr signing for every Nostr egress builder.
//!
//! * `id`     = lowercase hex `sha256(serde_json([0, pubkey, created_at, kind, tags, content]))`
//!   (serde_json's string escaping matches NIP-01 / `JSON.stringify`: `\n \" \\ \r \t \b \f`
//!   short forms, other C0 controls as `\u00xx`, everything else verbatim).
//! * `pubkey` = 32-byte x-only secp256k1 key, lowercase hex.
//! * `sig`    = 64-byte BIP-340 Schnorr signature over the 32-byte id, lowercase hex.
//!
//! There is no unsigned / placeholder path: builders take a [`NostrSigner`] and
//! always return a fully signed event. The signing key is supplied by the caller
//! (config / custody); it is never logged and `Debug` is redacted.

use k256::schnorr::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Wire-shape Nostr event (NIP-01) independent of ict-rs / harness crates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nip01Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

/// BIP-340 signing key for Nostr egress. Secret material is zeroized on drop
/// (k256 `SigningKey`) and never exposed through `Debug`/`Display`.
#[derive(Clone)]
pub struct NostrSigner {
    key: SigningKey,
}

impl std::fmt::Debug for NostrSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NostrSigner")
            .field("pubkey", &self.pubkey_hex())
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl NostrSigner {
    /// Load from 32 raw secret-key bytes supplied by the caller (config / custody).
    pub fn from_secret_bytes(sk: &[u8]) -> Result<Self, String> {
        SigningKey::from_bytes(sk)
            .map(|key| Self { key })
            .map_err(|_| "invalid secp256k1 secret key".to_string())
    }

    /// Fresh random key from the OS CSPRNG (ephemeral test / session identities).
    pub fn generate() -> Self {
        Self {
            key: SigningKey::random(&mut rand_core::OsRng),
        }
    }

    /// x-only public key, lowercase hex (64 chars).
    pub fn pubkey_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }

    /// BIP-340 sign a 32-byte message with explicit auxiliary randomness.
    pub fn sign_raw_with_aux(&self, msg32: &[u8; 32], aux: &[u8; 32]) -> Result<[u8; 64], String> {
        let sig = self
            .key
            .sign_raw(msg32, aux)
            .map_err(|_| "schnorr signing failed".to_string())?;
        Ok(sig.to_bytes())
    }

    /// BIP-340 sign with fresh OS randomness as `aux_rand`.
    pub fn sign_raw(&self, msg32: &[u8; 32]) -> Result<[u8; 64], String> {
        let mut aux = [0u8; 32];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut aux);
        self.sign_raw_with_aux(msg32, &aux)
    }

    /// Build, id, and sign a NIP-01 event authored by this key.
    pub fn sign_event(
        &self,
        created_at: i64,
        kind: u32,
        tags: Vec<Vec<String>>,
        content: String,
    ) -> Result<Nip01Event, String> {
        let pubkey = self.pubkey_hex();
        let id = compute_id_bytes(&pubkey, created_at, kind, &tags, &content);
        let sig = self.sign_raw(&id)?;
        Ok(Nip01Event {
            id: hex::encode(id),
            pubkey,
            created_at,
            kind,
            tags,
            content,
            sig: hex::encode(sig),
        })
    }
}

/// NIP-01 canonical serialization used for the event id.
pub fn canonical_serialize(
    pubkey: &str,
    created_at: i64,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> String {
    serde_json::json!([0, pubkey, created_at, kind, tags, content]).to_string()
}

fn compute_id_bytes(
    pubkey: &str,
    created_at: i64,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> [u8; 32] {
    Sha256::digest(canonical_serialize(pubkey, created_at, kind, tags, content).as_bytes()).into()
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// True if `s` is a 32-byte lowercase-hex x-only pubkey on the curve.
pub fn is_valid_pubkey_hex(s: &str) -> bool {
    is_lower_hex(s, 64)
        && hex::decode(s)
            .ok()
            .map(|b| VerifyingKey::from_bytes(&b).is_ok())
            .unwrap_or(false)
}

impl Nip01Event {
    /// Recompute the NIP-01 id (lowercase hex) from the event fields.
    pub fn compute_id(&self) -> String {
        hex::encode(compute_id_bytes(
            &self.pubkey,
            self.created_at,
            self.kind,
            &self.tags,
            &self.content,
        ))
    }

    /// Full NIP-01 check: field formats, id == sha256(canonical), BIP-340 sig over id.
    pub fn verify(&self) -> Result<(), String> {
        if !is_lower_hex(&self.id, 64) {
            return Err("id must be 64 lowercase hex chars".into());
        }
        if !is_lower_hex(&self.pubkey, 64) {
            return Err("pubkey must be 64 lowercase hex chars".into());
        }
        if !is_lower_hex(&self.sig, 128) {
            return Err("sig must be 128 lowercase hex chars".into());
        }
        if self.created_at < 0 {
            return Err("created_at must be non-negative".into());
        }
        if self.compute_id() != self.id {
            return Err("event id is computed incorrectly".into());
        }
        let pk = hex::decode(&self.pubkey).map_err(|e| e.to_string())?;
        let vk = VerifyingKey::from_bytes(&pk).map_err(|_| "pubkey not on curve".to_string())?;
        let sig_bytes = hex::decode(&self.sig).map_err(|e| e.to_string())?;
        let sig = Signature::try_from(sig_bytes.as_slice())
            .map_err(|_| "malformed schnorr signature".to_string())?;
        let id = hex::decode(&self.id).map_err(|e| e.to_string())?;
        vk.verify_raw(&id, &sig)
            .map_err(|_| "signature verification failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    /// BIP-340 official vectors 0 and 1 (public test data).
    #[test]
    fn bip340_vectors() {
        let cases = [
            (
                "0000000000000000000000000000000000000000000000000000000000000003",
                "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca821525f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0",
            ),
            (
                "b7e151628aed2a6abf7158809cf4f3c762e7160f38b4da56a784d9045190cfef",
                "dff1d77f2a671c5f36183726db2341be58feae1da2deced843240f7b502ba659",
                "0000000000000000000000000000000000000000000000000000000000000001",
                "243f6a8885a308d313198a2e03707344a4093822299f31d0082efa98ec4e6c89",
                "6896bd60eeae296db48a229ff71dfe071bde413e6d43f917dc8dcf8c78de33418906d11ac976abccb20b091292bff4ea897efcb639ea871cfa95f6de339e4b0a",
            ),
        ];
        for (sk, pk, aux, msg, sig) in cases {
            let s = NostrSigner::from_secret_bytes(&h(sk)).unwrap();
            assert_eq!(s.pubkey_hex(), pk);
            let aux: [u8; 32] = h(aux).try_into().unwrap();
            let msg: [u8; 32] = h(msg).try_into().unwrap();
            assert_eq!(hex::encode(s.sign_raw_with_aux(&msg, &aux).unwrap()), sig);
        }
    }

    /// Real signed kind-1 event (rust-nostr `test_verify_event_id` fixture): tags, newlines,
    /// unicode-free content. Checks id recomputation AND Schnorr verification.
    #[test]
    fn nip01_known_event_vector() {
        let json = include_str!("nip01_vector_event.json");
        let ev: Nip01Event = serde_json::from_str(json).unwrap();
        assert_eq!(
            ev.compute_id(),
            "38acf9b08d06859e49237688a9fd6558c448766f47457236c2331f93538992c6"
        );
        ev.verify().expect("vector must verify");

        let mut bad = ev.clone();
        bad.content.push(' ');
        assert!(bad.verify().is_err(), "tampered content must fail id check");
        let mut bad = ev.clone();
        bad.sig.replace_range(0..2, if &ev.sig[0..2] == "00" { "01" } else { "00" });
        assert!(bad.verify().is_err(), "tampered sig must fail");
    }

    /// Second real event (kind 4, rust-nostr `test_tags_deser_without_recommended_relay`).
    #[test]
    fn nip01_known_event_vector_kind4() {
        let json = r#"{"id":"2be17aa3031bdcb006f0fce80c146dea9c1c0268b0af2398bb673365c6444d45","pubkey":"f86c44a2de95d9149b51c6a29afeabba264c18e2fa7c49de93424a0c56947785","created_at":1640839235,"kind":4,"tags":[["p","13adc511de7e1cfcf1c6b7f6365fb5a03442d7bcacf565ea57fa7770912c023d"]],"content":"uRuvYr585B80L6rSJiHocw==?iv=oh6LVqdsYYol3JfFnXTbPA==","sig":"a5d9290ef9659083c490b303eb7ee41356d8778ff19f2f91776c8dc4443388a64ffcf336e61af4c25c05ac3ae952d1ced889ed655b67790891222aaa15b99fdd"}"#;
        let ev: Nip01Event = serde_json::from_str(json).unwrap();
        assert_eq!(ev.compute_id(), ev.id);
        ev.verify().expect("kind-4 vector must verify");
    }

    #[test]
    fn canonical_escaping_matches_nip01() {
        let s = canonical_serialize(
            &"a".repeat(64),
            1,
            1,
            &[vec!["t".into(), "q\"\\".into()]],
            "l1\nl2\r\t\u{8}\u{c}\u{1}é/",
        );
        assert_eq!(
            s,
            format!(
                "[0,\"{}\",1,1,[[\"t\",\"q\\\"\\\\\"]],\"l1\\nl2\\r\\t\\b\\f\\u0001é/\"]",
                "a".repeat(64)
            )
        );
    }

    #[test]
    fn sign_verify_roundtrip_and_serde() {
        let s = NostrSigner::generate();
        let ev = s
            .sign_event(1_700_000_000, 1, vec![vec!["t".into(), "x".into()]], "hi\n\"q\"".into())
            .unwrap();
        assert!(is_valid_pubkey_hex(&ev.pubkey));
        ev.verify().unwrap();
        let back: Nip01Event = serde_json::from_str(&serde_json::to_string(&ev).unwrap()).unwrap();
        assert_eq!(back, ev);
        back.verify().unwrap();
        // another key's pubkey swapped in must fail
        let mut forged = ev.clone();
        forged.pubkey = NostrSigner::generate().pubkey_hex();
        assert!(forged.verify().is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let s = NostrSigner::from_secret_bytes(&[7u8; 32]).unwrap();
        let d = format!("{s:?}");
        assert!(d.contains("<redacted>"));
        assert!(!d.contains(&hex::encode([7u8; 32])));
    }
}
