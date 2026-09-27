//! Cash App zkEmail ingress — in-process SoT for the ITL harness.
//! Same receipt JSON the Tickets plane fetches. Only mock is Payment received mail.
//! Nullifier: sha256("cashapp-v1" || txId) hex. payRef binds checkout. Not zkTLS.
//!
//! WASIX guest (`itl/cashapp/wasix`) is the sandboxed parse; this module is the
//! same regex/nullifier so onboard-authz local tests need no wasmer.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const NULLIFIER_PREFIX: &str = "cashapp-v1";
pub const LIST_CENTS: u32 = 2500;
pub const SKU: &str = "itl-speakeasy-oct18";
pub const CHECK_IN_GUEST: &str = "check_in_guest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReceipt {
    pub amount_cents: u32,
    pub pay_ref: String,
    pub tx_id: String,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub order_id: String,
    pub pay_ref: String,
    pub buyer_binding: String,
    pub amount_cents: u32,
    pub status: &'static str,
    pub tx_id: Option<String>,
    pub nullifier: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Engine {
    orders: HashMap<String, Order>,
    by_pay_ref: HashMap<String, String>,
    nullifiers: HashMap<String, String>,
    slots: HashMap<String, String>,
    seq: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            orders: HashMap::new(),
            by_pay_ref: HashMap::new(),
            nullifiers: HashMap::new(),
            slots: HashMap::new(),
            seq: 0,
        }
    }
}

pub fn nullifier_hex(tx_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(NULLIFIER_PREFIX.as_bytes());
    h.update(tx_id.as_bytes());
    format!("{:x}", h.finalize())
}

/// Gap 0 recipient HTML (tags already stripped to newlines).
pub fn parse_received_text(text: &str, subject: &str) -> Result<ParsedReceipt, &'static str> {
    if !subject.trim().eq_ignore_ascii_case("Payment received") {
        return Err("not_payment_received");
    }
    let amount_cents = parse_amount(text).ok_or("regex_miss")?;
    let pay_ref = parse_note(text).ok_or("regex_miss")?;
    let tx_id = parse_txid(text).ok_or("regex_miss")?;
    Ok(ParsedReceipt {
        amount_cents,
        pay_ref,
        tx_id,
    })
}

fn parse_amount(text: &str) -> Option<u32> {
    let i = text.find("+$")?;
    let rest = &text[i + 2..];
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let s = &rest[..end];
    let (d, c) = s.split_once('.')?;
    Some(d.parse::<u32>().ok()? * 100 + c.parse::<u32>().ok()?)
}

fn parse_note(text: &str) -> Option<String> {
    let i = text.find("\nFor ")?;
    let rest = &text[i + 5..];
    let end = rest.find('\n')?;
    let n = rest[..end].trim();
    if (4..=32).contains(&n.len())
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Some(n.to_string())
    } else {
        None
    }
}

fn parse_txid(text: &str) -> Option<String> {
    let i = text.find("Transaction number")?;
    let rest = &text[i..];
    let hash = rest.find("D-")?;
    let id = &rest[hash..hash + 11]; // D- + 9
    if id.len() == 11 && id.starts_with("D-") && id[2..].chars().all(|c| c.is_ascii_alphanumeric())
    {
        Some(id.to_string())
    } else {
        None
    }
}

/// Synthetic Payment received body (same fields as itl/cashapp/eml-template.mjs).
pub fn synthetic_received_html(pay_ref: &str, amount_cents: u32, tx_id: &str) -> String {
    let d = amount_cents / 100;
    let c = amount_cents % 100;
    format!("\n+${d}.{c:02}\nFor {pay_ref}\nTransaction number\n#{tx_id}\n")
}

/// Gap 0–shaped `.eml` (same fields as WASIX host `synthetic_received`).
pub fn synthetic_received_eml(pay_ref: &str, amount_cents: u32, tx_id: &str) -> String {
    let d = amount_cents / 100;
    let c = amount_cents % 100;
    let html = format!(
        "<html><body>\n<p>+${d}.{c:02}</p>\n<p>For {pay_ref}</p>\n<p>Transaction number</p>\n<p>#{tx_id}</p>\n</body></html>\n"
    );
    [
        "From: Cash App <cash@square.com>",
        "To: lab@itl.example",
        "Subject: Payment received",
        "MIME-Version: 1.0",
        "Content-Type: text/html; charset=UTF-8",
        "DKIM-Signature: v=1; a=rsa-sha256; c=relaxed/simple; d=square.com; s=lab; bh=lab; h=From:To:Subject:Content-Type;",
        "",
        html.as_str(),
    ]
    .join("\r\n")
}

pub fn synthetic_sent_eml(pay_ref: &str, amount_cents: u32, tx_id: &str) -> String {
    synthetic_received_eml(pay_ref, amount_cents, tx_id)
        .replace("Subject: Payment received", "Subject: Payment sent")
}

fn header_eml(raw: &str, name: &str) -> String {
    let prefix = format!("{name}:");
    for line in raw.lines() {
        if line.len() >= prefix.len() && line[..prefix.len()].eq_ignore_ascii_case(&prefix) {
            return line[prefix.len()..].trim().to_string();
        }
    }
    String::new()
}

fn dkim_d(sig: &str) -> String {
    for part in sig.split(';') {
        let p = part.trim();
        if let Some(v) = p.strip_prefix("d=") {
            return v.trim().to_string();
        }
    }
    String::new()
}

fn html_part(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let Some(i) = lower.find("content-type: text/html") else {
        return raw.to_string();
    };
    let slice = &raw[i..];
    let Some(blank) = slice.find("\r\n\r\n").or_else(|| slice.find("\n\n")) else {
        return String::new();
    };
    let skip = if slice[blank..].starts_with("\r\n\r\n") {
        4
    } else {
        2
    };
    let mut body = &slice[blank + skip..];
    if let Some(end) = body
        .find("\r\n------=_Part_")
        .or_else(|| body.find("\n------=_Part_"))
    {
        body = &body[..end];
    }
    let head = &slice[..blank.min(400)];
    if head.to_ascii_lowercase().contains("quoted-printable") {
        decode_qp(body)
    } else {
        body.to_string()
    }
}

fn decode_qp(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' && i + 1 < b.len() && (b[i + 1] == b'\n' || b[i + 1] == b'\r') {
            i += 1;
            if i < b.len() && b[i] == b'\r' {
                i += 1;
            }
            if i < b.len() && b[i] == b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i] == b'=' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(h) = h {
                if let Ok(v) = u8::from_str_radix(h, 16) {
                    out.push(v as char);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    while let Some(a) = s.find("<script") {
        if let Some(b) = s[a..].find("</script>") {
            s.replace_range(a..a + b + 9, " ");
        } else {
            break;
        }
    }
    while let Some(a) = s.find("<style") {
        if let Some(b) = s[a..].find("</style>") {
            s.replace_range(a..a + b + 8, " ");
        } else {
            break;
        }
    }
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push('\n');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Same parse as the WASIX guest (`itl/cashapp/wasix/guest`). Host still no net.
pub fn parse_eml_wasix(raw: &str) -> Result<ParsedReceipt, &'static str> {
    let subject = header_eml(raw, "Subject");
    let dkim = header_eml(raw, "DKIM-Signature");
    let d = dkim_d(&dkim);
    if !d.is_empty() && d != "square.com" {
        return Err("dkim_domain");
    }
    let text = html_to_text(&html_part(raw));
    parse_received_text(&text, &subject)
}

/// Guest stdout JSON. `proven: false` — merchant attest, not zkEmail Groth16.
pub fn wasix_parse_json(raw: &str) -> Value {
    match parse_eml_wasix(raw) {
        Ok(p) => json!({
            "ok": true,
            "payRef": p.pay_ref,
            "amountCents": p.amount_cents,
            "txId": p.tx_id,
            "nullifier": nullifier_hex(&p.tx_id),
            "dkimDomain": "square.com",
            "proven": false,
        }),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

impl Engine {
    pub fn create_order(&mut self, buyer: &str) -> Order {
        self.seq += 1;
        let pay_ref = format!("cmt_{:05}", self.seq);
        let order = Order {
            order_id: format!("ord_{}", self.seq),
            pay_ref: pay_ref.clone(),
            buyer_binding: buyer.into(),
            amount_cents: LIST_CENTS,
            status: "pending_payment",
            tx_id: None,
            nullifier: None,
        };
        self.orders.insert(order.order_id.clone(), order.clone());
        self.by_pay_ref.insert(pay_ref, order.order_id.clone());
        order
    }

    pub fn ingest_text(&mut self, text: &str, subject: &str) -> Result<Order, &'static str> {
        let p = parse_received_text(text, subject)?;
        self.ingest_parsed(p)
    }

    /// Gap 0 inject → WASIX parse → payment nullifier.
    pub fn ingest_eml(&mut self, raw: &str) -> Result<Order, &'static str> {
        let p = parse_eml_wasix(raw)?;
        self.ingest_parsed(p)
    }

    fn ingest_parsed(&mut self, p: ParsedReceipt) -> Result<Order, &'static str> {
        let nf = nullifier_hex(&p.tx_id);
        if self.nullifiers.contains_key(&nf) {
            return Err("nullifier_used");
        }
        let oid = self
            .by_pay_ref
            .get(&p.pay_ref)
            .ok_or("unknown_payRef")?
            .clone();
        let order = self.orders.get_mut(&oid).ok_or("unknown_payRef")?;
        if p.amount_cents < order.amount_cents {
            return Err("amount_too_low");
        }
        self.nullifiers.insert(nf.clone(), oid.clone());
        order.status = "paid";
        order.tx_id = Some(p.tx_id);
        order.nullifier = Some(nf);
        Ok(order.clone())
    }

    pub fn book_slot(&mut self, kind: &str, start: &str, guest: &str) -> Result<(), &'static str> {
        let key = format!("{kind}|{start}");
        if self.slots.contains_key(&key) {
            return Err("slot_taken");
        }
        self.slots.insert(key, guest.into());
        Ok(())
    }

    pub fn paid_count(&self) -> usize {
        self.orders.values().filter(|o| o.status == "paid").count()
    }
}

/// Receipt JSON the Next Tickets plane already fetches (no guest cashtag).
pub fn receipt_json(pay_ref: &str, amount_cents: u32, tx_id: &str) -> serde_json::Value {
    serde_json::json!({
        "payRef": pay_ref,
        "amountCents": amount_cents,
        "txId": tx_id,
        "nullifier": nullifier_hex(tx_id),
        "proven": false,
        "dkimDomain": "square.com",
    })
}
