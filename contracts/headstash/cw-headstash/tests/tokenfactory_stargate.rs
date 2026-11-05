//! Tokenfactory Stargate fixture for the tests/support harness.
//!
//! The contract encodes MsgCreateDenom/MsgMint/MsgBurn as protobuf
//! `CosmosMsg::Stargate` (headstash-v2), so this delegates to the proto-decoding
//! fixture in `common/tf_stargate.rs` (consolidation 2026-11-05; the previous
//! JSON `AnyMsg` handler no longer matched the contract).
include!("common/tf_stargate.rs");
pub use tf_stargate::*;

pub fn tf_app() -> TfApp {
    new_app()
}
