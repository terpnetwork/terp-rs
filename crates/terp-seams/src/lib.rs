//! Unified Terp L0 seams crate (the only seam package).
//!
//! | Module | Feature (default on) |
//! |--------|----------------------|
//! | [`domain`] | always |
//! | [`bridge`] | `bridge` |
//! | [`dex`] | `dex` |
//! | [`corridor`] | `corridor` |
//! | [`compose`] | `compose` (implies dex + bridge) |
//!
//! There is no `private_dex_seams` / `seam_private_dex` crate. Depend on `terp-seams`.

#![deny(unsafe_code)]

#[cfg(feature = "bridge")]
pub mod bridge;
#[cfg(feature = "compose")]
pub mod compose;
#[cfg(feature = "corridor")]
pub mod corridor;
#[cfg(feature = "dex")]
pub mod dex;
pub mod domain;

pub use domain::*;
