//! Normative raw-key layout for ante **raw queries** of this ceremony module.
//!
//! Thin gate (V2) should prefer `deps.querier.raw_query` (or host storage read)
//! over smart query `{"is_spent":…}` on the hot Authenticate path.
//!
//! ## Layout (product)
//!
//! CosmWasm `cw-storage-plus` Map with namespace `"spent"`:
//!
//! ```text
//! key = length-prefixed(namespace "spent")
//!       || length-prefixed(domain utf8)
//!       || length-prefixed(session_id utf8)
//!       || length-prefixed(nullifier bytes)
//! value = empty / 0x01-equivalent (presence = spent)
//! ```
//!
//! Ceremony metadata Map namespace `"ceremony"`:
//!
//! ```text
//! key = length-prefixed("ceremony") || length-prefixed(session_id)
//! value = JSON Ceremony
//! ```
//!
//! ### Logical sketch from design doc
//!
//! ```text
//! 0x00 || "spent" || domain || session_id || nullifier  →  spent
//! 0x01 || "ceremony" || session_id                     →  CeremonyInfo
//! ```
//!
//! Implementors of native raw readers must use the **same** encoding as
//! `cw-storage-plus` Map (not a hand-rolled 0x00 prefix) so gate and module agree.
//!
//! ## Stability
//!
//! - Namespace strings `spent`, `ceremony`, `reg`, `config` are **frozen** for V1.
//! - Do not rename Map namespaces without a migration + gate upgrade.

/// Frozen Map namespace for spent nullifiers (must match `state::SPENT`).
pub const SPENT_NAMESPACE: &str = "spent";

/// Frozen Map namespace for ceremony records (must match `state::CEREMONIES`).
pub const CEREMONY_NAMESPACE: &str = "ceremony";

/// Frozen Map namespace for registrations.
pub const REG_NAMESPACE: &str = "reg";

/// Item namespace for config.
pub const CONFIG_NAMESPACE: &str = "config";
