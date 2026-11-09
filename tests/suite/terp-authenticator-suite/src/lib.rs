//! Terp authenticator cw-orch suite — full core priority set.
//!
//! ## Types
//! - [`terp_auth`] / [`terp_account`] — in-tree trait + `AuthSudoMsg`
//!
//! ## Suite fields (all implement `TerpAccountTrait` + `AuthSudoContract`)
//! | Field | Contract | Auth path |
//! |-------|----------|-----------|
//! | `passkey` | terp-passkey | ES256 WebAuthn over sign_mode_direct |
//! | `recovery` | terp-recovery | guardian threshold |
//! | `ed25519` | terp-ed25519 | ed25519_verify |
//! | `eth` | terp-eth | EIP-191 recover |
//! | `zk_jwt` | terp-zkjwt | JWT ZK proof API |
//! | `vsck` | terp-vsck | vote-sdk private voting (fail-closed until circuits link) |
//!
//! Each contract routes sudo via `process_sudo_auth` → five `on_auth_*` handlers.

pub mod fixtures;
pub mod future;
pub mod interfaces;
pub mod passkey_fixture;
pub mod suite;
pub mod traits;

pub use fixtures::*;
pub use suite::*;
pub use traits::*;
