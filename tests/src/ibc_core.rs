//! Back-compat re-export of `terp_scripts::ibc` pure IBC derivation logic.
//!
//! Prefer `terp_scripts::ibc` for new code. Existing `use terp_scripts::ibc_core::...` keeps working.

pub use crate::ibc::{
    ChannelEdge, ChannelHop, IBCAssetRoute, IBCAssetRoutingTable, IBCChannelGraph,
    RoutingTableMetadata, TerpChannelInfo, build_channel_to_chain_map, compute_ibc_denom_hash,
    derive_terp_ibc_denom, hop_count_from_trace_path,
};
