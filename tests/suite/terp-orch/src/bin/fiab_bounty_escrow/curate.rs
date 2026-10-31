//! `curate`: approve / reject / cancel escrow bounties by id, as FIAB core, through the member's
//! live grant (FIAB proposal 4 keys approve/reject/cancel), wrapped by the cw-orch-kms session:
//! MsgExec(session){ MsgExec(member){ MsgExecuteContract(sender = FIAB core, escrow, <action>) } }.
//! Status is read from chain (LCD smart query, read-only) before any msg is built.
//! Contract rules (bounty-escrow src/contract.rs exec_approve/exec_reject/exec_cancel):
//!   approve: approver only, Pending -> Open, not expired
//!   reject:  approver only, Pending -> Refunded (full refund to proposer)
//!   cancel:  approver: Pending|Open|Assigned -> Cancelled (full refund to proposer)

use super::seed::{SESSION_PLACEHOLDER, limits_from, load_manifest};
use super::{Plan, SessionKind, T_AUTHZ_EXEC, T_EXEC, any, hexenc};
use anyhow::{Result, anyhow, bail};
use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec;
use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgExecuteContract;
use cosmos_sdk_proto::prost::Message as _;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { Approve, Reject, Cancel }

impl Action {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s { "approve" => Self::Approve, "reject" => Self::Reject, "cancel" => Self::Cancel,
            _ => bail!("action must be approve|reject|cancel (got {s})") })
    }
    pub fn key(self) -> &'static str {
        match self { Self::Approve => "approve", Self::Reject => "reject", Self::Cancel => "cancel" }
    }
    /// Statuses the approver (FIAB core) may act on.
    pub fn allowed(self) -> &'static [&'static str] {
        match self { Self::Approve | Self::Reject => &["pending"], Self::Cancel => &["pending", "open", "assigned"] }
    }
    pub fn msg(self, id: u64, reason: &Option<String>) -> Value {
        match self {
            Self::Approve => json!({"approve": {"id": id}}),
            Self::Reject => json!({"reject": {"id": id, "reason": reason}}),
            Self::Cancel => json!({"cancel": {"id": id}}),
        }
    }
}

/// What we know about one bounty before acting on it.
#[derive(Clone, Debug)]
pub struct BountyState { pub id: u64, pub status: String, pub bounty_ref: String, pub expires_at_unix: i64 }

fn from_chain(b: &Value) -> Result<BountyState> {
    let ns: i128 = b["expires_at"].as_str().unwrap_or("0").parse()?;
    Ok(BountyState {
        id: b["id"].as_u64().ok_or_else(|| anyhow!("bounty without id"))?,
        status: b["status"].as_str().ok_or_else(|| anyhow!("bounty without status"))?.to_string(),
        bounty_ref: b["bounty_ref"].as_str().unwrap_or_default().to_string(),
        expires_at_unix: (ns / 1_000_000_000) as i64,
    })
}

pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { o.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' }); }
    }
    o
}

/// Read-only wasm smart query over the public LCD (curl; no keys involved).
fn smart(lcd: &str, contract: &str, q: &Value) -> Result<Value> {
    let url = format!("{}/cosmwasm/wasm/v1/contract/{contract}/smart/{}", lcd.trim_end_matches('/'),
        b64(serde_json::to_string(q)?.as_bytes()));
    let out = std::process::Command::new("curl").args(["-sf", "--max-time", "20", &url]).output()?;
    if !out.status.success() { bail!("LCD query failed ({}): {q}", out.status); }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    v.get("data").cloned().ok_or_else(|| anyhow!("LCD reply without data: {v}"))
}

pub fn chain_states(lcd: &str, escrow: &str, ids: &[u64], all_pending: bool) -> Result<Vec<BountyState>> {
    let mut out = vec![];
    for id in ids { out.push(from_chain(&smart(lcd, escrow, &json!({"bounty": {"id": id}}))?)?); }
    if all_pending {
        let mut after: Option<u64> = None;
        loop {
            let r = smart(lcd, escrow, &json!({"bounties": {"status": "pending", "start_after": after, "limit": 30}}))?;
            let page = r["bounties"].as_array().cloned().unwrap_or_default();
            if page.is_empty() { break; }
            for b in &page { let s = from_chain(b)?; after = Some(s.id); if !out.iter().any(|x: &BountyState| x.id == s.id) { out.push(s); } }
        }
    }
    Ok(out)
}

/// PREVIEW-ONLY assumption: a fresh escrow where the seed manifest was created in order, ids 1..N, all Pending.
pub fn assumed_fresh(pack: &Path, pv: &str, ids: &[u64], all_pending: bool) -> Result<Vec<BountyState>> {
    let (m, _) = load_manifest(pack, pv)?;
    let all: Vec<BountyState> = m["bounties"].as_array().unwrap().iter().enumerate().map(|(i, e)| BountyState {
        id: i as u64 + 1, status: "pending".into(), bounty_ref: e["id"].as_str().unwrap_or_default().into(),
        expires_at_unix: e["expires_at_unix"].as_i64().unwrap_or(0) }).collect();
    if all_pending { return Ok(all); }
    ids.iter().map(|id| all.iter().find(|b| b.id == *id).cloned().ok_or_else(|| anyhow!("id {id} not in assumed seed"))).collect()
}

/// Refuse anything the contract would reject, and anything outside the seed manifest when one is given.
pub fn validate(action: Action, states: &[BountyState], only_refs: Option<&[String]>, now_unix: i64) -> Result<()> {
    if states.is_empty() { bail!("no bounties selected"); }
    let mut seen = std::collections::BTreeSet::new();
    for b in states {
        if !seen.insert(b.id) { bail!("bounty {} selected twice", b.id); }
        if !action.allowed().contains(&b.status.as_str()) {
            bail!("bounty {} ({}) is {}; {} needs {:?}", b.id, b.bounty_ref, b.status, action.key(), action.allowed());
        }
        if action == Action::Approve && b.expires_at_unix <= now_unix { bail!("bounty {} already expired", b.id); }
        if let Some(refs) = only_refs { if !refs.contains(&b.bounty_ref) { bail!("bounty {} ref {:?} not in the seed manifest", b.id, b.bounty_ref); } }
    }
    Ok(())
}

pub fn inner_exec(core: &str, member: &str, escrow: &str, action: Action, reason: &Option<String>, states: &[BountyState]) -> Result<(MsgExec, Vec<Value>)> {
    let (mut anys, mut views) = (vec![], vec![]);
    for b in states {
        let m = action.msg(b.id, reason);
        let x = MsgExecuteContract { sender: core.into(), contract: escrow.into(), msg: serde_json::to_vec(&m)?, funds: vec![] };
        anys.push(any(T_EXEC, &x));
        views.push(json!({"@type": T_EXEC, "sender": core, "contract": escrow, "msg": m, "funds": []}));
    }
    Ok((MsgExec { grantee: member.into(), msgs: anys }, views))
}

#[allow(clippy::too_many_arguments)]
pub fn plans(cfg: &Value, core: &str, member: &str, escrow: &str, action: Action, reason: &Option<String>,
    states: &[BountyState], batch_size: usize, spent_calls: u64, source: &str) -> Result<Vec<Plan>> {
    let g = if cfg["live_grant"].is_object() { &cfg["live_grant"] } else { &cfg["grant"] };
    let keys: Vec<&str> = g["accepted_keys"].as_array().map(|a| a.iter().filter_map(|k| k.as_str()).collect()).unwrap_or_default();
    if !keys.contains(&action.key()) { bail!("grant does not accept {}: keys {keys:?}", action.key()); }
    let lim = limits_from(cfg)?;
    if spent_calls + states.len() as u64 > lim.calls { bail!("calls {spent_calls} + {} > grant calls {}", states.len(), lim.calls); }
    let mut out = vec![];
    for (i, chunk) in states.chunks(batch_size.max(1)).enumerate() {
        let (exec, views) = inner_exec(core, member, escrow, action, reason, chunk)?;
        let ids: Vec<u64> = chunk.iter().map(|b| b.id).collect();
        let tx = json!({
            "body": {"messages": [{"@type": T_AUTHZ_EXEC, "grantee": SESSION_PLACEHOLDER,
                "msgs": [{"@type": T_AUTHZ_EXEC, "grantee": member, "msgs": views}]}],
                "memo": format!("fiab-bounty-escrow curate {} batch {}", action.key(), i + 1),
                "timeout_height": "0", "extension_options": [], "non_critical_extension_options": []},
            "auth_info": {"signer_infos": [{"public_key": SESSION_PLACEHOLDER, "mode_info": {"single": {"mode": "SIGN_MODE_DIRECT"}}, "sequence": "0"}],
                "fee": {"amount": "<from simulate>", "gas_limit": "<from simulate>", "payer": "", "granter": member}},
            "signatures": [], "inner_msg_exec_proto_hex": hexenc(exec.encode_to_vec()),
        });
        out.push(Plan {
            step: format!("06-curate-{}-batch-{:02}", action.key(), i + 1),
            signer: member.into(),
            note: format!("nested: MsgExec(session){{MsgExec(member){{{} x {}(core)}}}}; ids {ids:?}; status source: {source}", chunk.len(), action.key()),
            session: json!({"nested_exec": true, "type_urls": [T_AUTHZ_EXEC], "status_source": source,
                "targets": chunk.iter().map(|b| json!({"id": b.id, "bounty_ref": b.bounty_ref, "status_before": b.status})).collect::<Vec<_>>(),
                "unsigned_tx": tx}),
            msgs: vec![(any(T_AUTHZ_EXEC, &exec), json!({"@type": T_AUTHZ_EXEC, "grantee": member, "msgs": views}))],
            kind: SessionKind::NestedExec,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_sdk_proto::prost::Message;

    fn st(id: u64, s: &str) -> BountyState { BountyState { id, status: s.into(), bounty_ref: format!("TB-{id}"), expires_at_unix: 2_000_000_000 } }
    fn cfg() -> Value {
        json!({"live_grant": {"calls_remaining": 200, "accepted_keys": ["create_bounty", "approve", "reject", "cancel"],
            "amounts": [{"denom": "uterp", "amount": "50000000000"}]},
            "instantiate_msg": {"denoms": [{"denom": "uterp", "min_amount": "1", "max_amount": "2"}]}})
    }

    #[test]
    fn status_rules_match_contract() {
        let now = 1_900_000_000;
        assert!(validate(Action::Approve, &[st(1, "pending")], None, now).is_ok());
        assert!(validate(Action::Approve, &[st(1, "open")], None, now).is_err());
        assert!(validate(Action::Reject, &[st(1, "open")], None, now).is_err());
        for s in ["pending", "open", "assigned"] { assert!(validate(Action::Cancel, &[st(1, s)], None, now).is_ok()); }
        for s in ["submitted", "accepted", "paid", "cancelled", "refunded", "expired"] { assert!(validate(Action::Cancel, &[st(1, s)], None, now).is_err()); }
        let mut e = st(2, "pending"); e.expires_at_unix = now;
        assert!(validate(Action::Approve, &[e.clone()], None, now).is_err());
        assert!(validate(Action::Reject, &[e], None, now).is_ok());
        assert!(validate(Action::Approve, &[st(1, "pending"), st(1, "pending")], None, now).is_err());
        assert!(validate(Action::Approve, &[st(3, "pending")], Some(&["TB-1".to_string()]), now).is_err());
        assert!(validate(Action::Approve, &[], None, now).is_err());
        assert!(Action::parse("payout").is_err());
    }

    #[test]
    fn curate_msg_encoding_nested_no_funds() {
        let r = Some("dup".to_string());
        let (x, v) = inner_exec("core", "member", "escrow", Action::Reject, &r, &[st(7, "pending")]).unwrap();
        assert_eq!(x.grantee, "member");
        let m = MsgExecuteContract::decode(x.msgs[0].value.as_slice()).unwrap();
        assert_eq!((m.sender.as_str(), m.contract.as_str()), ("core", "escrow"));
        assert!(m.funds.is_empty());
        assert_eq!(serde_json::from_slice::<Value>(&m.msg).unwrap(), json!({"reject": {"id": 7, "reason": "dup"}}));
        assert_eq!(v[0]["msg"], json!({"reject": {"id": 7, "reason": "dup"}}));
        assert_eq!(Action::Approve.msg(3, &None), json!({"approve": {"id": 3}}));
        assert_eq!(Action::Cancel.msg(3, &None), json!({"cancel": {"id": 3}}));
    }

    #[test]
    fn plans_respect_grant_keys_and_calls() {
        let s: Vec<_> = (1..=20).map(|i| st(i, "pending")).collect();
        let p = plans(&cfg(), "c", "m", "e", Action::Approve, &None, &s, 10, 0, "test").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].step, "06-curate-approve-batch-01");
        assert!(plans(&cfg(), "c", "m", "e", Action::Approve, &None, &s, 10, 190, "test").is_err());
        let mut c = cfg(); c["live_grant"]["accepted_keys"] = json!(["create_bounty"]);
        assert!(plans(&c, "c", "m", "e", Action::Cancel, &None, &s, 10, 0, "test").is_err());
    }

    #[test]
    fn base64_vectors() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(br#"{"config":{}}"#), "eyJjb25maWciOnt9fQ==");
    }
}
