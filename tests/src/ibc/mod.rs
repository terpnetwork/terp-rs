//! IBC authenticity library: pure predict → observe → diff.
//!
//! Hard invariants (errors): channel side correctness, hop_count == transfer/
//! segments, prefer direct ACTIVE preferred routes, hash binding, schema.

pub mod diff;
pub mod fixtures;
pub mod graph;
pub mod hash;
pub mod normalize;
pub mod observe;
pub mod predict;
pub mod publish;
pub mod routes;
pub mod schema;
pub mod stamp;

pub use diff::{DiffItem, DiffReport, DiffSeverity};
pub use fixtures::{
    golden_chain_assets, golden_root, load_audited_known_hashes, load_expected_lookup,
    load_golden_ibc_data, load_json, load_known_hashes, load_synthetic_known_hashes,
    parse_known_hashes_value,
};
pub use graph::{ChannelEdge, ChannelHop, IBCChannelGraph};
pub use hash::{compute_ibc_denom_hash, hop_count_from_trace_path};
pub use normalize::{build_channel_to_chain_map, finalize_channels_for_ibc_entry, ordering_to_str};
pub use observe::{
    CompareOptions, FixtureBackend, ObserveBackend, ObservedBalance, ObservedChannel,
    ObservedDenomTrace, SnapshotBackend, compare_balance, compare_predict_observe,
    compare_predict_observe_with,
};
pub use predict::{PredictedWorld, check_invariants};
pub use publish::{AtomicPublisher, default_public_dir, file_sha256, resolve_out_dir};
pub use routes::{
    IBCAssetRoute, IBCAssetRoutingTable, RoutingTableMetadata, TerpChannelInfo,
    derive_terp_ibc_denom,
};
pub use schema::{validate_asset_entry, validate_ibc_data_entry};
pub use stamp::{
    git_build_stamp, merge_ibc_and_sources_into_state, stamp_code_sources,
    update_public_state_file, CODE_SOURCE_TABLE,
};
