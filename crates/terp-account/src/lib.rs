//! Terp account library for smart-account authenticators.
//!
//! Provides the trait path expected by authenticator contracts:
//! `terp_account::traits::default::TerpAccountTrait` → [`terp_auth::TerpAccountTrait`].

pub mod traits;

#[cfg(feature = "full")]
pub mod account;
#[cfg(feature = "full")]
pub mod manifold;
#[cfg(feature = "full")]
pub mod verify_generic;
