//! Curate onboard grants: DAO proposal *or* a wallet authorizing the runtime key.
//!
//! This is the cw-orch-shaped curator (msgs + files + terpd). A live
//! `Daemon::builder` broadcast stays optional (`ONBOARD_BROADCAST=1` / terpd)
//! until the cw-orch crate is on the workspace path.

use crate::{
    authz_exec_feegrant, dao_authz_grants, mint_feegrant, registration_feegrant, AllowanceScope,
    GrantPurpose, OnboardRoles, AUTHZ_MSG_TYPES, ALLOWED_MSG_TYPES, MINT_MSG_TYPES,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

const EMBEDDED_INFRA: &str = include_str!("../data/infra-codes.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraFile {
    pub source: Option<String>,
    pub chains: BTreeMap<String, ChainInfra>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChainInfra {
    #[serde(default)]
    pub code_ids: BTreeMap<String, u64>,
    #[serde(default)]
    pub default: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurateMode {
    /// DAO-core / proposal-single: authorize runtime to issue scoped feegrants.
    DaoProposal,
    /// Wallet grants a *separate* runtime key (authz proxy).
    WalletProxy,
    /// Wallet *is* the runtime (no authz hop — it feegrants users itself).
    WalletAsRuntime,
}

#[derive(Debug, Clone)]
pub struct GrantPreset {
    pub id: &'static str,
    pub purpose: GrantPurpose,
    pub title: &'static str,
    pub code_id_keys: &'static [&'static str],
    pub default_addr_keys: &'static [&'static str],
}

pub const PRESETS: &[GrantPreset] = &[
    GrantPreset {
        id: "register",
        purpose: GrantPurpose::Register,
        title: "Smart-account authenticator registration",
        code_id_keys: &[
            "terp-passkey",
            "terp-zkjwt",
            "terp-ed25519",
            "terp-recovery",
            "abstract:abstract_account",
        ],
        default_addr_keys: &[
            "terp-passkey",
            "terp-zkjwt",
            "terp-recovery",
            "abstract:account-monarchy",
        ],
    },
    GrantPreset {
        id: "norick",
        purpose: GrantPurpose::Mint,
        title: "No Rick / cw_norick execute (Halo2 verify + mint)",
        code_id_keys: &["headstash:cw_norick"],
        default_addr_keys: &["headstash:cw_norick"],
    },
    GrantPreset {
        id: "svg",
        purpose: GrantPurpose::Mint,
        title: "SVG mint execute (cw-svg-minter / cw721_svg)",
        code_id_keys: &["cw-svg-minter", "cw721_svg", "nft:cw721_base"],
        default_addr_keys: &["cw-svg-minter", "cw721_svg"],
    },
    GrantPreset {
        id: "headstash",
        purpose: GrantPurpose::Mint,
        title: "Headstash claim execute",
        code_id_keys: &["headstash:cw_headstash"],
        default_addr_keys: &[],
    },
    GrantPreset {
        id: "loyalty",
        purpose: GrantPurpose::Mint,
        title: "Loyalty verifier execute",
        code_id_keys: &["loyalty:loyalty_verifier"],
        default_addr_keys: &[],
    },
    GrantPreset {
        id: "marketplace",
        purpose: GrantPurpose::Mint,
        title: "Marketplace-mirror execute",
        code_id_keys: &["hash-merchant:marketplace_mirror"],
        default_addr_keys: &[],
    },
];

pub fn preset(id: &str) -> Option<&'static GrantPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

pub fn load_infra(extra_path: Option<&Path>) -> InfraFile {
    let mut infra: InfraFile =
        serde_json::from_str(EMBEDDED_INFRA).expect("embedded infra-codes.json");
    if let Some(p) = extra_path {
        if let Ok(raw) = std::fs::read_to_string(p) {
            if let Ok(over) = serde_json::from_str::<Value>(&raw) {
                merge_state_json(&mut infra, &over);
            }
        }
    } else if let Ok(p) = std::env::var("ONBOARD_STATE_JSON") {
        let path = Path::new(&p);
        if path.is_file() {
            if let Ok(raw) = std::fs::read_to_string(path) {
                if let Ok(over) = serde_json::from_str::<Value>(&raw) {
                    merge_state_json(&mut infra, &over);
                }
            }
        }
    }
    infra
}

/// Accept either our slim `{chains:{id:{code_ids,default}}}` or full terp-state.json.
fn merge_state_json(infra: &mut InfraFile, over: &Value) {
    let chains = over.get("chains").unwrap_or(over);
    let Some(map) = chains.as_object() else {
        return;
    };
    for (chain, v) in map {
        if !v.is_object() {
            continue;
        }
        let entry = infra.chains.entry(chain.clone()).or_default();
        if let Some(ids) = v.get("code_ids").and_then(|x| x.as_object()) {
            for (k, n) in ids {
                if let Some(id) = n.as_u64() {
                    entry.code_ids.insert(k.clone(), id);
                }
            }
        }
        if let Some(defs) = v.get("default").and_then(|x| x.as_object()) {
            for (k, n) in defs {
                if let Some(s) = n.as_str() {
                    if !s.is_empty() {
                        entry.default.insert(k.clone(), s.to_string());
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedPreset {
    pub id: String,
    pub purpose: String,
    pub title: String,
    pub chain_id: String,
    pub code_ids: BTreeMap<String, u64>,
    pub instances: BTreeMap<String, String>,
    pub missing_code_ids: Vec<String>,
    pub allowance_msg_types: Vec<String>,
}

impl GrantPreset {
    pub fn resolve(&self, chain_id: &str, infra: &InfraFile) -> ResolvedPreset {
        let chain = infra.chains.get(chain_id);
        let mut code_ids = BTreeMap::new();
        let mut missing = Vec::new();
        for k in self.code_id_keys {
            match chain.and_then(|c| c.code_ids.get(*k).copied()) {
                Some(id) => {
                    code_ids.insert((*k).to_string(), id);
                }
                None => missing.push((*k).to_string()),
            }
        }
        let mut instances = BTreeMap::new();
        if let Some(c) = chain {
            for k in self.default_addr_keys {
                if let Some(a) = c.default.get(*k) {
                    if !a.is_empty() {
                        instances.insert((*k).to_string(), a.clone());
                    }
                }
            }
        }
        let allowance = match self.purpose {
            GrantPurpose::Register => ALLOWED_MSG_TYPES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            GrantPurpose::Mint => MINT_MSG_TYPES.iter().map(|s| (*s).to_string()).collect(),
            GrantPurpose::Onboard => AllowanceScope::registration_only().allowed,
        };
        ResolvedPreset {
            id: self.id.into(),
            purpose: match self.purpose {
                GrantPurpose::Register => "register".into(),
                GrantPurpose::Mint => "mint".into(),
                GrantPurpose::Onboard => "onboard".into(),
            },
            title: self.title.into(),
            chain_id: chain_id.into(),
            code_ids,
            instances,
            missing_code_ids: missing,
            allowance_msg_types: allowance,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CurateBundle {
    pub mode: String,
    pub roles: Value,
    pub preset: ResolvedPreset,
    pub cosmos_msgs: Value,
    pub dao_proposal: Option<Value>,
    pub terpd: String,
}

pub fn curate(
    mode: CurateMode,
    roles: &OnboardRoles,
    preset: &GrantPreset,
    infra: &InfraFile,
    proposal_module: Option<&str>,
    sample_user: &str,
) -> CurateBundle {
    let resolved = preset.resolve(&roles.chain_id, infra);

    let cosmos_msgs = match mode {
        CurateMode::DaoProposal => {
            // Runtime → temp HashMerchant: authz to issue feegrants.
            dao_authz_grants(roles)
        }
        CurateMode::WalletProxy => {
            let mut wallet_roles = roles.clone();
            // granter = operator wallet (roles.runtime overwritten in the bin to --from-addr)
            wallet_roles.runtime = granter_wallet(roles);
            dao_authz_grants(&wallet_roles)
        }
        CurateMode::WalletAsRuntime => {
            let inner = match preset.purpose {
                GrantPurpose::Onboard => crate::onboard_feegrant(
                    roles,
                    &crate::FeegrantRequest {
                        grantee: sample_user.into(),
                        spend_limit: "50000".into(),
                        expiration: None,
                    },
                ),
                GrantPurpose::Register => registration_feegrant(
                    roles,
                    &crate::FeegrantRequest {
                        grantee: sample_user.into(),
                        spend_limit: "25000".into(),
                        expiration: None,
                    },
                ),
                GrantPurpose::Mint => mint_feegrant(
                    roles,
                    &crate::FeegrantRequest {
                        grantee: sample_user.into(),
                        spend_limit: "50000".into(),
                        expiration: None,
                    },
                ),
            };
            json!({
                "chain_id": roles.chain_id,
                "memo": "wallet is runtime: issue scoped user feegrant (no authz hop)",
                "msgs": [inner],
                "authz_exec": authz_exec_feegrant(roles, inner.clone()),
            })
        }
    };

    let dao_proposal = if mode == CurateMode::DaoProposal {
        Some(dao_proposal_envelope(
            roles,
            proposal_module,
            &resolved,
            &cosmos_msgs,
        ))
    } else {
        None
    };

    let terpd = terpd_script(mode, roles, preset, sample_user);

    CurateBundle {
        mode: match mode {
            CurateMode::DaoProposal => "dao-proposal".into(),
            CurateMode::WalletProxy => "wallet-proxy".into(),
            CurateMode::WalletAsRuntime => "wallet-as-runtime".into(),
        },
        roles: json!({
            "runtime": roles.runtime,
            "hashmerchant": roles.hashmerchant,
            "dao": roles.runtime,
            "runtime_key": roles.hashmerchant,
            "chain_id": roles.chain_id,
            "granter": match mode {
                CurateMode::DaoProposal => roles.runtime.clone(),
                _ => granter_wallet(roles),
            },
            "fee_granter": roles.hashmerchant,
        }),
        preset: resolved,
        cosmos_msgs,
        dao_proposal,
        terpd,
    }
}

fn granter_wallet(roles: &OnboardRoles) -> String {
    std::env::var("ONBOARD_FROM_ADDR").unwrap_or_else(|_| roles.runtime.clone())
}

fn dao_proposal_envelope(
    roles: &OnboardRoles,
    proposal_module: Option<&str>,
    preset: &ResolvedPreset,
    grants: &Value,
) -> Value {
    let inner_msgs = grants
        .get("msgs")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let cosmos: Vec<Value> = inner_msgs
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            json!({
                "stargate": {
                    "type_url": m.get("@type").and_then(|t| t.as_str()).unwrap_or(""),
                    "value_json": m,
                    "note": "Encode value_json to protobuf bytes before broadcast (cw-orch / proto-json)."
                }
            })
        })
        .collect();

    json!({
        "target": {
            "dao": roles.runtime,
            "proposal_module": proposal_module.unwrap_or(""),
            "proposal_module_code_id_keys": ["dao:dao_proposal_single", "dao_proposal_single"],
        },
        "execute": {
            "propose": {
                "msg": {
                    "propose": {
                        "title": format!("Onboard grant: {}", preset.title),
                        "description": format!(
                            "Authorize runtime {} to issue AllowedMsgAllowance feegrants.\n\
                             Preset `{}` on {}.\n\
                             Code ids: {:?}\n\
                             Instances: {:?}\n\
                             Allowance: {:?}\n\
                             Runtime may *not* bank-send.",
                            roles.hashmerchant,
                            preset.id,
                            preset.chain_id,
                            preset.code_ids,
                            preset.instances,
                            preset.allowance_msg_types
                        ),
                        "msgs": cosmos
                    }
                }
            }
        },
        "authz_msg_types": AUTHZ_MSG_TYPES,
    })
}

fn terpd_script(mode: CurateMode, roles: &OnboardRoles, preset: &GrantPreset, user: &str) -> String {
    let chain = &roles.chain_id;
    let denom = crate::fee_denom();
    let rt = &roles.hashmerchant;
    let from = match mode {
        CurateMode::DaoProposal => "DAO_MULTISIG_OR_PROP".to_string(),
        _ => std::env::var("ONBOARD_FROM").unwrap_or_else(|_| "wallet".into()),
    };
    match mode {
        CurateMode::DaoProposal => format!(
            "# Submit the curated proposal (after proto-encoding stargate values):\n\
             # terpd tx wasm execute {dao} dao-proposal.execute.json --from {from} --chain-id {chain} --fees 5000{denom} --yes\n\
             # After yes-vote: runtime {rt} may MsgGrantAllowance only.",
            dao = roles.runtime,
        ),
        CurateMode::WalletProxy => format!(
            "terpd tx authz grant {rt} generic \\\n\
             --msg-type /cosmos.feegrant.v1beta1.MsgGrantAllowance \\\n\
             --from {from} --chain-id {chain} --fees 5000{denom} --yes\n\
             terpd tx authz grant {rt} generic \\\n\
             --msg-type /cosmos.feegrant.v1beta1.MsgRevokeAllowance \\\n\
             --from {from} --chain-id {chain} --fees 5000{denom} --yes"
        ),
        CurateMode::WalletAsRuntime => {
            let types = match preset.purpose {
                GrantPurpose::Register => ALLOWED_MSG_TYPES.join(","),
                GrantPurpose::Mint => MINT_MSG_TYPES.join(","),
                GrantPurpose::Onboard => AllowanceScope::registration_only().allowed.join(","),
            };
            format!(
                "terpd tx feegrant grant {rt} {user} \\\n\
                 --allowed-messages {types} \\\n\
                 --spend-limit 25000{denom} \\\n\
                 --from {from} --chain-id {chain} --fees 5000{denom} --yes"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norick_preset_sees_code_72_on_120u() {
        let infra = load_infra(None);
        let p = preset("norick").unwrap();
        let r = p.resolve("120u-1", &infra);
        assert_eq!(r.code_ids.get("headstash:cw_norick"), Some(&72));
        assert_eq!(r.purpose, "mint");
    }

    #[test]
    fn register_preset_sees_passkey_on_120u() {
        let infra = load_infra(None);
        let r = preset("register").unwrap().resolve("120u-1", &infra);
        assert_eq!(r.code_ids.get("terp-passkey"), Some(&4));
        assert!(r.instances.get("terp-passkey").unwrap().starts_with("terp1"));
    }

    #[test]
    fn svg_preset_on_morocco() {
        let infra = load_infra(None);
        let r = preset("svg").unwrap().resolve("morocco-1", &infra);
        assert_eq!(r.code_ids.get("cw-svg-minter"), Some(&72));
        assert!(!r.instances["cw721_svg"].is_empty());
    }

    #[test]
    fn dao_bundle_has_proposal() {
        let infra = load_infra(None);
        let roles = OnboardRoles {
            runtime: "terp1dao".into(),
            hashmerchant: "terp1rt".into(),
            chain_id: "120u-1".into(),
        };
        let b = curate(
            CurateMode::DaoProposal,
            &roles,
            preset("register").unwrap(),
            &infra,
            Some("terp1prop"),
            "terp1user",
        );
        assert!(b.dao_proposal.is_some());
        assert_eq!(b.cosmos_msgs["msgs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn wallet_as_runtime_is_feegrant_only() {
        let infra = load_infra(None);
        let roles = OnboardRoles {
            runtime: "terp1dao".into(),
            hashmerchant: "terp1wallet".into(),
            chain_id: "120u-1".into(),
        };
        let b = curate(
            CurateMode::WalletAsRuntime,
            &roles,
            preset("norick").unwrap(),
            &infra,
            None,
            "terp1user",
        );
        assert!(b.dao_proposal.is_none());
        assert_eq!(
            b.cosmos_msgs["msgs"][0]["allowance"]["allowed_messages"][0],
            "/cosmwasm.wasm.v1.MsgExecuteContract"
        );
    }

    #[test]
    fn wallet_proxy_grant_is_runtime_to_hashmerchant() {
        std::env::remove_var("ONBOARD_FROM_ADDR");
        std::env::remove_var("ONBOARD_ALLOW_COLLAPSE");
        let infra = load_infra(None);
        let roles = OnboardRoles {
            runtime: "terp1runtimeaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            hashmerchant: "terp1hashmerchantaaaaaaaaaaaaaaaaaaaaaa".into(),
            chain_id: "morocco-1".into(),
        };
        let b = curate(
            CurateMode::WalletProxy,
            &roles,
            preset("register").unwrap(),
            &infra,
            None,
            "terp1USER",
        );
        assert_eq!(b.mode, "wallet-proxy");
        let msgs = b.cosmos_msgs["msgs"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        for m in msgs {
            assert_eq!(m["@type"], "/cosmos.authz.v1beta1.MsgGrant");
            assert_eq!(m["granter"], roles.runtime);
            assert_eq!(m["grantee"], roles.hashmerchant);
            let msg = m["grant"]["authorization"]["msg"].as_str().unwrap();
            assert!(AUTHZ_MSG_TYPES.contains(&msg));
            assert!(!msg.contains("MsgSend"));
        }
        assert_eq!(b.roles["fee_granter"], roles.hashmerchant);
        assert_ne!(b.roles["fee_granter"], roles.runtime);
    }
}
