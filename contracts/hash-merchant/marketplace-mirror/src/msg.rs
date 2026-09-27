use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Uint128};

use crate::state::{Product, PurchaseOrder, RootEntry};

// ---------------------------------------------------------------------------
// Instantiate
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct InstantiateMsg {
    /// Contract admin (also protocol-fee recipient when fee is set).
    pub admin: String,
    /// Settlement buffer in blocks. Default **7**. Allowed range **5–10**.
    #[serde(default = "default_min_buffer_blocks")]
    pub min_buffer_blocks: u64,
    /// Bank denom accepted for purchases (e.g. `uterp`).
    pub accepted_denom: String,
    /// Optional protocol fee in basis points (100 = 1%). Taken on settle.
    #[serde(default)]
    pub protocol_fee_bps: Option<u64>,
    /// Governance flag: allow a relayer to auto-execute `Purchase` from Nostr ingress.
    /// **Default false** — parse-intent only. The contract never submits txs itself.
    #[serde(default)]
    pub auto_execute_ingress: bool,
}

fn default_min_buffer_blocks() -> u64 {
    7
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

#[cw_serde]
pub enum ExecuteMsg {
    /// List a product SKU with initial stock and unit price.
    /// Caller becomes `seller`. Admin may also list.
    ListProduct {
        sku: String,
        stock: u64,
        unit_price: Uint128,
        /// Prefer bare sha256 hex (BUD primary). See hash-market `content-distribution.md`.
        content_cid: Option<String>,
        /// Optional NIP-15 product d-tag for Nostr mirror.
        d_tag: Option<String>,
    },
    /// Seller or admin: set absolute stock. Cannot go below reserved qty.
    UpdateStock { sku: String, stock: u64 },
    /// Buyer: pay ≥ unit_price * qty in accepted_denom.
    /// Creates a Reserved order; inventory reserved until settle/cancel.
    Purchase { sku: String, qty: u64 },
    /// Anyone: settle after buffer. Pays seller (minus optional fee), decrements stock.
    Settle { order_id: u64 },
    /// Buyer or admin only: cancel while Reserved; refund + release reservation.
    /// Seller cannot cancel (prevents grief during buffer escrow).
    Cancel { order_id: u64 },
    /// Admin only: flip ingress auto-execute policy. Default remains off.
    /// Does not cause the contract to pull Nostr events.
    SetAutoExecuteIngress { enabled: bool },
}

// ---------------------------------------------------------------------------
// Sudo — x/hashmerchant module callback
// ---------------------------------------------------------------------------

/// Matches module JSON: `{"hash_merchant":{...}}` (loyalty-verifier / hashmerchant-test).
#[cw_serde]
pub enum SudoMsg {
    HashMerchant {
        chain_uid: String,
        algo: String,
        /// Base64-encoded root bytes from the module (or passthrough hex).
        root: String,
        height: u64,
        #[serde(default)]
        attestation_count: u32,
        #[serde(default)]
        block_time: i64,
    },
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},

    #[returns(ProductResponse)]
    Product { sku: String },

    #[returns(OrderResponse)]
    Order { id: u64 },

    #[returns(OrdersResponse)]
    ListOrders {
        start_after: Option<u64>,
        limit: Option<u32>,
    },

    #[returns(ReservedQtyResponse)]
    ReservedQty { sku: String },

    /// Last foreign inventory/oracle root stored via hashmerchant sudo.
    #[returns(RootResponse)]
    GetRoot { chain_uid: String, algo: String },
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct ConfigResponse {
    pub admin: Addr,
    pub min_buffer_blocks: u64,
    pub accepted_denom: String,
    pub protocol_fee_bps: Option<u64>,
    pub next_order_id: u64,
    pub auto_execute_ingress: bool,
}

#[cw_serde]
pub struct ProductResponse {
    pub product: Product,
    pub reserved: u64,
    pub available: u64,
}

#[cw_serde]
pub struct OrderResponse {
    pub order: PurchaseOrder,
}

#[cw_serde]
pub struct OrdersResponse {
    pub orders: Vec<PurchaseOrder>,
}

#[cw_serde]
pub struct ReservedQtyResponse {
    pub sku: String,
    pub reserved: u64,
}

#[cw_serde]
pub struct RootResponse {
    pub chain_uid: String,
    pub algo: String,
    pub root: String,
    pub height: u64,
    pub attestation_count: u32,
    pub block_time: i64,
}

impl From<RootEntry> for RootResponse {
    fn from(e: RootEntry) -> Self {
        Self {
            chain_uid: e.chain_uid,
            algo: e.algo,
            root: e.root,
            height: e.height,
            attestation_count: e.attestation_count,
            block_time: e.block_time,
        }
    }
}
