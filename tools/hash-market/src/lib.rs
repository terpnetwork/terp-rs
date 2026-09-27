//! hash-market library.
//!
//! Feature gates:
//! - `msg` — protobuf message types
//! - `custody` — key management and signing
//! - `transport` — provider transport modes (grpc, http_poll, websocket)
//! - `server` — validator sidecar HTTP endpoints
//! - `client` — hashmerchant client implementations (includes nostr)
//! - `nostr` — Nostr relay integration (NIP-77, NIP-87)
//! - `blossom` — BUD protocol server (requires server). Default note-serve path (NIP-BUD).
//! - `pir` — opt-in public-leaf XOR PIR (`notes.json` + merkle path). Not default; not a second server; does not replace Nostr or blossom.
//! - `onboard` — DAO feegrant plane nested at `/onboard` (on with `server`; not a second process)
//! - frost inbox nested at `/itl/frost/*` on 127.0.0.1 (not FilePV)
#[cfg(test)]
pub mod tests;

// Re-export harness types for integration crates that enable `cfg(test)` only —
// production code uses `oracle` + `hash::tree` directly.

pub mod config;
/// BUD sha256 + optional IPFS dual-index, webhooks (oline-compatible). Off by default.
pub mod content;
pub mod fields;
#[cfg(feature = "onboard")]
pub mod frost;
pub mod hash;
pub mod msg;
#[cfg(feature = "onboard")]
pub mod onboard;
#[cfg(feature = "onboard")]
pub mod onboard_authz;
/// Multi-source price **bounds** (Tacit cUSD-like). Does not mint balances.
/// See `docs/oracle-connect-bounds.md`. Always compiled (pure types + aggregate).
///
/// Stable Connect-style surface: re-exported below for library consumers.
pub mod oracle;
/// XOR PIR kernel (always) plus feature-`pir` public-leaf ingest.
/// The `/notes/{hs_id}/pir` route is a no-op 404 unless `--features pir`.
pub mod pir;
pub mod server;
pub mod store;
pub mod trusted_builder;

// ── Stable oracle library re-exports (Connect-style bounds) ─────────────────
pub use oracle::{
    aggregate_bounds, bound_to_vote_extension, decode_bound_root, encode_bound_root,
    parse_decimal_str, parse_ticker_json, price_chain_uid, AggregateError, AggregationMethod,
    AggregationPolicy, AttributedPrice, OracleAttributeStore, OracleBound, OracleBoundsResponse,
    ProviderKind, ALGO_PRICE_MEDIAN_V1, BOUND_ROOT_LEN,
};

#[cfg(feature = "custody")]
pub mod custody;

#[cfg(feature = "transport")]
pub mod transport;

#[cfg(feature = "server")]
pub mod middleware;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "ve")]
pub mod ve;

#[cfg(feature = "eth")]
pub use client::eth::{EthBlock, EthClient, EthProofResponse};

#[cfg(feature = "ve")]
pub use ve::{
    router, run_provider_feeder, ExtendVoteAllRequest, ExtendVoteRequest, ExtendVoteResponse,
    ProviderData, ProviderKey, ProviderStatus,
};

#[cfg(feature = "ve")]
pub use crate::client::ve::{SignedVoteExtension, VoteExtensionHandler};

#[cfg(feature = "nostr")]
pub use client::nostr::{
    chain_attrs_to_nostr, metadata_to_nip52_event, offchain_to_nip52, onchain_to_nip52,
    parse_calendar_action, CalendarChainAction, CalendarMetaView, HashMerchantKind,
    HashMerchantPayload, HashMerchantRelayInfo, NegentropyFilter, Nip01Event,
};

// Content plane calendar/listing bind/resolve is always available (no second blob store).
pub use content::{
    bind_listing_body, bind_offchain_body, bind_offchain_event, content_path, encode_for_chain,
    is_nip52_event_kind, parse_cid, resolve_local, resolve_url, CidKind, OffchainBind,
    OffchainCalendarMeta, ParsedCid, KIND_DATE_BASED, KIND_TIME_BASED,
};
