//! ITL FROST 20/80 pay — in-process SoT for the product harness.
//! Artists/vendors: transparent t-addr (grant accountability). 20% upfront, 80% after event.
//! Paid ticket inflow required before any 20% job. Inbox holds shares until t-of-n; then broadcast.
//! No KeyPackages. Passkey is not a share. SIGN/C/S is frost-spend wire, not a stub txid.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const FROST_T: u16 = 2;
pub const FROST_N: u16 = 3;
pub const DKG_TAG_R1: &str = "dkg1";
pub const DKG_TAG_R2: &str = "dkg2";
/// Domain 1: ZIP-244 / SpendAuth over the PCZT (the transfer itself).
pub const DOMAIN_ZEC_SPEND_V0: &[u8] = b"terp/frost_escrow/zec_spend/v0";
/// Domain 2: holder mempool (who currently holds FROST shares). Bound in tx memo.
pub const ROSTER_DOMAIN: &[u8] = b"itl-frost-roster-v1";
pub const SPEND_MEMO_PREFIX: &str = "itl-frost-memo-v1:";
pub const CHECK_IN_GUEST: &str = "check_in_guest";

/// Canonical hash of the live holder set. Spends must carry this in the memo
/// so FROST signatures attest roster consensus — not only the HashMerchant proxy.
pub fn roster_commit(epoch: u32, t: u16, n: u16, group_vk: &str, seats: &[String]) -> String {
    let mut seats: Vec<&str> = seats.iter().map(|s| s.as_str()).collect();
    seats.sort_unstable();
    let mut h = Sha256::new();
    h.update(ROSTER_DOMAIN);
    h.update(epoch.to_be_bytes());
    h.update(t.to_be_bytes());
    h.update(n.to_be_bytes());
    h.update(group_vk.as_bytes());
    h.update([0u8]);
    for s in seats {
        h.update(s.as_bytes());
        h.update([0u8]);
    }
    hex32(&h.finalize())
}

pub fn spend_memo_roster(roster_commit_hex: &str) -> String {
    format!("{SPEND_MEMO_PREFIX}{roster_commit_hex}")
}

pub fn parse_spend_memo_roster(memo: &str) -> Result<&str, &'static str> {
    let m = memo.trim();
    let rest = m
        .strip_prefix(SPEND_MEMO_PREFIX)
        .ok_or("memo_missing_roster")?;
    if rest.len() != 64 || !rest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("memo_bad_roster");
    }
    Ok(rest)
}

pub fn require_spend_memo_roster(memo: &str, expected_commit: &str) -> Result<(), &'static str> {
    let got = parse_spend_memo_roster(memo)?;
    if got != expected_commit {
        return Err("memo_mismatch");
    }
    Ok(())
}

fn hex32(bytes: &[u8]) -> String {
    const H: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0xf) as usize] as char);
    }
    s
}

/// HashMerchant inbox scheme (zk-wasmvm analog: pick curve/prover).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrostScheme {
    RedPallas,
    Ed25519Consensus,
    Bls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BroadcastTarget {
    Zcash,
    Penumbra,
    SmartAccountExecute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayeeKind {
    Artist,
    Vendor,
    Attendee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayoutLeg {
    #[serde(rename = "20")]
    Twenty,
    #[serde(rename = "80")]
    Eighty,
    #[serde(rename = "refund")]
    Refund,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayRecord {
    pub code: String,
    pub kind: PayeeKind,
    pub name: String,
    pub taddr: String,
    pub leg: PayoutLeg,
    pub amount_label: String,
    pub attest_url: Option<String>,
    pub txid: Option<String>,
}

/// Confirmed ticket/gallery payment that funds the event pool (other lane writes this).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketInflow {
    pub record_id: String,
    pub rail: String,
    pub amount_label: String,
}

#[derive(Debug, Default)]
pub struct Ledger {
    records: Vec<PayRecord>,
    inflows: Vec<TicketInflow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRow {
    pub kind: PayeeKind,
    pub name: String,
    pub taddr: String,
    pub up: Vec<String>,
    pub after: Vec<String>,
}

impl Ledger {
    pub fn confirm_inflow(&mut self, inf: TicketInflow) -> Result<(), &'static str> {
        if inf.record_id.trim().is_empty() {
            return Err("inflow_unconfirmed");
        }
        if self.inflows.iter().any(|i| i.record_id == inf.record_id) {
            return Ok(());
        }
        self.inflows.push(inf);
        Ok(())
    }

    pub fn has_confirmed_inflow(&self) -> bool {
        !self.inflows.is_empty()
    }

    pub fn inflows(&self) -> &[TicketInflow] {
        &self.inflows
    }

    pub fn enqueue(&mut self, rec: PayRecord) -> Result<(), &'static str> {
        require_transparent_taddr(&rec.taddr)?;
        if rec.kind != PayeeKind::Attendee
            && rec
                .attest_url
                .as_ref()
                .map(|s| s.is_empty())
                .unwrap_or(true)
        {
            return Err("attest_required");
        }
        if rec.leg == PayoutLeg::Twenty && !self.has_confirmed_inflow() {
            return Err("inflow_unconfirmed");
        }
        if rec.leg == PayoutLeg::Eighty {
            let ok = self.records.iter().any(|r| {
                r.name == rec.name
                    && r.kind == rec.kind
                    && r.leg == PayoutLeg::Twenty
                    && r.txid.is_some()
            });
            if !ok {
                return Err("eighty_before_twenty");
            }
        }
        if rec.txid.is_some() {
            return Err("queued_has_txid");
        }
        self.records.push(rec);
        Ok(())
    }

    pub fn mark_broadcast(&mut self, code: &str, txid: &str) -> Result<(), &'static str> {
        let rec = self
            .records
            .iter_mut()
            .find(|r| r.code == code)
            .ok_or("unknown_code")?;
        if rec.txid.is_some() {
            return Err("already_broadcast");
        }
        rec.txid = Some(txid.to_string());
        Ok(())
    }

    pub fn records(&self) -> &[PayRecord] {
        &self.records
    }

    pub fn grant_rollup(&self) -> Vec<GrantRow> {
        grant_rollup(&self.records)
    }
}

/// Hold admin shares until t distinct admins; do not ingress until then.
/// Blobs are opaque (frost-spend verifies on the live path). This is inbox *policy*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Threshold,
    Broadcast,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingJob {
    pub id: String,
    pub pay_code: String,
    pub t: u16,
    pub n: u16,
    pub scheme: FrostScheme,
    pub target: BroadcastTarget,
    pub status: JobStatus,
    pub txid: Option<String>,
    /// Domain 2: holder-mempool commit all shares must bind. Empty = unbound (cannot pay).
    #[serde(default)]
    pub roster_commit: String,
    #[serde(default)]
    shares: BTreeMap<String, String>,
}

impl PendingJob {
    pub fn open(id: &str, pay_code: &str, t: u16, n: u16) -> Self {
        Self::open_for(
            id,
            pay_code,
            t,
            n,
            FrostScheme::RedPallas,
            BroadcastTarget::Zcash,
        )
    }

    pub fn open_for(
        id: &str,
        pay_code: &str,
        t: u16,
        n: u16,
        scheme: FrostScheme,
        target: BroadcastTarget,
    ) -> Self {
        Self {
            id: id.into(),
            pay_code: pay_code.into(),
            t,
            n,
            scheme,
            target,
            status: JobStatus::Pending,
            txid: None,
            roster_commit: String::new(),
            shares: BTreeMap::new(),
        }
    }

    /// Pay job bound to a holder-mempool commit (required to move funds).
    pub fn open_bound(
        id: &str,
        pay_code: &str,
        t: u16,
        n: u16,
        roster_commit: &str,
    ) -> Result<Self, &'static str> {
        let c = roster_commit.trim().to_ascii_lowercase();
        if c.len() != 64 || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err("roster_commit_required");
        }
        let mut j = Self::open(id, pay_code, t, n);
        j.roster_commit = c;
        Ok(j)
    }

    pub fn share_count(&self) -> usize {
        self.shares.len()
    }

    pub fn admin_ids(&self) -> BTreeSet<&str> {
        self.shares.keys().map(|s| s.as_str()).collect()
    }

    /// Hold. Does not broadcast. Same admin twice is replay.
    pub fn submit_share(&mut self, admin_id: &str, blob: &str) -> Result<JobStatus, &'static str> {
        if self.status == JobStatus::Broadcast {
            return Err("already_broadcast");
        }
        if admin_id.trim().is_empty() || blob.trim().is_empty() {
            return Err("empty_share");
        }
        if blob.to_ascii_lowercase().contains("keypackage") {
            return Err("keypackage_rejected");
        }
        if self.shares.contains_key(admin_id) {
            return Err("duplicate_admin");
        }
        if self.shares.len() as u16 >= self.n {
            return Err("over_n");
        }
        self.shares.insert(admin_id.into(), blob.into());
        if self.shares.len() as u16 >= self.t {
            self.status = JobStatus::Threshold;
        }
        Ok(self.status)
    }

    /// Share bound to a roster commit. First bind sets the job mempool; mismatch is split-view.
    pub fn submit_share_bound(
        &mut self,
        admin_id: &str,
        blob: &str,
        roster_commit: &str,
    ) -> Result<JobStatus, &'static str> {
        let c = roster_commit.trim().to_ascii_lowercase();
        if c.len() != 64 || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err("roster_commit_required");
        }
        if self.roster_commit.is_empty() {
            self.roster_commit = c;
        } else if self.roster_commit != c {
            return Err("roster_split");
        }
        self.submit_share(admin_id, blob)
    }

    /// Only after threshold. Pay jobs must have holder-mempool consensus (roster_commit).
    pub fn broadcast(&mut self, txid: &str) -> Result<(), &'static str> {
        if self.status != JobStatus::Threshold {
            return Err("below_threshold");
        }
        if self.scheme != FrostScheme::RedPallas {
            return Err("scheme_not_for_pay");
        }
        if self.target != BroadcastTarget::Zcash {
            return Err("target_not_for_pay");
        }
        if self.roster_commit.is_empty() {
            return Err("roster_commit_required");
        }
        if txid.trim().is_empty() {
            return Err("no_txid");
        }
        self.txid = Some(txid.into());
        self.status = JobStatus::Broadcast;
        self.shares.clear(); // drop blobs after broadcast (memory)
        Ok(())
    }

    pub fn spend_memo(&self) -> Result<String, &'static str> {
        if self.roster_commit.is_empty() {
            return Err("roster_commit_required");
        }
        Ok(spend_memo_roster(&self.roster_commit))
    }

    /// Live path: reject fixtures / premature mempool strings; require a node-confirmed txid.
    pub fn broadcast_node_confirmed(&mut self, txid: &str, rpc_url: &str) -> Result<(), &'static str> {
        if self.status != JobStatus::Threshold {
            return Err("below_threshold");
        }
        require_node_txid(txid)?;
        confirm_txid_on_node(rpc_url, txid)?;
        self.broadcast(txid)
    }
}

pub fn grant_rollup(records: &[PayRecord]) -> Vec<GrantRow> {
    let mut out: Vec<GrantRow> = Vec::new();
    for r in records {
        if let Some(row) = out
            .iter_mut()
            .find(|x| x.taddr == r.taddr && x.kind == r.kind && x.name == r.name)
        {
            match r.leg {
                PayoutLeg::Twenty => row
                    .up
                    .push(r.txid.clone().unwrap_or_else(|| "queued".into())),
                PayoutLeg::Eighty => row
                    .after
                    .push(r.txid.clone().unwrap_or_else(|| "queued".into())),
                PayoutLeg::Refund => {}
            }
        } else {
            let mut row = GrantRow {
                kind: r.kind,
                name: r.name.clone(),
                taddr: r.taddr.clone(),
                up: vec![],
                after: vec![],
            };
            match r.leg {
                PayoutLeg::Twenty => row
                    .up
                    .push(r.txid.clone().unwrap_or_else(|| "queued".into())),
                PayoutLeg::Eighty => row
                    .after
                    .push(r.txid.clone().unwrap_or_else(|| "queued".into())),
                PayoutLeg::Refund => {}
            }
            out.push(row);
        }
    }
    out
}

/// Browser / Next session must never carry FROST share material.
pub fn session_json_ok(json: &str) -> bool {
    let lower = json.to_ascii_lowercase();
    !lower.contains("keypackage")
        && !lower.contains("secret_share")
        && !lower.contains("signingshare")
        && !lower.contains("packages")
        && !json.contains("S:0:")
}

pub fn sign_line(
    sighash_hex: &str,
    alphas_csv: &str,
    recipient: &str,
    amount_zat: &str,
    fee_zat: &str,
    pczt_hex: &str,
) -> String {
    format!("SIGN:{sighash_hex}:{alphas_csv}:{recipient}:{amount_zat}:{fee_zat}:{pczt_hex}")
}

pub fn commit_line(commits: &[&str]) -> String {
    format!("C:{}", commits.join("|"))
}

pub fn share_line(action_idx: u32, share: &str) -> String {
    format!("S:{action_idx}:{share}")
}

pub fn parse_sign_recipient(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("SIGN:")?;
    rest.split(':').nth(2)
}

fn taddr_ink() -> String {
    lab_taddr(1)
}

/// Demo: confirmed ticket inflow, artist 20 broadcast, 80 queued, vendor 20 queued.
pub fn local_twenty_eighty() -> Ledger {
    let mut l = Ledger::default();
    l.confirm_inflow(TicketInflow {
        record_id: "ticket-cashapp-lab".into(),
        rail: "cashapp".into(),
        amount_label: "list".into(),
    })
    .expect("inflow");
    l.enqueue(PayRecord {
        code: "pay-20-ink".into(),
        kind: PayeeKind::Artist,
        name: "ink".into(),
        taddr: taddr_ink(),
        leg: PayoutLeg::Twenty,
        amount_label: "20% upfront".into(),
        attest_url: Some("https://example.invalid/attest/ink".into()),
        txid: None,
    })
    .expect("20 ink");
    l.mark_broadcast("pay-20-ink", "txid-20-ink")
        .expect("bc 20");
    l.enqueue(PayRecord {
        code: "pay-80-ink".into(),
        kind: PayeeKind::Artist,
        name: "ink".into(),
        taddr: taddr_ink(),
        leg: PayoutLeg::Eighty,
        amount_label: "80% after event".into(),
        attest_url: Some("https://example.invalid/attest/ink".into()),
        txid: None,
    })
    .expect("80 ink");
    l.enqueue(PayRecord {
        code: "pay-20-kitchen".into(),
        kind: PayeeKind::Vendor,
        name: "kitchen".into(),
        taddr: lab_taddr(2),
        leg: PayoutLeg::Twenty,
        amount_label: "20% upfront".into(),
        attest_url: Some("https://example.invalid/attest/kitchen".into()),
        txid: None,
    })
    .expect("20 vendor");
    l
}

/// Sealed persist. Missing/short key => ceremony stranded.
pub fn persist_required() -> Result<(), &'static str> {
    match std::env::var("ITL_FROST_PERSIST_KEY") {
        Ok(k) if k.len() >= 16 => Ok(()),
        _ => Err("persistence_off"),
    }
}

const B58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn b58_encode(data: &[u8]) -> String {
    let mut zeros = 0usize;
    while zeros < data.len() && data[zeros] == 0 {
        zeros += 1;
    }
    let mut acc = vec![0u8; data.len() * 2];
    let mut len = 0usize;
    for &b in data {
        let mut carry = b as u32;
        for digit in acc.iter_mut().take(len) {
            carry += (*digit as u32) * 256;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            acc[len] = (carry % 58) as u8;
            len += 1;
            carry /= 58;
        }
    }
    let mut out = "1".repeat(zeros);
    for i in (0..len).rev() {
        out.push(B58[acc[i] as usize] as char);
    }
    out
}

fn sha256d(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let h1 = Sha256::digest(data);
    let h2 = Sha256::digest(&h1);
    let mut o = [0u8; 32];
    o.copy_from_slice(&h2);
    o
}


/// 32-byte hex txid from the node. Rejects fixtures, dummy 0xaa.., and short labels.
pub fn require_node_txid(txid: &str) -> Result<(), &'static str> {
    let t = txid.trim();
    let t = t.strip_prefix("0x").unwrap_or(t);
    if t.is_empty() {
        return Err("no_txid");
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("txid") || lower.contains("fixture") || lower.contains("dummy") {
        return Err("fixture_txid");
    }
    if t.len() != 64 {
        return Err("txid_len");
    }
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("txid_hex");
    }
    if lower.chars().all(|c| c == 'a') {
        return Err("fixture_txid");
    }
    Ok(())
}

/// `getrawtransaction` with at least one confirmation. Mempool (0 conf) is premature.
pub fn confirm_txid_on_node(rpc_url: &str, txid: &str) -> Result<(), &'static str> {
    require_node_txid(txid)?;
    let url = rpc_url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err("no_rpc");
    }
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getrawtransaction\",\"params\":[\"{}\",1]}}",
        txid.trim().trim_start_matches("0x")
    );
    let out = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "8",
            "-X",
            "POST",
            &format!("{url}/"),
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ])
        .output()
        .map_err(|_| "rpc_curl")?;
    if !out.status.success() {
        return Err("rpc_fail");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|_| "rpc_json")?;
    if v.get("error").map(|e| !e.is_null()).unwrap_or(false) {
        return Err("rpc_error");
    }
    let result = v.get("result").ok_or("rpc_no_result")?;
    if result.is_null() {
        return Err("not_on_node");
    }
    let conf = result
        .get("confirmations")
        .and_then(|c| c.as_u64())
        .unwrap_or(0);
    if conf < 1 {
        return Err("premature");
    }
    Ok(())
}

/// Zcash mainnet P2PKH version 0x1CB8 → t1…
pub fn encode_t1(payload20: [u8; 20]) -> String {
    let mut d = vec![0x1c, 0xb8];
    d.extend_from_slice(&payload20);
    let cs = sha256d(&d);
    d.extend_from_slice(&cs[..4]);
    b58_encode(&d)
}

pub fn lab_taddr(tag: u8) -> String {
    let mut p = [0u8; 20];
    p[0] = tag;
    encode_t1(p)
}

fn b58_decode(s: &str) -> Option<Vec<u8>> {
    let mut acc: Vec<u8> = Vec::new();
    for c in s.chars() {
        let v = B58.iter().position(|&b| b == c as u8)?;
        let mut carry = v as u32;
        for d in acc.iter_mut() {
            carry += (*d as u32) * 58;
            *d = (carry % 256) as u8;
            carry /= 256;
        }
        while carry > 0 {
            acc.push((carry % 256) as u8);
            carry /= 256;
        }
    }
    acc.reverse();
    let zeros = s.chars().take_while(|&c| c == '1').count();
    let mut out = vec![0u8; zeros];
    out.extend(acc);
    Some(out)
}

/// Prefix + base58check. Rejects u1/zs1 and bad checksum.
pub fn require_transparent_taddr(taddr: &str) -> Result<(), &'static str> {
    let t = taddr.trim();
    if t.is_empty() {
        return Err("no_taddr");
    }
    if t.starts_with("u1") || t.starts_with("zs1") || t.starts_with("tex1") {
        return Err("not_transparent");
    }
    if !(t.starts_with("t1") || t.starts_with("t3") || t.starts_with("tm")) {
        return Err("not_transparent");
    }
    let raw = b58_decode(t).ok_or("not_transparent")?;
    if raw.len() < 6 {
        return Err("not_transparent");
    }
    let (body, cs) = raw.split_at(raw.len() - 4);
    let want = sha256d(body);
    if cs != &want[..4] {
        return Err("bad_checksum");
    }
    Ok(())
}
