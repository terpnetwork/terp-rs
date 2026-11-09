use cw_storage_plus::{Item, Map};

use crate::types::{ClientState, ConsensusState};

pub const CLIENT_STATE: Item<ClientState> = Item::new("client_state");
pub const CONSENSUS_HEAD: Item<ConsensusState> = Item::new("consensus_head");
/// pack_lanes hex → true (includes genesis).
pub const PROVEN_ROOTS: Map<&str, bool> = Map::new("proven_roots");
pub const WASM_CHECKSUM: Item<cosmwasm_std::Binary> = Item::new("checksum");
