use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::{Item, Map};

/// Default settlement buffer (blocks). Spec range 5–10; product default **7**.
pub const DEFAULT_MIN_BUFFER_BLOCKS: u64 = 7;
pub const MIN_BUFFER_BLOCKS_LO: u64 = 5;
pub const MIN_BUFFER_BLOCKS_HI: u64 = 10;

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub min_buffer_blocks: u64,
    pub accepted_denom: String,
    /// Optional bps fee taken from purchase amount on settle (paid to admin).
    pub protocol_fee_bps: Option<u64>,
    /// Relayer may auto-submit `Purchase` from Nostr only when true (admin-gated).
    #[serde(default)]
    pub auto_execute_ingress: bool,
}

#[cw_serde]
pub struct Product {
    pub sku: String,
    pub seller: Addr,
    /// Absolute listed stock (not reduced on reserve; reduced on settle).
    pub stock: u64,
    pub unit_price: Uint128,
    /// Prefer bare sha256 hex (BUD / content-plane primary).
    pub content_cid: Option<String>,
    /// Optional NIP-15 product d-tag.
    pub d_tag: Option<String>,
}

#[cw_serde]
#[derive(Eq)]
pub enum OrderStatus {
    Reserved,
    Settled,
    Cancelled,
}

impl OrderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderStatus::Reserved => "reserved",
            OrderStatus::Settled => "settled",
            OrderStatus::Cancelled => "cancelled",
        }
    }
}

#[cw_serde]
pub struct PurchaseOrder {
    pub id: u64,
    pub buyer: Addr,
    pub seller: Addr,
    pub sku: String,
    pub qty: u64,
    pub amount: Uint128,
    pub reserve_height: u64,
    /// `reserve_height + min_buffer_blocks` — settle only at/after this height.
    pub settle_after: u64,
    pub status: OrderStatus,
}

/// Last hashmerchant-attested foreign root for (chain_uid, algo).
/// Used for future proof-gated restock; **does not mint supply**.
#[cw_serde]
pub struct RootEntry {
    pub chain_uid: String,
    pub algo: String,
    /// Hex-encoded root bytes.
    pub root: String,
    pub height: u64,
    pub attestation_count: u32,
    pub block_time: i64,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const NEXT_ORDER_ID: Item<u64> = Item::new("next_order_id");

/// sku → Product
pub const PRODUCTS: Map<&str, Product> = Map::new("products");
/// order_id → PurchaseOrder
pub const ORDERS: Map<u64, PurchaseOrder> = Map::new("orders");
/// sku → currently reserved qty (sum of Reserved orders)
pub const RESERVED: Map<&str, u64> = Map::new("reserved");
/// (chain_uid, algo) → last confirmed foreign root
pub const ROOTS: Map<(&str, &str), RootEntry> = Map::new("roots");
