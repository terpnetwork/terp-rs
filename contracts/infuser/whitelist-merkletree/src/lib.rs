/// Chain must advertise `hash_blake`. Membership hashes go through `Api::blake3_256`.
#[no_mangle]
extern "C" fn requires_hash_blake() {}

pub mod admin;
pub mod contract;
pub mod error;
pub mod msg;
pub mod state;
pub mod tests;
pub use crate::error::ContractError;

#[cfg(feature = "interface")]
pub mod interface;
