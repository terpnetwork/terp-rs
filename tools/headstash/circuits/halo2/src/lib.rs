//! # orchard
//!
//! ## Nomenclature
//!
//! All types in the `orchard` crate, unless otherwise specified, are Orchard-specific
//! types. For example, [`Address`] is documented as being a shielded payment address; we
//! implicitly mean it is an Orchard payment address (as opposed to e.g. a Sapling payment
//! address, which is also shielded).

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Temporary until we have more of the crate implemented.
#![allow(dead_code)]
// Catch documentation errors caused by code changes.
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(missing_debug_implementations)]
// #![deny(missing_docs)]
#![deny(unsafe_code)]

#[macro_use]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

/// rand 0.10 OS entropy is fallible (`SysRng`). Tests/prove use thread rng (infallible).
#[cfg(feature = "std")]
pub(crate) fn os_rng() -> rand::rngs::ThreadRng {
    rand::rng()
}

use alloc::vec::Vec;

mod action;
pub mod address;
pub mod builder;
mod constants;
// Bundle module is partially ported; BatchValidator lives under bundle/batch for
// internal use. Prefer `Proof::add_to_batch` for multi-proof tooling.
// pub mod bundle;

// Full Halo2 Action circuit (host prove) — needs host-crypto (halo2-base + libsecp).
#[cfg(all(feature = "circuit", feature = "host-crypto"))]
pub mod circuit;

// Guest CosmWasm: Instance-only surface (no multicore / C-sys graph).
#[cfg(all(feature = "circuit", not(feature = "host-crypto")))]
#[path = "circuit_guest.rs"]
pub mod circuit;

#[cfg(feature = "circuit")]
mod vm_instance;

#[cfg(feature = "circuit")]
pub mod keys;
pub mod note;
// pub mod note_encryption;
// pub mod pczt;
pub mod gen;
pub mod primitives;
mod spec;

/// Poseidon-v1 public distribution / inclusion Merkle set (pure helpers).
///
/// See [`distro_poseidon`] and ADR `docs/plans/spectrum/ADR-POSEIDON-DISTRO-TREE.md`.
#[cfg(feature = "circuit")]
pub mod distro_poseidon;

/// Poseidon-v1 private note commitment (pure helpers).
///
/// See [`note_poseidon`] and ADR `docs/plans/spectrum/ADR-POSEIDON-NOTE-COMMIT.md`.
/// Replaces Sinsemilla `NoteCommit` for Headstash private notes (lab; not mainnet).
#[cfg(feature = "circuit")]
pub mod note_poseidon;

/// Offline-wallet claim authorization: EIP-191 message, nullifier, signature check.
#[cfg(feature = "circuit")]
pub mod claim_auth;

#[cfg(feature = "interface")]
pub mod suite;

#[cfg(test)]
mod test_vectors;
#[cfg(all(test, feature = "circuit"))]
mod orchard_delta_part_t;
#[cfg(all(test, feature = "circuit", feature = "host-crypto", feature = "interface"))]
mod snap_bench;
pub mod tree;
pub mod value;
pub mod zip32;
pub use action::Action;
pub use address::Address;
// pub use bundle::Bundle;
pub use constants::{
    fixed_bases::{OrchardFixedBases, FIXED_AMOUNTS},
    sinsemilla::{OrchardCommitDomains, LEAF_PERSONALIZATION, MERKLE_CRH_PERSONALIZATION},
    OrchardHashDomains, MERKLE_DEPTH_ORCHARD as NOTE_COMMITMENT_TREE_DEPTH,
};
pub use note::Note;
pub use spec::{
    biguint_to_fe_simple, decompose_biguint_simple, recp_to_fp, to_native_out_of_circuit,
};
pub use tree::Anchor;

/// A proof of the validity of an Orchard [`Bundle`].
///
/// [`Bundle`]: crate::bundle::Bundle
#[derive(Clone)]
pub struct Proof(Vec<u8>);

impl core::fmt::Debug for Proof {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if f.alternate() {
            f.debug_tuple("Proof").field(&self.0).finish()
        } else {
            // By default, only show the proof length, not its contents.
            f.debug_tuple("Proof")
                .field(&format_args!("{} bytes", self.0.len()))
                .finish()
        }
    }
}

impl AsRef<[u8]> for Proof {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl memuse::DynamicUsage for Proof {
    fn dynamic_usage(&self) -> usize {
        self.0.dynamic_usage()
    }

    fn dynamic_usage_bounds(&self) -> (usize, Option<usize>) {
        self.0.dynamic_usage_bounds()
    }
}

impl Proof {
    /// Constructs a new Proof value.
    pub fn new(bytes: Vec<u8>) -> Self {
        Proof(bytes)
    }
}
