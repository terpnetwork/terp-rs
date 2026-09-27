//! Terp authenticator cw-orch suite — full core priority set.
//!
//! ## Types
//! - [`terp_auth`] / [`terp_account`] — in-tree trait + `AuthSudoMsg`
//!
//! ## Suite fields (all implement `TerpAccountTrait` + `AuthSudoContract`)
//! | Field | Contract | Auth path |
//! |-------|----------|-----------|
//! | `passkey` | terp-passkey | WebAuthn-shaped payload |
//! | `recovery` | terp-recovery | guardian threshold |
//! | `ed25519` | terp-ed25519 | ed25519_verify |
//! | `eth` | terp-eth | EIP-191 recover |
//! | `irl` | terp-irl | epoch witness scaffold |
//! | `zk_jwt` | terp-zkjwt | JWT ZK proof API |
//! | `zk_poseidon` | terp-zkposiedon | membership scaffold |
//! | `vsck` | terp-vsck | vote-sdk private voting |
//!
//! Each contract routes sudo via `process_sudo_auth` → five `on_auth_*` handlers.

pub mod fixtures;
pub mod future;
pub mod interfaces;
pub mod suite;
pub mod traits;

pub use fixtures::*;
pub use suite::*;
pub use traits::*;
