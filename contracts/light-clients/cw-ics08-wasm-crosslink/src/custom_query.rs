//! Custom query for the Crosslink light client.
//!
//! Currently uses `CosmWasm`'s `Empty` query type. When precompile support
//! (`RedPallas`, Poseidon, BLAKE3) is added to the VM API, this will be
//! replaced with a proper `CrosslinkCustomQuery` type.

use cosmwasm_std::Empty;

/// Crosslink custom query (currently aliased to `Empty`).
pub type CrosslinkCustomQuery = Empty;
