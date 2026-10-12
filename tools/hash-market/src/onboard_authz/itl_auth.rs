//! ITL Speakeasy auth as a cw-orch / ict-rs product surface.
//!
//! Local-first: in-process add → query → AnyOf → rotate (never empty).
//! Live: LCD query + optional `terpd` broadcast (ignored tests).
//!
//! Passkey is not the chain signer. Each device is a module authenticator;
//! AnyOf is the multi-device wrap. Same msgs HashMerchant may feegrant.

use serde_json::{json, Value};

use super::{
    authenticators_query_ok, msg_add_authenticator, AUTHENTICATOR_ALL_OF,
    AUTHENTICATOR_ANY_OF, AUTHENTICATOR_SIGNATURE, REMOVE_AUTHENTICATOR_TYPE_URL,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub label: String,
    /// Chain authenticator id once queried; None until the add lands.
    pub id: Option<u64>,
    pub authenticator_type: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthAccount {
    pub sender: String,
    pub policy: ComposeOp,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeOp {
    AnyOf,
    AllOf,
}

impl ComposeOp {
    pub fn type_str(self) -> &'static str {
        match self {
            ComposeOp::AnyOf => AUTHENTICATOR_ANY_OF,
            ComposeOp::AllOf => AUTHENTICATOR_ALL_OF,
        }
    }
}

pub fn msg_remove_authenticator(sender: &str, id: u64) -> Value {
    json!({
        "@type": REMOVE_AUTHENTICATOR_TYPE_URL,
        "sender": sender,
        "id": id.to_string(),
    })
}

/// Osmosis-shaped nested init list (JSON). Live encode is proto; this is the plan.
pub fn compose_members(devices: &[Device]) -> Value {
    json!(devices
        .iter()
        .map(|d| json!({
            "authenticator_type": d.authenticator_type,
            "data": d.data_b64,
        }))
        .collect::<Vec<_>>())
}

pub fn msg_add_any_of(sender: &str, devices: &[Device]) -> Value {
    let members = compose_members(devices);
    let data = base64_json(&members);
    msg_add_authenticator(sender, AUTHENTICATOR_ANY_OF, &data)
}

fn base64_json(v: &Value) -> String {
    let raw = serde_json::to_vec(v).unwrap_or_default();
    base64_nopad(&raw)
}

fn base64_nopad(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(T[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < bytes.len() {
            out.push(T[(b2 & 63) as usize] as char);
        }
        i += 3;
    }
    out
}

/// In-process ledger: assign ids 1,2,3… like the module.
pub fn add_device(acc: &mut AuthAccount, label: &str, data_b64: &str) -> u64 {
    let next = acc
        .devices
        .iter()
        .filter_map(|d| d.id)
        .max()
        .unwrap_or(0)
        + 1;
    acc.devices.push(Device {
        label: label.into(),
        id: Some(next),
        authenticator_type: AUTHENTICATOR_SIGNATURE.into(),
        data_b64: data_b64.into(),
    });
    next
}

pub fn remove_device(acc: &mut AuthAccount, id: u64) -> Result<(), String> {
    if acc.devices.len() < 2 {
        return Err("keep at least one authenticator".into());
    }
    let before = acc.devices.len();
    acc.devices.retain(|d| d.id != Some(id));
    if acc.devices.len() == before {
        return Err("id not found".into());
    }
    Ok(())
}

/// Rotate: add the new device, then drop `retire` if we still have ≥1 other.
pub fn rotate(acc: &mut AuthAccount, retire: u64, new_label: &str, new_data: &str) -> Result<u64, String> {
    let added = add_device(acc, new_label, new_data);
    remove_device(acc, retire)?;
    Ok(added)
}

pub fn parse_authenticator_ids(queried: &str) -> Vec<u64> {
    let v: Value = match serde_json::from_str(queried.trim()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut ids = Vec::new();
    walk_ids(&v, &mut ids);
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn walk_ids(v: &Value, out: &mut Vec<u64>) {
    match v {
        Value::Object(m) => {
            if let Some(id) = m.get("id").and_then(|x| {
                x.as_u64()
                    .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
            }) {
                if m.contains_key("type")
                    || m.contains_key("authenticator_type")
                    || m.contains_key("config")
                {
                    out.push(id);
                }
            }
            for x in m.values() {
                walk_ids(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                walk_ids(x, out);
            }
        }
        _ => {}
    }
}

/// ZIP-321 as DREGG issues it. Never invent UA. Never zcash://.
pub fn zip321_ok(uri: &str) -> bool {
    let s = uri.trim();
    if !s.starts_with("zcash:") || s.starts_with("zcash://") {
        return false;
    }
    if s.to_ascii_lowercase().contains("terp1") {
        return false;
    }
    true
}

/// LCD query (Osmosis-shaped + Terp alias). Empty list is a valid account.
pub fn lcd_authenticators_path(account: &str) -> Vec<String> {
    let a = urlencoding_minimal(account);
    vec![
        format!("/osmosis/smartaccount/authenticators/{a}"),
        format!("/terp/smartaccount/v1beta1/authenticators/{a}"),
        format!("/osmosis/smartaccount/v1beta1/authenticators/{a}"),
    ]
}

fn urlencoding_minimal(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

pub fn query_json_looks_like_account(body: &str) -> bool {
    authenticators_query_ok(body) || body.contains("authenticators")
}

/// Full local workflow the FE `/keys` page mirrors.
pub fn local_anyof_rotate() -> AuthAccount {
    let mut acc = AuthAccount {
        sender: "terp1guest".into(),
        policy: ComposeOp::AnyOf,
        devices: Vec::new(),
    };
    add_device(&mut acc, "device-1", "ZGV2aWNlMQ");
    add_device(&mut acc, "device-2", "ZGV2aWNlMg");
    let retire = acc.devices[0].id.unwrap();
    rotate(&mut acc, retire, "device-3", "ZGV2aWNlMw").expect("rotate");
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard_authz::ADD_AUTHENTICATOR_TYPE_URL;

    #[test]
    fn add_query_rotate_never_empty() {
        let acc = local_anyof_rotate();
        assert_eq!(acc.policy, ComposeOp::AnyOf);
        assert_eq!(acc.devices.len(), 2);
        let labels: Vec<_> = acc.devices.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, ["device-2", "device-3"]);
        assert!(acc.devices.iter().all(|d| d.id.is_some()));
        let mut once = acc.clone();
        let drop_id = once.devices[0].id.unwrap();
        let keep = once.devices[1].id.unwrap();
        assert!(remove_device(&mut once, drop_id).is_ok());
        assert_eq!(once.devices.len(), 1);
        assert_eq!(once.devices[0].id, Some(keep));
    }

    #[test]
    fn rotate_refuses_empty() {
        let mut acc = AuthAccount {
            sender: "terp1guest".into(),
            policy: ComposeOp::AnyOf,
            devices: Vec::new(),
        };
        add_device(&mut acc, "only", "YQ");
        assert_eq!(
            remove_device(&mut acc, 1).unwrap_err(),
            "keep at least one authenticator"
        );
    }

    #[test]
    fn msgs_are_module_proto_not_cosmwasm() {
        let add = msg_add_authenticator("terp1a", AUTHENTICATOR_SIGNATURE, "YQ");
        assert_eq!(add["@type"], ADD_AUTHENTICATOR_TYPE_URL);
        assert!(add.get("execute").is_none());
        let rm = msg_remove_authenticator("terp1a", 1);
        assert_eq!(rm["@type"], REMOVE_AUTHENTICATOR_TYPE_URL);
        assert_eq!(rm["id"], "1");
        let any = msg_add_any_of(
            "terp1a",
            &[Device {
                label: "d1".into(),
                id: Some(1),
                authenticator_type: AUTHENTICATOR_SIGNATURE.into(),
                data_b64: "YQ".into(),
            }],
        );
        assert_eq!(any["authenticator_type"], AUTHENTICATOR_ANY_OF);
    }

    #[test]
    fn parse_ids_from_query_shapes() {
        assert_eq!(
            parse_authenticator_ids(
                r#"{"authenticators":[{"id":"1","type":"SignatureVerification"},{"id":"2","type":"AnyOf"}]}"#
            ),
            vec![1, 2]
        );
        assert!(parse_authenticator_ids(r#"{"authenticators":[]}"#).is_empty());
    }

    #[test]
    fn zip321_dregg_rules() {
        assert!(zip321_ok("zcash:u1xyz?amount=0.1&memo=abc"));
        assert!(!zip321_ok("zcash://u1xyz"));
        assert!(!zip321_ok(""));
        assert!(!zip321_ok("zcash:u1?memo=terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0"));
    }

    #[test]
    fn query_ok_still_requires_nonempty_or_type() {
        assert!(authenticators_query_ok(
            r#"{"authenticators":[{"id":"1","type":"AnyOf"}]}"#
        ));
        assert!(!authenticators_query_ok(r#"{"authenticators":[]}"#));
    }
}
