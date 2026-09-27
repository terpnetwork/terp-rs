//! DAO → runtime key → scoped feegrant for ephemeral wallet onboarding.
//!
//! Soundness: the feegrant allowance must not include bank send, mint, or IBC.
//! A **separate** granter-paid `MsgSend` of 1 THIOL creates the account and
//! leaves runway. That send is not in `AllowedMsgAllowance`.
//!
//! Embedded in hash-market (`/onboard/*` on hash-market-server). Not a second process.

pub mod curate;
pub mod workflow;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Messages a **registration** feegrant may cover. Keep this list tiny.
///
/// SoT is the chain module proto (`osmosis.smartaccount.v1beta1`):
/// `MsgAddAuthenticator { sender, authenticator_type, data }`.
/// CosmWasm `{ add_authenticator: … }` / `{ register_authenticator: … }`
/// execute bodies are **not** this module and must not appear here.
pub const ADD_AUTHENTICATOR_TYPE_URL: &str = "/osmosis.smartaccount.v1beta1.MsgAddAuthenticator";
/// Lab `terp-core:local-zk` / morocco smartaccount module type URL.
pub const ADD_AUTHENTICATOR_TYPE_URL_TERP: &str = "/terp.smartaccount.v1beta1.MsgAddAuthenticator";

/// Type strings the module's authenticator manager actually knows.
/// Not CosmWasm `ty: "passkey"`. Passkey-as-module type is whatever the
/// chain registered (often still SignatureVerification + device pubkey
/// in `data`, or CosmwasmAuthenticatorV1 wrapping a contract).
pub const AUTHENTICATOR_SIGNATURE: &str = "SignatureVerification";
pub const AUTHENTICATOR_COSMWASM: &str = "CosmwasmAuthenticatorV1";

pub const ALLOWED_MSG_TYPES: &[&str] = &[
    ADD_AUTHENTICATOR_TYPE_URL,
    ADD_AUTHENTICATOR_TYPE_URL_TERP,
    "/abstractaccount.v1.MsgRegisterAccount",
];

/// Proto `MsgAddAuthenticator` as Amino/JSON Any. This is the module msg.
/// Do not wrap this as `{ add_authenticator: { … } }` CosmWasm execute.
pub fn msg_add_authenticator(sender: &str, authenticator_type: &str, data_b64: &str) -> Value {
    json!({
        "@type": ADD_AUTHENTICATOR_TYPE_URL,
        "sender": sender,
        "authenticator_type": authenticator_type,
        "data": data_b64,
    })
}

/// Authz grants the DAO gives the off-chain runtime key.
pub const AUTHZ_MSG_TYPES: &[&str] = &[
    "/cosmos.feegrant.v1beta1.MsgGrantAllowance",
    "/cosmos.feegrant.v1beta1.MsgRevokeAllowance",
];

/// Gas for CosmWasm mint only — never mixed into [`AllowanceScope::registration_only`].
pub const MINT_MSG_TYPES: &[&str] = &["/cosmwasm.wasm.v1.MsgExecuteContract"];

pub const DENOM: &str = "uthiol";
/// 1 THIOL (6 decimals) — granter-paid runway on passkey register.
pub const RUNWAY_UTHOL: &str = "1000000";

pub fn fee_denom() -> String {
    std::env::var("ONBOARD_DENOM").unwrap_or_else(|_| DENOM.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardRoles {
    pub dao: String,
    pub runtime_key: String,
    pub chain_id: String,
}

/// Bech32 `terp1` of real length — not env placeholders like `DAO_ADDR`.
pub fn is_live_terp1(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("terp1") && s.len() >= 39 && !s.contains("placeholder") && s != "DAO_ADDR"
}

impl OnboardRoles {
    pub fn from_env() -> Self {
        Self {
            dao: std::env::var("ONBOARD_DAO").unwrap_or_else(|_| "DAO_ADDR".into()),
            runtime_key: std::env::var("ONBOARD_RUNTIME")
                .unwrap_or_else(|_| "RUNTIME_KEY_ADDR".into()),
            chain_id: std::env::var("ONBOARD_CHAIN").unwrap_or_else(|_| "120u-1".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeegrantRequest {
    pub grantee: String,
    pub spend_limit: String,
    pub expiration: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantPurpose {
    Register,
    Mint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowanceScope {
    pub allowed: Vec<String>,
}

impl AllowanceScope {
    pub fn registration_only() -> Self {
        Self {
            allowed: ALLOWED_MSG_TYPES.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    pub fn mint_only() -> Self {
        Self {
            allowed: MINT_MSG_TYPES.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    pub fn permits(&self, type_url: &str) -> bool {
        self.allowed.iter().any(|a| a == type_url)
    }

    pub fn is_sound(&self) -> bool {
        !self.allowed.is_empty()
            && !self.permits("/cosmos.bank.v1beta1.MsgSend")
            && self.allowed.iter().all(|a| {
                ALLOWED_MSG_TYPES.contains(&a.as_str()) || MINT_MSG_TYPES.contains(&a.as_str())
            })
            && !(self
                .allowed
                .iter()
                .any(|a| ALLOWED_MSG_TYPES.contains(&a.as_str()))
                && self
                    .allowed
                    .iter()
                    .any(|a| MINT_MSG_TYPES.contains(&a.as_str())))
    }
}

/// DAO `MsgGrant` bundle: runtime may grant/revoke feegrant only.
pub fn dao_authz_grants(roles: &OnboardRoles) -> Value {
    let grants: Vec<Value> = AUTHZ_MSG_TYPES
        .iter()
        .map(|type_url| {
            json!({
                "@type": "/cosmos.authz.v1beta1.MsgGrant",
                "granter": roles.dao,
                "grantee": roles.runtime_key,
                "grant": {
                    "authorization": {
                        "@type": "/cosmos.authz.v1beta1.GenericAuthorization",
                        "msg": type_url
                    },
                    "expiration": null
                }
            })
        })
        .collect();
    json!({
        "chain_id": roles.chain_id,
        "memo": "dao → onboard-authz runtime: feegrant register-only",
        "msgs": grants
    })
}

pub fn mint_feegrant(roles: &OnboardRoles, req: &FeegrantRequest) -> Value {
    json!({
        "@type": "/cosmos.feegrant.v1beta1.MsgGrantAllowance",
        "granter": roles.runtime_key,
        "grantee": req.grantee,
        "allowance": {
            "@type": "/cosmos.feegrant.v1beta1.AllowedMsgAllowance",
            "allowance": {
                "@type": "/cosmos.feegrant.v1beta1.BasicAllowance",
                "spend_limit": [{ "denom": fee_denom(), "amount": req.spend_limit }],
                "expiration": req.expiration
            },
            "allowed_messages": MINT_MSG_TYPES
        }
    })
}

pub fn runway_send(roles: &OnboardRoles, to: &str, amount: &str) -> Value {
    json!({
        "@type": "/cosmos.bank.v1beta1.MsgSend",
        "from_address": roles.runtime_key,
        "to_address": to,
        "amount": [{ "denom": fee_denom(), "amount": amount }]
    })
}

pub fn registration_feegrant(roles: &OnboardRoles, req: &FeegrantRequest) -> Value {
    let scope = AllowanceScope::registration_only();
    json!({
        "@type": "/cosmos.feegrant.v1beta1.MsgGrantAllowance",
        "granter": roles.runtime_key,
        "grantee": req.grantee,
        "allowance": {
            "@type": "/cosmos.feegrant.v1beta1.AllowedMsgAllowance",
            "allowance": {
                "@type": "/cosmos.feegrant.v1beta1.BasicAllowance",
                "spend_limit": [{ "denom": fee_denom(), "amount": req.spend_limit }],
                "expiration": req.expiration
            },
            "allowed_messages": scope.allowed
        }
    })
}

pub fn authz_exec_feegrant(roles: &OnboardRoles, inner: Value) -> Value {
    json!({
        "@type": "/cosmos.authz.v1beta1.MsgExec",
        "grantee": roles.runtime_key,
        "msgs": [inner]
    })
}

pub fn terpd_hint(roles: &OnboardRoles) -> String {
    format!(
        "# After DAO votes / multisig signs dao_authz_grants.json:\n\
         # Runtime key {rt} may MsgGrantAllowance only for:\n\
         #   {allowed}\n\
         terpd tx authz exec feegrant-register.json --from onboard-runtime --chain-id {chain} --fees 5000{denom} --yes",
        rt = roles.runtime_key,
        allowed = ALLOWED_MSG_TYPES.join("\n#   "),
        chain = roles.chain_id,
        denom = fee_denom()
    )
}

/// Optional chain broadcast. Hash-market / faucet can supply their own signer.
pub trait GrantBroadcaster: Send + Sync {
    fn broadcast_authz_exec(&self, exec: &Value) -> Value;
    fn broadcast_bank_send(&self, to: &str, amount: &str) -> Value {
        let _ = (to, amount);
        json!({"error": "bank send not wired"})
    }
}

pub fn txhash_from_broadcast(v: &Value) -> Option<String> {
    if let Some(h) = v.get("txhash").and_then(|x| x.as_str()) {
        if !h.is_empty() {
            return Some(h.to_string());
        }
    }
    let stdout = v.get("stdout").and_then(|x| x.as_str()).unwrap_or("");
    if let Ok(j) = serde_json::from_str::<Value>(stdout) {
        if let Some(h) = j.get("txhash").and_then(|x| x.as_str()) {
            if !h.is_empty() {
                return Some(h.to_string());
            }
        }
    }
    None
}

/// Parse `terpd q auth account` JSON. Address argument must be bech32, never a key name.
pub fn parse_account_sequence(blob: &str) -> Option<u64> {
    let v: Value = serde_json::from_str(blob.trim()).ok()?;
    v.pointer("/account/sequence")
        .or_else(|| v.pointer("/base_account/sequence"))
        .or_else(|| v.pointer("/account/base_account/sequence"))
        .or_else(|| v.get("sequence"))
        .and_then(|x| {
            x.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| x.as_u64())
        })
}

/// Parse `terpd status` / docker `terpd status` for latest height.
pub fn parse_status_height(blob: &str) -> u64 {
    for key in ["latest_block_height", "latestBlockHeight"] {
        if let Some(i) = blob.find(key) {
            let rest = &blob[i + key.len()..];
            let digits: String = rest
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<u64>() {
                return n;
            }
        }
    }
    0
}

/// If `start` is 0, wait until height is >0 **and then increases once more**.
/// Do not treat the first nonzero height as the commit of the last tx.
pub fn wait_until_height_commits<H, S>(start: u64, mut height: H, mut nap: S) -> bool
where
    H: FnMut() -> u64,
    S: FnMut(),
{
    let mut baseline = start;
    if baseline == 0 {
        for _ in 0..40 {
            nap();
            baseline = height();
            if baseline > 0 {
                break;
            }
        }
        if baseline == 0 {
            return false;
        }
    }
    for _ in 0..40 {
        nap();
        if height() > baseline {
            return true;
        }
    }
    false
}

/// Smartaccount query success: non-empty `authenticators` array or a
/// `SignatureVerification` type/id in the JSON. Substring `"authenticator"`
/// matches empty lists (`{"authenticators":[]}`) and is not enough.
pub fn authenticators_query_ok(queried: &str) -> bool {
    let v: Value = match serde_json::from_str(queried.trim()) {
        Ok(v) => v,
        Err(_) => return queried.contains("SignatureVerification"),
    };
    let nonempty = |a: &Value| a.as_array().map(|x| !x.is_empty()).unwrap_or(false);
    if nonempty(&v["authenticators"]) || nonempty(&v["account_authenticators"]) {
        return true;
    }
    if v.get("account")
        .and_then(|a| {
            a.get("authenticators")
                .or_else(|| a.get("account_authenticators"))
        })
        .map(nonempty)
        == Some(true)
    {
        return true;
    }
    json_has_signature_verification(&v)
}

fn json_has_signature_verification(v: &Value) -> bool {
    match v {
        Value::String(s) => s == AUTHENTICATOR_SIGNATURE || s.contains(AUTHENTICATOR_SIGNATURE),
        Value::Array(a) => a.iter().any(json_has_signature_verification),
        Value::Object(m) => m.values().any(json_has_signature_verification),
        _ => false,
    }
}

fn terpd_status_height() -> u64 {
    use std::process::Command;
    let out = Command::new("terpd").args(["status"]).output();
    let blob = match out {
        Ok(o) => format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(_) => return 0,
    };
    parse_status_height(&blob)
}

fn terpd_wait_next_block() {
    wait_until_height_commits(terpd_status_height(), terpd_status_height, || {
        std::thread::sleep(std::time::Duration::from_millis(400));
    });
}

/// `terpd keys show` when `from` is a key name; otherwise the terp1 string itself.
pub fn terpd_from_bech32(from: &str) -> String {
    if from.starts_with("terp1") {
        return from.to_string();
    }
    use std::process::Command;
    let out = Command::new("terpd")
        .args(["keys", "show", from, "--output", "json"])
        .output();
    let blob = match out {
        Ok(o) => format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(_) => return String::new(),
    };
    serde_json::from_str::<Value>(blob.trim())
        .ok()
        .and_then(|v| {
            v.get("address")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
        .filter(|s| s.starts_with("terp1"))
        .unwrap_or_default()
}

fn terpd_account_sequence(bech32: &str) -> Option<u64> {
    if !bech32.starts_with("terp1") {
        return None;
    }
    use std::process::Command;
    let out = Command::new("terpd")
        .args(["q", "auth", "account", bech32, "--output", "json"])
        .output()
        .ok()?;
    let blob = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_account_sequence(&blob)
}

fn onboard_from_key() -> String {
    std::env::var("ONBOARD_FROM").unwrap_or_else(|_| "onboard-runtime".into())
}

/// Who signs feegrant / bank send. Inner `MsgGrantAllowance.granter` must be this bech32.
pub fn onboard_signer_bech32() -> String {
    let from = onboard_from_key();
    let addr = terpd_from_bech32(&from);
    if !addr.is_empty() {
        addr
    } else {
        std::env::var("ONBOARD_RUNTIME").unwrap_or(from)
    }
}

fn terpd_run(args: &[String]) -> Value {
    use std::process::Command;
    match Command::new("terpd").args(args).output() {
        Ok(o) => json!({
            "status": o.status.code(),
            "stdout": String::from_utf8_lossy(&o.stdout),
            "stderr": String::from_utf8_lossy(&o.stderr),
        }),
        Err(e) => json!({"error": e.to_string()}),
    }
}

/// `ONBOARD_BROADCAST=1` + `terpd` in PATH (same as the original sidecar).
/// `terpd` rejects `--broadcast-mode block`; use `sync`, wait for height, then
/// the next tx passes `--sequence` from `q auth account` **bech32**.
pub struct TerpdBroadcaster;

impl GrantBroadcaster for TerpdBroadcaster {
    fn broadcast_authz_exec(&self, exec: &Value) -> Value {
        let tmp = std::env::temp_dir().join("terp-onboard-authz-exec.json");
        if std::fs::write(&tmp, exec.to_string()).is_err() {
            return json!({"error":"write tmp"});
        }
        let from = onboard_from_key();
        let signer = onboard_signer_bech32();
        let chain = std::env::var("ONBOARD_CHAIN").unwrap_or_else(|_| "120u-1".into());
        let mut args = vec![
            "tx".into(),
            "authz".into(),
            "exec".into(),
            tmp.to_string_lossy().into_owned(),
            "--from".into(),
            from,
            "--chain-id".into(),
            chain,
            "--fees".into(),
            format!("5000{}", fee_denom()),
            "--broadcast-mode".into(),
            "sync".into(),
            "--yes".into(),
            "--output".into(),
            "json".into(),
        ];
        if let Some(seq) = terpd_account_sequence(&signer) {
            args.push("--sequence".into());
            args.push(seq.to_string());
        }
        let out = terpd_run(&args);
        terpd_wait_next_block();
        out
    }

    fn broadcast_bank_send(&self, to: &str, amount: &str) -> Value {
        let from = onboard_from_key();
        let signer = onboard_signer_bech32();
        let chain = std::env::var("ONBOARD_CHAIN").unwrap_or_else(|_| "120u-1".into());
        let coins = format!("{amount}{}", fee_denom());
        let mut args = vec![
            "tx".into(),
            "bank".into(),
            "send".into(),
            from.clone(),
            to.to_string(),
            coins,
            "--from".into(),
            from,
            "--chain-id".into(),
            chain,
            "--fees".into(),
            format!("5000{}", fee_denom()),
            "--broadcast-mode".into(),
            "sync".into(),
            "--yes".into(),
            "--output".into(),
            "json".into(),
        ];
        if let Some(seq) = terpd_account_sequence(&signer) {
            args.push("--sequence".into());
            args.push(seq.to_string());
        }
        let out = terpd_run(&args);
        terpd_wait_next_block();
        out
    }
}

/// In-process grant allocator. Services hold one of these and call
/// [`OnboardRuntime::try_handle`] from their existing HTTP server.
pub struct OnboardRuntime {
    pub roles: OnboardRoles,
    pub service_name: String,
    pub register_limit: String,
    pub mint_limit: String,
    /// Grantee-paid runway in uthiol (`"0"` disables). Default 1 THIOL.
    pub runway_amount: String,
    pub broadcast: bool,
    pub broadcaster: Option<Box<dyn GrantBroadcaster>>,
    /// One granter sequence at a time: bank-send then feegrant must not interleave
    /// with another checkout's broadcasts (HTTP is threaded).
    pub grant_tx_mu: std::sync::Mutex<()>,
}

impl OnboardRuntime {
    pub fn from_env() -> Self {
        let broadcast = std::env::var("ONBOARD_BROADCAST").ok().as_deref() == Some("1");
        Self {
            roles: OnboardRoles::from_env(),
            service_name: std::env::var("ONBOARD_SERVICE")
                .unwrap_or_else(|_| "onboard-authz".into()),
            register_limit: std::env::var("ONBOARD_SPEND_LIMIT").unwrap_or_else(|_| "25000".into()),
            mint_limit: std::env::var("ONBOARD_MINT_LIMIT").unwrap_or_else(|_| "50000".into()),
            runway_amount: std::env::var("ONBOARD_RUNWAY_AMOUNT")
                .unwrap_or_else(|_| RUNWAY_UTHOL.into()),
            broadcast,
            broadcaster: if broadcast {
                Some(Box::new(TerpdBroadcaster))
            } else {
                None
            },
            grant_tx_mu: std::sync::Mutex::new(()),
        }
    }

    pub fn require_live_roles(&self) -> Result<(), String> {
        if !is_live_terp1(&self.roles.dao) {
            return Err("ONBOARD_DAO / dao must be a live terp1 address".into());
        }
        if !is_live_terp1(&self.roles.runtime_key) {
            return Err("ONBOARD_RUNTIME / runtime_key must be a live terp1 address".into());
        }
        Ok(())
    }

    pub fn status_json(&self) -> Value {
        json!({
            "ready": is_live_terp1(&self.roles.dao) && is_live_terp1(&self.roles.runtime_key),
            "service": self.service_name,
            "chain_id": self.roles.chain_id,
            "dao": self.roles.dao,
            "runtime_key": self.roles.runtime_key,
            "allowed_messages": AllowanceScope::registration_only().allowed,
            "mint_messages": MINT_MSG_TYPES,
            "broadcast": self.broadcast,
            "runway_amount": self.runway_amount,
            "runway_denom": fee_denom(),
            "passkey_grant": "/onboard/v1/passkey-grant",
        })
    }

    pub fn allocate(&self, purpose: GrantPurpose, address: &str) -> Result<Value, String> {
        if !address.starts_with("terp1") || address.len() < 20 {
            return Err("expected terp1 bech32".into());
        }
        let (mut inner, extra) = match purpose {
            GrantPurpose::Register => {
                let inner = registration_feegrant(
                    &self.roles,
                    &FeegrantRequest {
                        grantee: address.into(),
                        spend_limit: self.register_limit.clone(),
                        expiration: None,
                    },
                );
                if !AllowanceScope::registration_only().is_sound() {
                    return Err("unsound allowance".into());
                }
                let amt = self.runway_amount.clone();
                let extra = if amt.is_empty() || amt == "0" {
                    json!({})
                } else {
                    json!({
                        "runway": {
                            "amount": amt,
                            "denom": fee_denom(),
                            "msg": runway_send(&self.roles, address, &amt),
                        }
                    })
                };
                (inner, extra)
            }
            GrantPurpose::Mint => {
                let inner = mint_feegrant(
                    &self.roles,
                    &FeegrantRequest {
                        grantee: address.into(),
                        spend_limit: self.mint_limit.clone(),
                        expiration: None,
                    },
                );
                let allowed = inner["allowance"]["allowed_messages"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if allowed
                    .iter()
                    .any(|v| v.as_str() == Some("/cosmos.bank.v1beta1.MsgSend"))
                {
                    return Err("mint grant must not include bank send".into());
                }
                (inner, json!({"purpose": "mint"}))
            }
        };
        // Inner MsgGrantAllowance.granter must be who actually signs the feegrant.
        let signer = onboard_signer_bech32();
        let granter = if self.roles.runtime_key.starts_with("terp1") {
            self.roles.runtime_key.clone()
        } else if signer.starts_with("terp1") {
            signer
        } else {
            self.roles.runtime_key.clone()
        };
        if let Some(g) = inner.get_mut("granter") {
            *g = json!(granter.clone());
        }
        let exec = authz_exec_feegrant(&self.roles, inner.clone());
        let mut runway_txhash = Value::Null;
        let mut send_bc = json!(null);
        let mut feegrant_broadcast = json!(null);
        let runway_needed = extra.get("runway").is_some();
        if let Some(b) = &self.broadcaster {
            if self.broadcast {
                let _seq = self.grant_tx_mu.lock().unwrap_or_else(|p| p.into_inner());
                if purpose == GrantPurpose::Register {
                    if let Some(amt) = extra
                        .get("runway")
                        .and_then(|r| r.get("amount"))
                        .and_then(|x| x.as_str())
                    {
                        send_bc = b.broadcast_bank_send(address, amt);
                        runway_txhash = match txhash_from_broadcast(&send_bc) {
                            Some(h) => json!(h),
                            None => Value::Null,
                        };
                    }
                }
                feegrant_broadcast = b.broadcast_authz_exec(&exec);
            }
        }
        let fg_hash = txhash_from_broadcast(&feegrant_broadcast);
        let send_ok = !self.broadcast || !runway_needed || runway_txhash.as_str().is_some();
        let mut out = json!({
            "ok": send_ok,
            "grantee": address,
            "granter": granter,
            "msg": inner,
            "authz_exec": exec,
            "broadcast": feegrant_broadcast,
            "txhash": fg_hash,
            "runway_txhash": runway_txhash,
        });
        if let Some(obj) = extra.as_object() {
            if let Some(dst) = out.as_object_mut() {
                for (k, v) in obj {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(r) = out.get_mut("runway") {
            r["broadcast"] = send_bc;
            r["txhash"] = runway_txhash;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles() -> OnboardRoles {
        OnboardRoles {
            dao: "terp1dao00000000000000000000000000000000000".into(),
            runtime_key: "terp1runtime000000000000000000000000000000".into(),
            chain_id: "120u-1".into(),
        }
    }

    #[test]
    fn scope_rejects_bank_and_wasm_on_register() {
        let s = AllowanceScope::registration_only();
        assert!(s.is_sound());
        assert!(s.permits(ADD_AUTHENTICATOR_TYPE_URL));
        assert!(!s.permits("/cosmwasm.wasm.v1.MsgExecuteContract"));
        assert!(!s.permits("/cosmos.bank.v1beta1.MsgSend"));
        assert!(!s.permits("/cosmwasm.wasm.v1.MsgExecuteContract"));
    }

    #[test]
    fn runway_is_1_thiol_not_in_allowance() {
        let r = roles();
        let s = runway_send(&r, "terp1alice", RUNWAY_UTHOL);
        assert_eq!(s["@type"], "/cosmos.bank.v1beta1.MsgSend");
        assert_eq!(s["amount"][0]["amount"], "1000000");
        assert!(!AllowanceScope::registration_only().permits("/cosmos.bank.v1beta1.MsgSend"));
    }

    #[test]
    fn add_authenticator_json_is_module_proto_not_cosmwasm() {
        let msg = msg_add_authenticator("terp1alice", AUTHENTICATOR_SIGNATURE, "AA==");
        assert_eq!(msg["@type"], ADD_AUTHENTICATOR_TYPE_URL);
        assert_eq!(msg["authenticator_type"], AUTHENTICATOR_SIGNATURE);
        assert!(msg.get("add_authenticator").is_none());
        assert!(msg.get("register_authenticator").is_none());
    }

    #[test]
    fn mixed_register_and_mint_is_unsound() {
        let mut s = AllowanceScope::registration_only();
        s.allowed
            .push("/cosmwasm.wasm.v1.MsgExecuteContract".into());
        assert!(!s.is_sound());
    }

    #[test]
    fn dao_grant_is_feegrant_only() {
        let bundle = dao_authz_grants(&roles());
        let msgs = bundle["msgs"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        for m in msgs {
            assert_eq!(m["@type"], "/cosmos.authz.v1beta1.MsgGrant");
            let msg = m["grant"]["authorization"]["msg"].as_str().unwrap();
            assert!(AUTHZ_MSG_TYPES.contains(&msg));
        }
    }

    #[test]
    fn mint_grant_is_execute_only() {
        let fg = mint_feegrant(
            &roles(),
            &FeegrantRequest {
                grantee: "terp1user000000000000000000000000000000000".into(),
                spend_limit: "50000".into(),
                expiration: None,
            },
        );
        let allowed = fg["allowance"]["allowed_messages"].as_array().unwrap();
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0], "/cosmwasm.wasm.v1.MsgExecuteContract");
    }

    #[test]
    fn unsound_if_operator_adds_bank() {
        let mut s = AllowanceScope::registration_only();
        s.allowed.push("/cosmos.bank.v1beta1.MsgSend".into());
        assert!(!s.is_sound());
    }

    #[test]
    fn runtime_rejects_bad_addr() {
        let rt = OnboardRuntime {
            roles: roles(),
            service_name: "test".into(),
            register_limit: "1".into(),
            mint_limit: "1".into(),
            runway_amount: RUNWAY_UTHOL.into(),
            broadcast: false,
            broadcaster: None,
            grant_tx_mu: std::sync::Mutex::new(()),
        };
        assert!(rt.allocate(GrantPurpose::Register, "nope").is_err());
        assert!(rt
            .allocate(
                GrantPurpose::Register,
                "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0"
            )
            .is_ok());
    }

    struct FakeBc {
        send: Value,
        exec: Value,
    }
    impl GrantBroadcaster for FakeBc {
        fn broadcast_authz_exec(&self, _exec: &Value) -> Value {
            self.exec.clone()
        }
        fn broadcast_bank_send(&self, _to: &str, _amount: &str) -> Value {
            self.send.clone()
        }
    }

    fn rt_broadcast(bc: FakeBc) -> OnboardRuntime {
        OnboardRuntime {
            roles: roles(),
            service_name: "test".into(),
            register_limit: "1".into(),
            mint_limit: "1".into(),
            runway_amount: RUNWAY_UTHOL.into(),
            broadcast: true,
            broadcaster: Some(Box::new(bc)),
            grant_tx_mu: std::sync::Mutex::new(()),
        }
    }

    #[test]
    fn allocate_ok_requires_runway_txhash_when_runway_positive() {
        let rt = rt_broadcast(FakeBc {
            send: json!({"error": "no send"}),
            exec: json!({"txhash": "feegrant"}),
        });
        let v = rt
            .allocate(
                GrantPurpose::Register,
                "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0",
            )
            .unwrap();
        assert_eq!(v["ok"], false);
        assert!(v.get("runway_txhash").unwrap().is_null());
    }

    #[test]
    fn allocate_ok_when_runway_send_lands() {
        let rt = rt_broadcast(FakeBc {
            send: json!({"txhash": "sendhash"}),
            exec: json!({"txhash": "feegrant"}),
        });
        let v = rt
            .allocate(
                GrantPurpose::Register,
                "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0",
            )
            .unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["runway_txhash"], "sendhash");
        assert_eq!(v["msg"]["granter"], roles().runtime_key);
        assert_eq!(v["granter"], roles().runtime_key);
    }

    #[test]
    fn wait_zero_start_needs_nonzero_then_increase() {
        use std::cell::Cell;
        let n = Cell::new(0u32);
        let h = || {
            let i = n.get();
            n.set(i + 1);
            match i {
                0 | 1 => 0,
                2 | 3 => 1,
                _ => 2,
            }
        };
        assert!(wait_until_height_commits(0, h, || {}));
        let n = Cell::new(0u32);
        let stuck = || {
            let i = n.get();
            n.set(i + 1);
            if i == 0 {
                0
            } else {
                1
            }
        };
        assert!(!wait_until_height_commits(0, stuck, || {}));
    }

    #[test]
    fn authenticators_query_rejects_empty_list_substring() {
        assert!(!authenticators_query_ok(r#"{"authenticators":[]}"#));
        assert!(!authenticators_query_ok(
            r#"{"account":{"authenticators":[]}}"#
        ));
        assert!(authenticators_query_ok(
            r#"{"authenticators":[{"id":"1","type":"SignatureVerification"}]}"#
        ));
        assert!(authenticators_query_ok(
            r#"{"account":{"authenticators":[{"authenticator_type":"SignatureVerification"}]}}"#
        ));
        assert!(!authenticators_query_ok("authenticator"));
        assert!(!authenticators_query_ok(r#"{"account_authenticators":[]}"#));
        assert!(authenticators_query_ok(
            r#"{"account_authenticators":[{"id":"1","type":"SignatureVerification"}]}"#
        ));
    }

    #[test]
    fn parse_sequence_needs_bech32_shaped_json() {
        assert_eq!(
            parse_account_sequence(r#"{"account":{"sequence":"7"}}"#),
            Some(7)
        );
        assert_eq!(
            parse_account_sequence(r#"{"account":{"base_account":{"sequence":"3"}}}"#),
            Some(3)
        );
    }
}
