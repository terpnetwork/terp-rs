#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult,
};

use crate::error::ContractError;
use crate::msg::{
    CeremonyInfo, CeremonyResponse, CeremonyStatus, ConfigResponse, ExecuteMsg, InstantiateMsg,
    RegistrationGate,
    IsRegisteredResponse, IsSpentResponse, QueryMsg, RawSpentKeyResponse, SudoMsg,
};
use crate::raw_keys::SPENT_NAMESPACE;
use crate::state::{spent_key, Ceremony, ClerkLeaf, Config, CEREMONIES, CLERK_LEAVES, CONFIG, REGISTRATIONS, SPENT};

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let admin = match msg.admin {
        Some(a) => Some(deps.api.addr_validate(&a)?.to_string()),
        None => None,
    };
    let max = if msg.max_nullifiers_per_mark == 0 {
        8
    } else {
        msg.max_nullifiers_per_mark
    };
    CONFIG.save(
        deps.storage,
        &Config {
            admin,
            max_nullifiers_per_mark: max,
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::StartCeremony {
            session_id,
            domain,
            registration_open,
            anchor_policy,
            registration_gate,
            clerk,
            membership_module,
            vm_verifier,
        } => start_ceremony(
            deps,
            info,
            session_id,
            domain,
            registration_open,
            anchor_policy,
            registration_gate,
            clerk,
            membership_module,
            vm_verifier,
        ),
        ExecuteMsg::CloseCeremony { session_id } => close_ceremony(deps, info, session_id),
        ExecuteMsg::Register {
            session_id,
            leaf_commit,
            proof,
        } => register(deps, env, info, session_id, leaf_commit, proof),
        ExecuteMsg::AttestLeaf {
            session_id,
            leaf_commit,
        } => attest_leaf(deps, info, session_id, leaf_commit),
        ExecuteMsg::SetRegistrationOpen { session_id, open } => {
            set_registration_open(deps, info, session_id, open)
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: cfg.admin,
                max_nullifiers_per_mark: cfg.max_nullifiers_per_mark,
            })
        }
        QueryMsg::Ceremony { session_id } => {
            let c = CEREMONIES.load(deps.storage, &session_id)?;
            to_json_binary(&CeremonyResponse {
                ceremony: CeremonyInfo {
                    session_id,
                    domain: c.domain,
                    status: c.status,
                    registration_open: c.registration_open,
                    anchor_policy: c.anchor_policy,
                    registration_gate: c.registration_gate,
                    clerk: c.clerk,
                    membership_module: c.membership_module,
                    vm_verifier: c.vm_verifier,
                },
            })
        }
        QueryMsg::IsSpent {
            domain,
            session_id,
            nullifier,
        } => {
            let spent = SPENT.has(
                deps.storage,
                spent_key(&domain, &session_id, nullifier.as_slice()),
            );
            to_json_binary(&IsSpentResponse { spent })
        }
        QueryMsg::IsRegistered { session_id, addr } => {
            let leaf = REGISTRATIONS.may_load(deps.storage, (&session_id, &addr))?;
            to_json_binary(&IsRegisteredResponse {
                registered: leaf.is_some(),
                leaf_commit: leaf.filter(|s| !s.is_empty()),
            })
        }
        QueryMsg::RawSpentKey {
            domain,
            session_id,
            nullifier,
        } => to_json_binary(&RawSpentKeyResponse {
            map_namespace: SPENT_NAMESPACE.into(),
            domain,
            session_id,
            nullifier,
        }),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::MarkSpent {
            domain,
            session_id,
            nullifiers,
        } => mark_spent(deps, domain, session_id, nullifiers),
    }
}

// ─── execute handlers ────────────────────────────────────────────────────────

fn ensure_admin(deps: Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    match cfg.admin {
        Some(ref a) if a == info.sender.as_str() => Ok(()),
        Some(_) => Err(ContractError::Unauthorized),
        // No admin: open for tests (DAO path will set admin or use sudo proposal).
        None => Ok(()),
    }
}

pub fn start_ceremony(
    deps: DepsMut,
    info: MessageInfo,
    session_id: String,
    domain: String,
    registration_open: bool,
    anchor_policy: Option<String>,
    registration_gate: RegistrationGate,
    clerk: Option<String>,
    membership_module: Option<String>,
    vm_verifier: Option<String>,
) -> Result<Response, ContractError> {
    ensure_admin(deps.as_ref(), &info)?;
    if session_id.is_empty() {
        return Err(ContractError::EmptySession);
    }
    if domain.is_empty() {
        return Err(ContractError::EmptyDomain);
    }
    if CEREMONIES.has(deps.storage, &session_id) {
        return Err(ContractError::CeremonyExists { session_id });
    }
    CEREMONIES.save(
        deps.storage,
        &session_id,
        &Ceremony {
            domain: domain.clone(),
            status: CeremonyStatus::Active,
            registration_open,
            anchor_policy,
            registration_gate,
            clerk,
            membership_module,
            vm_verifier,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "start_ceremony")
        .add_attribute("session_id", session_id)
        .add_attribute("domain", domain))
}

fn close_ceremony(
    deps: DepsMut,
    info: MessageInfo,
    session_id: String,
) -> Result<Response, ContractError> {
    ensure_admin(deps.as_ref(), &info)?;
    let mut c = CEREMONIES
        .may_load(deps.storage, &session_id)?
        .ok_or_else(|| ContractError::CeremonyNotFound {
            session_id: session_id.clone(),
        })?;
    c.status = CeremonyStatus::Closed;
    c.registration_open = false;
    CEREMONIES.save(deps.storage, &session_id, &c)?;
    Ok(Response::new()
        .add_attribute("action", "close_ceremony")
        .add_attribute("session_id", session_id))
}

fn register(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    session_id: String,
    leaf_commit: Option<String>,
    proof: Option<cosmwasm_std::Binary>,
) -> Result<Response, ContractError> {
    let c = CEREMONIES
        .may_load(deps.storage, &session_id)?
        .ok_or_else(|| ContractError::CeremonyNotFound {
            session_id: session_id.clone(),
        })?;
    if c.status != CeremonyStatus::Active {
        return Err(ContractError::CeremonyNotActive {
            session_id: session_id.clone(),
            status: c.status.as_str().into(),
        });
    }
    if !c.registration_open {
        return Err(ContractError::RegistrationClosed {
            session_id: session_id.clone(),
        });
    }
    match c.registration_gate {
        RegistrationGate::Open => {}
        RegistrationGate::PrePropose => return Err(ContractError::PreProposeUnwired),
        RegistrationGate::VmProof => {
            let verifier = c.vm_verifier.as_ref().ok_or_else(|| ContractError::GateDenied {
                reason: "vm_verifier not set".into(),
            })?;
            let proof = proof.ok_or_else(|| ContractError::GateDenied {
                reason: "proof required for vm_proof gate".into(),
            })?;
            #[derive(serde::Serialize)]
            struct Vq<'a> {
                verify_min_holders: VqInner<'a>,
            }
            #[derive(serde::Serialize)]
            struct VqInner<'a> {
                session_id: &'a str,
                registrant: &'a str,
                proof: &'a cosmwasm_std::Binary,
            }
            #[derive(serde::Deserialize)]
            struct Vr {
                ok: bool,
            }
            let q = Vq {
                verify_min_holders: VqInner {
                    session_id: &session_id,
                    registrant: info.sender.as_str(),
                    proof: &proof,
                },
            };
            let ans: Vr = deps
                .querier
                .query_wasm_smart(verifier, &q)
                .map_err(|_| ContractError::GateDenied {
                    reason: "vm verifier query failed".into(),
                })?;
            if !ans.ok {
                return Err(ContractError::GateDenied {
                    reason: "min-holders proof rejected".into(),
                });
            }
        }
        RegistrationGate::ClerkAttested => {
            let leaf = leaf_commit
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(ContractError::LeafCommitRequired)?;
            let rec = CLERK_LEAVES
                .may_load(deps.storage, (&session_id, leaf))?
                .ok_or_else(|| ContractError::GateDenied {
                    reason: "leaf_commit not attested by clerk".into(),
                })?;
            if let Some(owner) = rec.consumed_by {
                if owner != info.sender.as_str() {
                    return Err(ContractError::LeafConsumed { addr: owner });
                }
            }
        }
        RegistrationGate::DaoMember => {
            let module = c.membership_module.as_ref().ok_or_else(|| {
                ContractError::GateDenied {
                    reason: "membership_module not set".into(),
                }
            })?;
            // Query site for voting power — fail closed if the module is missing.
            #[derive(serde::Serialize)]
            struct VpQuery<'a> {
                voting_power_at_height: VpInner<'a>,
            }
            #[derive(serde::Serialize)]
            struct VpInner<'a> {
                address: &'a str,
            }
            let q = VpQuery {
                voting_power_at_height: VpInner {
                    address: info.sender.as_str(),
                },
            };
            let raw: Result<cosmwasm_std::Binary, _> =
                deps.querier.query_wasm_smart(module, &q);
            if raw.is_err() {
                return Err(ContractError::GateDenied {
                    reason: "membership query failed".into(),
                });
            }
        }
    }
    let addr = info.sender.as_str();
    if REGISTRATIONS.has(deps.storage, (&session_id, addr)) {
        return Err(ContractError::AlreadyRegistered {
            addr: addr.to_string(),
        });
    }
    let stored_leaf = leaf_commit
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    if c.registration_gate == RegistrationGate::ClerkAttested && !stored_leaf.is_empty() {
        CLERK_LEAVES.save(
            deps.storage,
            (&session_id, stored_leaf.as_str()),
            &ClerkLeaf {
                consumed_by: Some(addr.to_string()),
            },
        )?;
    }
    REGISTRATIONS.save(deps.storage, (&session_id, addr), &stored_leaf)?;
    Ok(Response::new()
        .add_attribute("action", "register")
        .add_attribute("session_id", session_id)
        .add_attribute("addr", addr))
}

fn attest_leaf(
    deps: DepsMut,
    info: MessageInfo,
    session_id: String,
    leaf_commit: String,
) -> Result<Response, ContractError> {
    let c = CEREMONIES
        .may_load(deps.storage, &session_id)?
        .ok_or_else(|| ContractError::CeremonyNotFound {
            session_id: session_id.clone(),
        })?;
    if c.status != CeremonyStatus::Active {
        return Err(ContractError::CeremonyNotActive {
            session_id: session_id.clone(),
            status: c.status.as_str().into(),
        });
    }
    let clerk_ok = c
        .clerk
        .as_deref()
        .map(|a| a == info.sender.as_str())
        .unwrap_or(false);
    if clerk_ok {
        // clerk may attest
    } else if c.clerk.is_none() {
        ensure_admin(deps.as_ref(), &info)?;
        let cfg = crate::state::CONFIG.load(deps.storage)?;
        if cfg.admin.is_none() {
            return Err(ContractError::Unauthorized);
        }
    } else {
        return Err(ContractError::Unauthorized);
    }
    let leaf = leaf_commit.trim();
    if leaf.is_empty() {
        return Err(ContractError::LeafCommitRequired);
    }
    if CLERK_LEAVES.has(deps.storage, (&session_id, leaf)) {
        return Err(ContractError::LeafAlreadyAttested);
    }
    CLERK_LEAVES.save(
        deps.storage,
        (&session_id, leaf),
        &ClerkLeaf { consumed_by: None },
    )?;
    Ok(Response::new()
        .add_attribute("action", "attest_leaf")
        .add_attribute("session_id", session_id)
        .add_attribute("leaf_commit", leaf))
}

fn set_registration_open(
    deps: DepsMut,
    info: MessageInfo,
    session_id: String,
    open: bool,
) -> Result<Response, ContractError> {
    ensure_admin(deps.as_ref(), &info)?;
    let mut c = CEREMONIES
        .may_load(deps.storage, &session_id)?
        .ok_or_else(|| ContractError::CeremonyNotFound {
            session_id: session_id.clone(),
        })?;
    c.registration_open = open;
    CEREMONIES.save(deps.storage, &session_id, &c)?;
    Ok(Response::new()
        .add_attribute("action", "set_registration_open")
        .add_attribute("open", open.to_string()))
}

// ─── MarkSpent (sudo / privileged) ───────────────────────────────────────────

/// Mark nullifiers spent. Ceremony must be Active. Fail-closed on double-spend.
pub fn mark_spent(
    deps: DepsMut,
    domain: String,
    session_id: String,
    nullifiers: Vec<Binary>,
) -> Result<Response, ContractError> {
    if domain.is_empty() {
        return Err(ContractError::EmptyDomain);
    }
    if session_id.is_empty() {
        return Err(ContractError::EmptySession);
    }
    if nullifiers.is_empty() {
        return Err(ContractError::EmptyNullifiers);
    }
    let cfg = CONFIG.load(deps.storage)?;
    let got = nullifiers.len() as u32;
    if got > cfg.max_nullifiers_per_mark {
        return Err(ContractError::TooManyNullifiers {
            got,
            max: cfg.max_nullifiers_per_mark,
        });
    }

    let c = CEREMONIES
        .may_load(deps.storage, &session_id)?
        .ok_or_else(|| ContractError::CeremonyNotFound {
            session_id: session_id.clone(),
        })?;
    if c.status != CeremonyStatus::Active {
        return Err(ContractError::CeremonyNotActive {
            session_id: session_id.clone(),
            status: c.status.as_str().into(),
        });
    }

    for nf in &nullifiers {
        if nf.is_empty() {
            return Err(ContractError::EmptyNullifierBytes);
        }
        let key = spent_key(&domain, &session_id, nf.as_slice());
        if SPENT.has(deps.storage, key) {
            return Err(ContractError::AlreadySpent {
                domain: domain.clone(),
                session: session_id.clone(),
            });
        }
        SPENT.save(deps.storage, key, &Empty {})?;
    }

    Ok(Response::new()
        .add_attribute("action", "mark_spent")
        .add_attribute("domain", domain)
        .add_attribute("session_id", session_id)
        .add_attribute("spent_count", nullifiers.len().to_string()))
}

/// Read helper for gate / tests: true if spent.
pub fn is_spent(deps: Deps, domain: &str, session_id: &str, nullifier: &[u8]) -> bool {
    SPENT.has(deps.storage, spent_key(domain, session_id, nullifier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{from_json, Addr, Coin, MessageInfo};

    fn human(label: &str) -> String {
        cosmwasm_std::testing::MockApi::default()
            .addr_make(label)
            .into_string()
    }

    fn mock_info(sender: &str, funds: &[Coin]) -> MessageInfo {
        message_info(&Addr::unchecked(human(sender)), funds)
    }

    fn setup() -> (cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >,) {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg {
                admin: Some(human("terp1admin")),
                max_nullifiers_per_mark: 8,
            },
        )
        .unwrap();
        (deps,)
    }

    #[test]
    fn start_ceremony_register_mark_spent() {
        let (mut deps,) = setup();
        let admin = mock_info("terp1admin", &[]);

        execute(
            deps.as_mut(),
            mock_env(),
            admin.clone(),
            ExecuteMsg::StartCeremony {
                session_id: "round-1".into(),
                domain: "terp.vote.v1".into(),
                registration_open: true,
                anchor_policy: Some("m1-admin-roots".into()),
                registration_gate: RegistrationGate::Open,
                clerk: None,
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap();

        // non-admin cannot start
        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1other", &[]),
            ExecuteMsg::StartCeremony {
                session_id: "round-2".into(),
                domain: "terp.vote.v1".into(),
                registration_open: true,
                anchor_policy: None,
                registration_gate: RegistrationGate::Open,
                clerk: None,
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Unauthorized));

        // register participant
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1voter", &[]),
            ExecuteMsg::Register { session_id: "round-1".into(), leaf_commit: None,
                proof: None },
        )
        .unwrap();

        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::IsRegistered {
                session_id: "round-1".into(),
                addr: human("terp1voter"),
            },
        )
        .unwrap();
        let reg: IsRegisteredResponse = from_json(q).unwrap();
        assert!(reg.registered);

        // mark spent via sudo
        sudo(
            deps.as_mut(),
            mock_env(),
            SudoMsg::MarkSpent {
                domain: "terp.vote.v1".into(),
                session_id: "round-1".into(),
                nullifiers: vec![Binary::from(b"nf-aaa")],
            },
        )
        .unwrap();

        assert!(is_spent(
            deps.as_ref(),
            "terp.vote.v1",
            "round-1",
            b"nf-aaa"
        ));

        // double spend fails
        let err = sudo(
            deps.as_mut(),
            mock_env(),
            SudoMsg::MarkSpent {
                domain: "terp.vote.v1".into(),
                session_id: "round-1".into(),
                nullifiers: vec![Binary::from(b"nf-aaa")],
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::AlreadySpent { .. }));

        // domain isolation
        sudo(
            deps.as_mut(),
            mock_env(),
            SudoMsg::MarkSpent {
                domain: "other.domain".into(),
                session_id: "round-1".into(),
                nullifiers: vec![Binary::from(b"nf-aaa")],
            },
        )
        .unwrap();
    }

    /// Public execute has no MarkSpent variant (T-VOTE-01 / residual R-mark-spent-sudo).
    /// Gate ConfirmExecution currently emits Wasm execute MarkSpent proxy — ceremony only
    /// accepts sudo. This locks the fail-closed public surface.
    #[test]
    fn mark_spent_not_public_execute() {
        // Snake-case JSON that would be SudoMsg::MarkSpent must not parse as ExecuteMsg.
        let raw = r#"{"mark_spent":{"domain":"d","session_id":"s","nullifiers":[]}}"#;
        assert!(
            from_json::<ExecuteMsg>(raw.as_bytes()).is_err(),
            "MarkSpent must not deserialize as public ExecuteMsg"
        );

        // Same payload is valid SudoMsg (privileged path only).
        let sudo_msg: SudoMsg = from_json(raw.as_bytes()).unwrap();
        assert!(matches!(
            sudo_msg,
            SudoMsg::MarkSpent {
                domain: _,
                session_id: _,
                nullifiers: _
            }
        ));
    }

    #[test]
    fn spent_namespace_stable() {
        assert_eq!(SPENT_NAMESPACE, "spent");
        assert_eq!(crate::raw_keys::CEREMONY_NAMESPACE, "ceremony");
    }

    #[test]
    fn mark_spent_requires_active_ceremony() {
        let (mut deps,) = setup();
        let admin = mock_info("terp1admin", &[]);
        execute(
            deps.as_mut(),
            mock_env(),
            admin.clone(),
            ExecuteMsg::StartCeremony {
                session_id: "s".into(),
                domain: "d".into(),
                registration_open: false,
                anchor_policy: None,
                registration_gate: RegistrationGate::Open,
                clerk: None,
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            admin,
            ExecuteMsg::CloseCeremony {
                session_id: "s".into(),
            },
        )
        .unwrap();

        let err = sudo(
            deps.as_mut(),
            mock_env(),
            SudoMsg::MarkSpent {
                domain: "d".into(),
                session_id: "s".into(),
                nullifiers: vec![Binary::from(b"nf")],
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::CeremonyNotActive { .. }));
    }

    #[test]
    fn ceremony_query_and_raw_key_doc() {
        let (mut deps,) = setup();
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1admin", &[]),
            ExecuteMsg::StartCeremony {
                session_id: "sess".into(),
                domain: "terp.vote.v1".into(),
                registration_open: true,
                anchor_policy: None,
                registration_gate: RegistrationGate::Open,
                clerk: None,
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap();

        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Ceremony {
                session_id: "sess".into(),
            },
        )
        .unwrap();
        let resp: CeremonyResponse = from_json(q).unwrap();
        assert_eq!(resp.ceremony.status, CeremonyStatus::Active);

        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::RawSpentKey {
                domain: "terp.vote.v1".into(),
                session_id: "sess".into(),
                nullifier: Binary::from(b"nf"),
            },
        )
        .unwrap();
        let rk: RawSpentKeyResponse = from_json(q).unwrap();
        assert_eq!(rk.map_namespace, "spent");
    }

    #[test]
    fn _addr_api_sanity() {
        // keep Addr import warm for future multi-test
        let _ = Addr::unchecked("terp1x");
    }
    #[test]
    fn clerk_attested_requires_leaf_then_register() {
        let (mut deps,) = setup();
        let admin = mock_info("terp1admin", &[]);
        execute(
            deps.as_mut(),
            mock_env(),
            admin.clone(),
            ExecuteMsg::StartCeremony {
                session_id: "s-clerk".into(),
                domain: "d".into(),
                registration_open: true,
                anchor_policy: None,
                registration_gate: RegistrationGate::ClerkAttested,
                clerk: Some(human("terp1clerk")),
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap();

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1voter", &[]),
            ExecuteMsg::Register { session_id: "s-clerk".into(), leaf_commit: Some("leaf-a".into()),
                proof: None },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::GateDenied { .. }));

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1clerk", &[]),
            ExecuteMsg::AttestLeaf {
                session_id: "s-clerk".into(),
                leaf_commit: "leaf-a".into(),
            },
        )
        .unwrap();

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1voter", &[]),
            ExecuteMsg::Register { session_id: "s-clerk".into(), leaf_commit: Some("leaf-a".into()),
                proof: None },
        )
        .unwrap();

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1other", &[]),
            ExecuteMsg::Register { session_id: "s-clerk".into(), leaf_commit: Some("leaf-a".into()),
                proof: None },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::LeafConsumed { .. }));
    }

    #[test]
    fn dao_member_without_module_denied() {
        let (mut deps,) = setup();
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1admin", &[]),
            ExecuteMsg::StartCeremony {
                session_id: "s-dao".into(),
                domain: "d".into(),
                registration_open: true,
                anchor_policy: None,
                registration_gate: RegistrationGate::DaoMember,
                clerk: None,
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap();
        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1voter", &[]),
            ExecuteMsg::Register { session_id: "s-dao".into(), leaf_commit: None,
                proof: None },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::GateDenied { .. }));
    }

    #[test]
    fn pre_propose_gate_unwired() {
        let (mut deps,) = setup();
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1admin", &[]),
            ExecuteMsg::StartCeremony {
                session_id: "s-pp".into(),
                domain: "d".into(),
                registration_open: true,
                anchor_policy: None,
                registration_gate: RegistrationGate::PrePropose,
                clerk: None,
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap();
        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1voter", &[]),
            ExecuteMsg::Register { session_id: "s-pp".into(), leaf_commit: None,
                proof: None },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::PreProposeUnwired));
    }

    #[test]
    fn vm_proof_without_verifier_denied() {
        let (mut deps,) = setup();
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1admin", &[]),
            ExecuteMsg::StartCeremony {
                session_id: "s-vm".into(),
                domain: "d".into(),
                registration_open: true,
                anchor_policy: None,
                registration_gate: RegistrationGate::VmProof,
                clerk: None,
                membership_module: None,
                vm_verifier: None,
            },
        )
        .unwrap();
        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1voter", &[]),
            ExecuteMsg::Register {
                session_id: "s-vm".into(),
                leaf_commit: None,
                proof: Some(cosmwasm_std::Binary::from(b"x")),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::GateDenied { .. }));
    }

}
