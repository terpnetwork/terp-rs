//! Process-wide halo2 host. hash-market installs one before it listens.
//! Claim checks call it. A missing host fails closed so a lane unit test
//! can still count a verifier call.

use std::sync::{Arc, OnceLock};

/// The host rejected the proof, or no host is installed.
#[derive(Debug, PartialEq, Eq)]
pub enum HostVerifyError {
    Rejected,
}

/// Cached verifying keys. `verify` must not build or load a key.
pub trait Halo2Host: Send + Sync {
    fn verify(&self, vk_id: u64, proof: &[u8], instances: &[u8]) -> Result<(), HostVerifyError>;

    fn vk_hash(&self, vk_id: u64) -> Option<[u8; 32]>;
}

static HOST: OnceLock<Arc<dyn Halo2Host>> = OnceLock::new();

/// Install the process host. A second call keeps the first host.
pub fn install_halo2_host(host: Arc<dyn Halo2Host>) {
    let _ = HOST.set(host);
}

/// SHA-256 of the blob registered under `vk_id`, when that id is loaded.
pub fn halo2_vk_hash(vk_id: u64) -> Option<[u8; 32]> {
    HOST.get().and_then(|host| host.vk_hash(vk_id))
}

/// Run the installed host. Missing host is a reject, not a skip.
pub fn verify_with_host(vk_id: u64, proof: &[u8], instances: &[u8]) -> Result<(), HostVerifyError> {
    match HOST.get() {
        Some(host) => host.verify(vk_id, proof, instances),
        None => Err(HostVerifyError::Rejected),
    }
}
