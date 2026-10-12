//! In-process keeper simulation for the onboard grant graph.

use super::{
    authz_exec_feegrant, dao_authz_grants, registration_feegrant, AllowanceScope, FeegrantRequest,
    OnboardRoles, ALLOWED_MSG_TYPES, AUTHZ_MSG_TYPES,
};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyErr {
    UnknownType(String),
    AuthzDenied,
    FeegrantDenied { reason: &'static str },
    UnsoundAllowance,
}

#[derive(Debug, Default)]
pub struct SimKeeper {
    pub authz: HashMap<(String, String), Vec<String>>,
    pub feegrant: HashMap<(String, String), Vec<String>>,
}

impl SimKeeper {
    pub fn apply(&mut self, msg: &Value) -> Result<(), ApplyErr> {
        let t = msg["@type"].as_str().unwrap_or("");
        match t {
            "/cosmos.authz.v1beta1.MsgGrant" => {
                let granter = str_field(msg, "granter");
                let grantee = str_field(msg, "grantee");
                let inner = msg["grant"]["authorization"]["msg"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if !AUTHZ_MSG_TYPES.contains(&inner.as_str()) {
                    return Err(ApplyErr::AuthzDenied);
                }
                self.authz
                    .entry((granter, grantee))
                    .or_default()
                    .push(inner);
                Ok(())
            }
            "/cosmos.authz.v1beta1.MsgExec" => {
                let grantee = str_field(msg, "grantee");
                let inner = &msg["msgs"][0];
                let inner_type = inner["@type"].as_str().unwrap_or("");
                let granter = str_field(inner, "granter");
                let allowed = self
                    .authz
                    .get(&(granter.clone(), grantee))
                    .cloned()
                    .unwrap_or_default();
                if !allowed.iter().any(|a| a == inner_type) {
                    return Err(ApplyErr::AuthzDenied);
                }
                self.apply(inner)
            }
            "/cosmos.feegrant.v1beta1.MsgGrantAllowance" => {
                let allowed = msg["allowance"]["allowed_messages"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let allowed: Vec<String> = allowed
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                let scope = AllowanceScope {
                    allowed: allowed.clone(),
                };
                if !scope.is_sound() {
                    return Err(ApplyErr::UnsoundAllowance);
                }
                let granter = str_field(msg, "granter");
                let grantee = str_field(msg, "grantee");
                self.feegrant.insert((granter, grantee), allowed);
                Ok(())
            }
            "/cosmos.feegrant.v1beta1.MsgRevokeAllowance" => {
                let granter = str_field(msg, "granter");
                let grantee = str_field(msg, "grantee");
                self.feegrant.remove(&(granter, grantee));
                Ok(())
            }
            other => Err(ApplyErr::UnknownType(other.to_string())),
        }
    }

    pub fn use_feegrant(
        &self,
        granter: &str,
        grantee: &str,
        msg_type: &str,
    ) -> Result<(), ApplyErr> {
        let allowed = self
            .feegrant
            .get(&(granter.to_string(), grantee.to_string()))
            .ok_or(ApplyErr::FeegrantDenied {
                reason: "no allowance",
            })?;
        if !allowed.iter().any(|a| a == msg_type) {
            return Err(ApplyErr::FeegrantDenied {
                reason: "msg not on AllowedMsgAllowance",
            });
        }
        if !ALLOWED_MSG_TYPES.contains(&msg_type) {
            return Err(ApplyErr::FeegrantDenied {
                reason: "not a registration msg",
            });
        }
        Ok(())
    }
}

fn str_field(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}

pub fn simulate_onboard(roles: &OnboardRoles, user: &str) -> Result<SimKeeper, ApplyErr> {
    let mut k = SimKeeper::default();
    let bundle = dao_authz_grants(roles);
    for m in bundle["msgs"].as_array().unwrap() {
        k.apply(m)?;
    }
    let inner = registration_feegrant(
        roles,
        &FeegrantRequest {
            grantee: user.into(),
            spend_limit: "25000".into(),
            expiration: None,
        },
    );
    let granted = k
        .authz
        .get(&(roles.runtime.clone(), roles.hashmerchant.clone()))
        .cloned()
        .unwrap_or_default();
    if !granted
        .iter()
        .any(|m| m == "/cosmos.feegrant.v1beta1.MsgGrantAllowance")
    {
        return Err(ApplyErr::AuthzDenied);
    }
    k.apply(&inner)?;
    let _ = authz_exec_feegrant(roles, inner.clone());
    Ok(k)
}

pub fn run_happy_path(
    k: &mut SimKeeper,
    roles: &OnboardRoles,
    user: &str,
) -> Result<(), ApplyErr> {
    *k = simulate_onboard(roles, user)?;
    k.use_feegrant(
        &roles.hashmerchant,
        user,
        super::ADD_AUTHENTICATOR_TYPE_URL,
    )?;
    if k.use_feegrant(&roles.runtime, user, super::ADD_AUTHENTICATOR_TYPE_URL)
        .is_ok()
    {
        return Err(ApplyErr::FeegrantDenied {
            reason: "runtime must not be customer fee.granter",
        });
    }
    if k.use_feegrant(&roles.hashmerchant, user, "/cosmos.bank.v1beta1.MsgSend")
        .is_ok()
    {
        return Err(ApplyErr::UnsoundAllowance);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles() -> OnboardRoles {
        OnboardRoles {
            runtime: "terp1dao".into(),
            hashmerchant: "terp1rt".into(),
            chain_id: "120u-1".into(),
        }
    }

    #[test]
    fn keeper_allows_add_authenticator_rejects_send_and_wasm() {
        let user = "terp1user";
        let k = simulate_onboard(&roles(), user).expect("sim");
        let rt = "terp1rt";
        assert!(k
            .use_feegrant(rt, user, crate::onboard_authz::ADD_AUTHENTICATOR_TYPE_URL)
            .is_ok());
        let send = k.use_feegrant(rt, user, "/cosmos.bank.v1beta1.MsgSend");
        assert!(matches!(send, Err(ApplyErr::FeegrantDenied { .. })), "{send:?}");
        let wasm = k.use_feegrant(rt, user, "/cosmwasm.wasm.v1.MsgExecuteContract");
        assert!(matches!(wasm, Err(ApplyErr::FeegrantDenied { .. })), "{wasm:?}");
    }

    #[test]
    fn authz_cannot_grant_bank_send() {
        let mut k = SimKeeper::default();
        let bad = serde_json::json!({
            "@type": "/cosmos.authz.v1beta1.MsgGrant",
            "granter": "terp1dao",
            "grantee": "terp1rt",
            "grant": {
                "authorization": {
                    "@type": "/cosmos.authz.v1beta1.GenericAuthorization",
                    "msg": "/cosmos.bank.v1beta1.MsgSend"
                }
            }
        });
        assert_eq!(k.apply(&bad), Err(ApplyErr::AuthzDenied));
    }
}
