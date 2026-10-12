//! Ingress plane: payment received → Poseidon commitment in `guest_root`.
//!
//! Cash App: Gap 0 `.eml` inject → WASIX parse → `H(cashapp-v1||txId)` in our DB
//! → issue `ticketSecret` → `leaf = Poseidon(ticketSecret, sku, event_id)` → bump root.
//!
//! DREGG ZEC/BTC/ETH/Base: reuse `svg_mint_local_prod`. Do not spawn a second
//! ETH/BTC/ZEC stack. Do not invent RPC. MockBech32 is not a paid path.
//!
//! Public after a sale: new `guest_root` (and count). Not txId, not amount, not credId.
//! zkEmail Groth16 is out. FROST is out.

use super::itl_auth::zip321_ok;
use super::itl_cashapp::{nullifier_hex, wasix_parse_json, Engine as CashEngine, LIST_CENTS, SKU};
use super::poseidon::{door_nullifier, hex_encode, ticket_leaf, PoseidonTree};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const EVENT_ID: &str = "evt/speakeasy-2026-10-18";
pub const CW_AVE: &str = "terp1cwave";
/// ict-rs Cash App Docker Terp. Not morocco-1. Not a FROST terpd home.
pub const CASHAPP_CHAIN_ID: &str = "itl-cashapp-1";

/// Existing ict-rs example. Do not duplicate this spawn.
pub const SVG_MINT_LOCAL_PROD: &str =
    "cargo run --example svg_mint_local_prod -p ict-rs-cw-orch --features docker,zakura,dregg-node,ethereum,bitcoin";
pub const SVG_MINT_FEATURES: &str = "docker,zakura,dregg-node,ethereum,bitcoin";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rail {
    CashApp,
    Zec,
    Btc,
    Eth,
    Base,
    Penumbra,
}

impl Rail {
    pub fn as_str(self) -> &'static str {
        match self {
            Rail::CashApp => "cashapp",
            Rail::Zec => "zec",
            Rail::Btc => "btc",
            Rail::Eth => "eth",
            Rail::Base => "base",
            Rail::Penumbra => "penumbra",
        }
    }

    pub fn is_dregg(self) -> bool {
        matches!(self, Rail::Zec | Rail::Btc | Rail::Eth | Rail::Base)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    CashappReceipt,
    DreggWatch,
}

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::CashappReceipt => "cashapp_receipt",
            EvidenceKind::DreggWatch => "dregg_watch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreggWatch {
    pub rail: Rail,
    /// memo_commit or chain tx from the watcher — never an invented id.
    pub record_id: String,
    pub dest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedTicket {
    /// Device-local. Never posted as a public sale signal.
    pub ticket_secret: [u8; 32],
    pub leaf: [u8; 32],
    pub guest_root: [u8; 32],
    pub paid_n: usize,
    /// Internal payment authenticity (Cash App txId hash or DREGG record).
    pub payment_nullifier: String,
    pub rail: Rail,
    pub sku: String,
    pub event_id: String,
    pub leaf_index: u64,
}

impl IssuedTicket {
    pub fn ticket_secret_hex(&self) -> String {
        hex_encode(&self.ticket_secret)
    }

    pub fn leaf_hex(&self) -> String {
        hex_encode(&self.leaf)
    }

    pub fn guest_root_hex(&self) -> String {
        hex_encode(&self.guest_root)
    }

    pub fn door_nullifier(&self) -> [u8; 32] {
        door_nullifier(&self.ticket_secret, &self.event_id)
    }
}

#[derive(Debug, Clone)]
pub struct IngressEngine {
    cash: CashEngine,
    tree: PoseidonTree,
    payment_nullifiers: HashSet<String>,
    /// Internal only. Never copied into `public_sale_json`.
    tx_ids: HashMap<String, String>,
    issued: usize,
}

impl Default for IngressEngine {
    fn default() -> Self {
        Self {
            cash: CashEngine::default(),
            tree: PoseidonTree::default(),
            payment_nullifiers: HashSet::new(),
            tx_ids: HashMap::new(),
            issued: 0,
        }
    }
}

impl IngressEngine {
    pub fn create_cash_order(&mut self, buyer: &str) -> super::itl_cashapp::Order {
        self.cash.create_order(buyer)
    }

    pub fn paid_count(&self) -> usize {
        self.issued
    }

    pub fn guest_root_hex(&self) -> String {
        self.tree.root_hex()
    }

    pub fn unique_payment_nullifiers(&self) -> usize {
        self.payment_nullifiers.len()
    }

    /// Public after a sale. Not txId, not amount, not credId.
    pub fn public_sale_json(&self) -> Value {
        json!({
            "guest_root": self.tree.root_hex(),
            "paid_n": self.issued,
            "update_guest_root": {
                "guest_root": self.tree.root_hex()
            }
        })
    }

    pub fn update_guest_root_msg(&self) -> Value {
        json!({
            "update_guest_root": { "guest_root": self.tree.root_hex() }
        })
    }

    /// Gap 0 inject → WASIX parse → payment nullifier → Poseidon leaf → bump root.
    pub fn ingest_cashapp_eml(&mut self, raw: &str) -> Result<IssuedTicket, &'static str> {
        let order = self.cash.ingest_eml(raw)?;
        let nf = order.nullifier.clone().ok_or("missing_nullifier")?;
        let tx = order.tx_id.clone().unwrap_or_default();
        self.issue_from_payment(Rail::CashApp, &nf, Some(&tx), order.buyer_binding.as_str())
    }

    /// WASIX guest stdout JSON (same shape as `wasix_parse_json`).
    pub fn ingest_wasix_json(&mut self, v: &Value) -> Result<IssuedTicket, &'static str> {
        if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
            let err = v
                .get("error")
                .and_then(|x| x.as_str())
                .unwrap_or("wasix_fail");
            return Err(match err {
                "not_payment_received" => "not_payment_received",
                "dkim_domain" => "dkim_domain",
                "regex_miss" => "regex_miss",
                _ => "wasix_fail",
            });
        }
        if v.get("proven").and_then(|x| x.as_bool()) == Some(true) {
            return Err("zkemail_groth16_out");
        }
        let pay_ref = v
            .get("payRef")
            .and_then(|x| x.as_str())
            .ok_or("regex_miss")?;
        let amount = v
            .get("amountCents")
            .and_then(|x| x.as_u64())
            .ok_or("regex_miss")? as u32;
        let tx_id = v.get("txId").and_then(|x| x.as_str()).ok_or("regex_miss")?;
        if amount < LIST_CENTS {
            return Err("amount_too_low");
        }
        let nf = v
            .get("nullifier")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| nullifier_hex(tx_id));
        if nf != nullifier_hex(tx_id) {
            return Err("nullifier_mismatch");
        }
        // Bind to an existing order when the engine minted the payRef.
        let html = super::itl_cashapp::synthetic_received_html(pay_ref, amount, tx_id);
        let _ = self.cash.ingest_text(&html, "Payment received")?;
        self.issue_from_payment(Rail::CashApp, &nf, Some(tx_id), pay_ref)
    }

    pub fn ingest_dregg(&mut self, watch: &DreggWatch) -> Result<IssuedTicket, &'static str> {
        if watch.rail == Rail::Penumbra {
            return Err("penumbra_skipped");
        }
        if !watch.rail.is_dregg() {
            return Err("not_dregg_rail");
        }
        if is_mock_bech32(&watch.dest) || is_mock_bech32(&watch.record_id) {
            return Err("mock_bech32");
        }
        if looks_invented(&watch.record_id) || looks_invented(&watch.dest) {
            return Err("invented_rpc");
        }
        dest_ok(watch.rail, &watch.dest)?;
        if watch.record_id.trim().is_empty() {
            return Err("missing_record_id");
        }
        let nf = dregg_payment_nullifier(watch.rail, &watch.record_id);
        self.issue_from_payment(watch.rail, &nf, None, &watch.record_id)
    }

    fn issue_from_payment(
        &mut self,
        rail: Rail,
        payment_nullifier: &str,
        tx_id: Option<&str>,
        seed: &str,
    ) -> Result<IssuedTicket, &'static str> {
        if !self
            .payment_nullifiers
            .insert(payment_nullifier.to_string())
        {
            return Err("nullifier_used");
        }
        if let Some(tx) = tx_id {
            self.tx_ids
                .insert(payment_nullifier.to_string(), tx.to_string());
        }
        let secret = issue_ticket_secret(self.issued as u64, payment_nullifier, seed);
        let leaf = ticket_leaf(&secret, SKU, EVENT_ID);
        let leaf_index = self.tree.append_bytes(leaf)?;
        self.issued += 1;
        Ok(IssuedTicket {
            ticket_secret: secret,
            leaf,
            guest_root: self.tree.root_bytes(),
            paid_n: self.issued,
            payment_nullifier: payment_nullifier.to_string(),
            rail,
            sku: SKU.into(),
            event_id: EVENT_ID.into(),
            leaf_index,
        })
    }

    pub fn merkle_path(&self, index: u64) -> Result<super::poseidon::MerklePath, &'static str> {
        self.tree.prove(index)
    }

    pub fn tree(&self) -> &PoseidonTree {
        &self.tree
    }
}

fn issue_ticket_secret(seq: u64, payment_nullifier: &str, seed: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"ticketSecret");
    h.update(seq.to_le_bytes());
    h.update(payment_nullifier.as_bytes());
    h.update(seed.as_bytes());
    h.finalize().into()
}

pub fn dregg_payment_nullifier(rail: Rail, record_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"dregg-v1");
    h.update(rail.as_str().as_bytes());
    h.update(record_id.as_bytes());
    format!("{:x}", h.finalize())
}

pub fn dest_ok(rail: Rail, dest: &str) -> Result<(), &'static str> {
    let s = dest.trim();
    match rail {
        Rail::Zec => {
            if zip321_ok(s) {
                Ok(())
            } else {
                Err("bad_zec_uri")
            }
        }
        Rail::Btc => {
            if s.starts_with("bc1") || s.starts_with("tb1") || s.starts_with("bcrt1") {
                Ok(())
            } else {
                Err("bad_btc_dest")
            }
        }
        Rail::Eth | Rail::Base => {
            if s.starts_with("0x") && s.len() == 42 && s[2..].chars().all(|c| c.is_ascii_hexdigit())
            {
                Ok(())
            } else {
                Err("bad_evm_dest")
            }
        }
        Rail::Penumbra => Err("penumbra_skipped"),
        Rail::CashApp => Err("cashapp_not_dest"),
    }
}

pub fn is_mock_bech32(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("mockbech32") || l.contains("mock1") || l.contains("bech32mock")
}

pub fn looks_invented(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("invented")
        || l.contains("fake-rpc")
        || l.contains("fakerpc")
        || l.starts_with("http://example")
        || l.contains("json-rpc-mock")
}

/// Instruct (ZIP-321 / HD dest) is not paid. Only `dregg_watch` commits.
pub fn dregg_instruct_commits_paid() -> bool {
    false
}

pub fn evidence_may_commit(kind: EvidenceKind) -> bool {
    matches!(
        kind,
        EvidenceKind::CashappReceipt | EvidenceKind::DreggWatch
    )
}

/// Browser `paid=true`, auto GET receipt, zkTLS — not a paid writer.
pub fn browser_paid_refused() -> &'static str {
    "browser_paid"
}

/// Second ETH/BTC/ZEC stack is forbidden. Reuse svg_mint_local_prod.
pub fn dregg_spawn_cmd() -> &'static str {
    SVG_MINT_LOCAL_PROD
}

pub fn is_second_stack_spawn(cmd: &str) -> bool {
    let l = cmd.to_ascii_lowercase();
    let mentions_chain = l.contains("bitcoin")
        || l.contains("zcash")
        || l.contains("geth")
        || l.contains("anvil")
        || l.contains("hardhat")
        || l.contains("ganache")
        || l.contains("bitcoind");
    let is_svg = l.contains("svg_mint_local_prod");
    mentions_chain && !is_svg
}

pub fn dregg_endpoint_ok(url: &str) -> bool {
    let l = url.to_lowercase();
    (l.contains("127.0.0.1") || l.contains("localhost")) && !looks_invented(url)
}

/// Live ict-rs Cash App path. Refuses ICT_MOCK=1.
pub fn cashapp_local_prod_cmd() -> &'static str {
    "cargo run --example itl_cashapp_local_prod -p ict-rs-cw-orch --features docker"
}

pub fn cashapp_chain_id() -> &'static str {
    CASHAPP_CHAIN_ID
}

pub fn ict_mock_refused(ict_mock: Option<&str>) -> bool {
    matches!(ict_mock, Some("1") | Some("true") | Some("yes"))
}

/// Public JSON must not leak Square mail fields.
pub fn public_json_is_clean(v: &Value) -> bool {
    let s = v.to_string();
    let lower = s.to_ascii_lowercase();
    let root = v
        .get("guest_root")
        .or_else(|| v.pointer("/update_guest_root/guest_root"))
        .and_then(|x| x.as_str());
    !lower.contains("txid")
        && !lower.contains("tx_id")
        && !lower.contains("\"amount")
        && !lower.contains("credid")
        && !lower.contains("cashtag")
        && !lower.contains("ticket_secret")
        && !lower.contains("ticketsecret")
        && root.map(|h| h.len() == 64) == Some(true)
}

/// WASIX parse helper for tests that inject Gap 0 mail.
pub fn parse_inject_eml(raw: &str) -> Value {
    wasix_parse_json(raw)
}

pub fn guest_root_contract() -> &'static str {
    CW_AVE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard_authz::itl_cashapp::synthetic_received_eml;

    #[test]
    fn public_json_omits_txid() {
        let mut e = IngressEngine::default();
        let o = e.create_cash_order("cred");
        let eml = synthetic_received_eml(&o.pay_ref, LIST_CENTS, "D-VR992KMXL");
        e.ingest_cashapp_eml(&eml).unwrap();
        assert!(public_json_is_clean(&e.public_sale_json()));
    }
}
