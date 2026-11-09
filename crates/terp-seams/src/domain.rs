//! Single SSOT for **domain tags, encodings, and numeric scales** used by
//! bridge mint, private DEX, SEAM-NOTE-OUT, Cash App corridor, and ZAP1 LC.
//!
//! Do not re-declare these bytes in sibling modules. Product paths must hash
//! with these exact labels so notes, nullifiers, and intents stay compatible.

/// Opaque 32-byte digest / id (shared width for assets, roots, bindings).
pub type Hash32 = [u8; 32];

/// Terp-canonical 32-byte asset id (SEAM-NOTE-OUT width).
pub type AssetId32 = [u8; 32];

// ---------------------------------------------------------------------------
// Price / slip scales (DEX + corridor)
// ---------------------------------------------------------------------------

/// Price fixed-point scale (1e18). DEX implied price and corridor mid floors.
pub const PRICE_SCALE: u128 = 1_000_000_000_000_000_000;

/// Basis-point denominator (10_000 = 100%).
pub const BPS_DEN: u128 = 10_000;

// ---------------------------------------------------------------------------
// SEAM-NOTE-OUT V0 layout
// ---------------------------------------------------------------------------

pub const SEAM_NOTE_OUT_V0_LEN: usize = 382;
pub const VERSION_V0: u8 = 0;
pub const DOMAIN_TAG_NOTE_OUT: [u8; 16] = *b"terp-note-out-v0";

pub const ORIGIN_HEADSTASH: u8 = 0x01;
pub const ORIGIN_BRIDGE_MINT: u8 = 0x02;

pub const CM_UNSPECIFIED: u8 = 0x00;
pub const CM_ORCHARD_CMX: u8 = 0x01;
pub const CM_TACIT_KECCAK_LEAF: u8 = 0x02;
pub const CM_ABSTRACT_LEAF_V0: u8 = 0x03;

pub const NF_UNSPECIFIED: u8 = 0x00;
pub const NF_HEADSTASH_CLAIM: u8 = 0x01;
pub const NF_BRIDGE_BURN: u8 = 0x02;
pub const NF_POOL_SPEND: u8 = 0x03;

pub const ABSTRACT_LEAF_LABEL: &[u8] = b"terp-seam-leaf-v0";
pub const POOL_NF_LABEL: &[u8] = b"pool-nf-v0";
pub const EGRESS_NF_LABEL: &[u8] = b"egress-nf-v0";
pub const DEPOSIT_NU_LABEL: &[u8] = b"terp-btc-deposit-nu-v0";

pub const CLEARTEXT_LAYOUT_SEAM_NOTE_OUT_V0: &str = "SEAM-NOTE-OUT-V0";
pub const SCHEME_XCHACHA20POLY1305: &str = "xchacha20poly1305";
pub const NOTE_ID_MAX_LEN: usize = 200;

// ---------------------------------------------------------------------------
// Bridge / Tacit hinge
// ---------------------------------------------------------------------------

pub const TERP_ASSET_DOMAIN_TAG: &[u8] = b"terp-tacit-asset-v1";
pub const TERP_PRIVATE_BRIDGE_DOMAIN_TAG: &[u8] = b"terp-private-bridge-v1";
pub const TERP_BRIDGE_CLAIM_V1: &[u8] = b"terp-bridge-claim-v1";

pub const DEFAULT_CONFIRMATIONS_K: u64 = 6;
pub const DEFAULT_MAX_LC_LAG: u64 = 64;

// ---------------------------------------------------------------------------
// Cash App → ZEC corridor
// ---------------------------------------------------------------------------

pub const INTENT_DOMAIN_TAG: &[u8] = b"terp-cashapp-intent-v0";
pub const CORRIDOR_ID_CASHAPP_BTC_ZEC_V0: &str = "cashapp-btc-zec-v0";
pub const SIM_BURN_DOMAIN_TAG: &[u8] = b"terp-cashapp-sim-burn-v0";
pub const CORRIDOR_TAG_DOMAIN: &[u8] = b"terp-corridor-tag-v0";

// ---------------------------------------------------------------------------
// IBC v2 / ZAP1 light client (must match cw-ics08-wasm-zap1)
// ---------------------------------------------------------------------------

pub const IBC_V2_ZAP1_APP_V1: &[u8] = b"ibc-v2-zap1-app-v1";
