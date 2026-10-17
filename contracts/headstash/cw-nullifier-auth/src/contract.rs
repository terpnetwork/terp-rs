#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult,
};

use crate::auth_types::{
    AuthenticationRequest, ConfirmExecutionRequest, LocalAny, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};
use crate::error::ContractError;
use crate::msg::{
    AuthenticatorParams, AuthenticatorSudoMsg, ConfigResponse, ExecuteMsg, InstantiateMsg,
    IsSpentResponse, NullifierAuthPayload, QueryMsg,
};
use crate::state::{spent_key, Config, CONFIG, SPENT};

pub const DEFAULT_MAX_NULLIFIERS: u32 = 8;

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
    CONFIG.save(deps.storage, &Config { admin })?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    // Authenticator spends happen only via sudo ConfirmExecution.
    Err(ContractError::Std(cosmwasm_std::StdError::msg(
        "execute disabled; use authenticator sudo lifecycle",
    )))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
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
        QueryMsg::Config {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse { admin: cfg.admin })
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, _env: Env, msg: AuthenticatorSudoMsg) -> Result<Response, ContractError> {
    match msg {
        AuthenticatorSudoMsg::OnAuthenticatorAdded(req) => on_authenticator_added(deps, req),
        AuthenticatorSudoMsg::OnAuthenticatorRemoved(req) => on_authenticator_removed(deps, req),
        AuthenticatorSudoMsg::Authenticate(req) => authenticate(deps, req),
        AuthenticatorSudoMsg::Track(req) => track(deps, req),
        AuthenticatorSudoMsg::ConfirmExecution(req) => confirm_execution(deps, req),
    }
}

// ─── hooks ───────────────────────────────────────────────────────────────────

fn on_authenticator_added(
    _deps: DepsMut,
    req: OnAuthenticatorAddedRequest,
) -> Result<Response, ContractError> {
    let params = parse_params(req.authenticator_params.as_ref())?;
    validate_params(&params)?;
    Ok(Response::new().add_attribute("action", "on_authenticator_added"))
}

fn on_authenticator_removed(
    _deps: DepsMut,
    _req: OnAuthenticatorRemovedRequest,
) -> Result<Response, ContractError> {
    // Spent set retained (append-only audit).
    Ok(Response::new().add_attribute("action", "on_authenticator_removed"))
}

/// Read-only: reject if any nullifier already spent. Writes are discarded by ante CacheContext.
fn authenticate(deps: DepsMut, req: AuthenticationRequest) -> Result<Response, ContractError> {
    let params = parse_params(req.authenticator_params.as_ref())?;
    let payload = parse_payload_from_auth_data(&req.signature)?;
    let session = resolve_session(&params, &payload)?;
    check_nullifiers_fresh(deps.as_ref(), &params, &session, &payload.nullifiers)?;
    Ok(Response::new().add_attribute("action", "authenticate"))
}

/// Bookkeeping only — do **not** mark spent (Track commits even if execute fails).
fn track(_deps: DepsMut, _req: TrackRequest) -> Result<Response, ContractError> {
    Ok(Response::new().add_attribute("action", "track"))
}

/// After successful execution path: mark nullifiers spent.
/// Nullifiers come from msg (Go CosmwasmAuthenticator omits signature on ConfirmExecution).
fn confirm_execution(
    deps: DepsMut,
    req: ConfirmExecutionRequest,
) -> Result<Response, ContractError> {
    let params = parse_params(req.authenticator_params.as_ref())?;
    let payload = extract_payload_for_confirm(&req.msg)?;
    let session = resolve_session(&params, &payload)?;
    spend_nullifiers(deps, &params, &session, &payload.nullifiers)?;
    Ok(Response::new()
        .add_attribute("action", "confirm_execution")
        .add_attribute("spent_count", payload.nullifiers.len().to_string()))
}

// ─── pure / shared logic (unit-tested) ───────────────────────────────────────

pub fn validate_params(params: &AuthenticatorParams) -> Result<(), ContractError> {
    if params.domain.is_empty() {
        return Err(ContractError::InvalidParams("domain must be non-empty".into()));
    }
    if params.max_nullifiers_per_tx == 0 {
        return Err(ContractError::InvalidParams(
            "max_nullifiers_per_tx must be >= 1".into(),
        ));
    }
    if params.require_session {
        if let Some(ref s) = params.session_id {
            if s.is_empty() {
                return Err(ContractError::InvalidParams(
                    "session_id in params is empty but require_session".into(),
                ));
            }
        }
    }
    Ok(())
}

pub fn parse_params(raw: Option<&Binary>) -> Result<AuthenticatorParams, ContractError> {
    let Some(bz) = raw else {
        return Err(ContractError::InvalidParams("missing authenticator_params".into()));
    };
    cosmwasm_std::from_json(bz).map_err(|e| ContractError::InvalidParams(e.to_string()))
}

pub fn parse_payload_from_auth_data(sig: &Binary) -> Result<NullifierAuthPayload, ContractError> {
    cosmwasm_std::from_json(sig).map_err(|e| ContractError::InvalidPayload(e.to_string()))
}

/// ConfirmExecution: parse NullifierAuthPayload from msg.value JSON.
/// `type_url` may be any string; value must be JSON payload (scaffold carrier).
pub fn extract_payload_for_confirm(msg: &LocalAny) -> Result<NullifierAuthPayload, ContractError> {
    cosmwasm_std::from_json(&msg.value).map_err(|_| ContractError::NullifiersNotInMsg)
}

pub fn resolve_session(
    params: &AuthenticatorParams,
    payload: &NullifierAuthPayload,
) -> Result<String, ContractError> {
    match (&params.session_id, &payload.session_id) {
        (Some(p), Some(a)) if p == a => Ok(p.clone()),
        (Some(p), Some(a)) => Err(ContractError::SessionMismatch {
            params: Some(p.clone()),
            payload: Some(a.clone()),
        }),
        (Some(p), None) => Ok(p.clone()),
        (None, Some(a)) => Ok(a.clone()),
        (None, None) => {
            if params.require_session {
                Err(ContractError::MissingSession)
            } else {
                Ok(String::new())
            }
        }
    }
}

pub fn validate_nullifier_list(
    params: &AuthenticatorParams,
    nullifiers: &[Binary],
) -> Result<(), ContractError> {
    if nullifiers.is_empty() {
        return Err(ContractError::EmptyNullifiers);
    }
    let max = params.max_nullifiers_per_tx;
    let got = nullifiers.len() as u32;
    if got > max {
        return Err(ContractError::TooManyNullifiers { got, max });
    }
    for nf in nullifiers {
        if nf.is_empty() {
            return Err(ContractError::EmptyNullifierBytes);
        }
    }
    Ok(())
}

pub fn check_nullifiers_fresh(
    deps: Deps,
    params: &AuthenticatorParams,
    session: &str,
    nullifiers: &[Binary],
) -> Result<(), ContractError> {
    validate_nullifier_list(params, nullifiers)?;
    for nf in nullifiers {
        if SPENT.has(
            deps.storage,
            spent_key(&params.domain, session, nf.as_slice()),
        ) {
            return Err(ContractError::AlreadySpent {
                domain: params.domain.clone(),
                session: session.to_string(),
            });
        }
    }
    Ok(())
}

pub fn spend_nullifiers(
    deps: DepsMut,
    params: &AuthenticatorParams,
    session: &str,
    nullifiers: &[Binary],
) -> Result<(), ContractError> {
    validate_nullifier_list(params, nullifiers)?;
    for nf in nullifiers {
        let key = spent_key(&params.domain, session, nf.as_slice());
        if SPENT.has(deps.storage, key) {
            return Err(ContractError::AlreadySpent {
                domain: params.domain.clone(),
                session: session.to_string(),
            });
        }
        SPENT.save(deps.storage, key, &Empty {})?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{Addr, Coin, MessageInfo};

    fn human(label: &str) -> String {
        cosmwasm_std::testing::MockApi::default()
            .addr_make(label)
            .into_string()
    }

    fn mock_info(sender: &str, funds: &[Coin]) -> MessageInfo {
        message_info(&Addr::unchecked(sender), funds)
    }
    use cosmwasm_std::{from_json, Binary};

    fn params(domain: &str, session: Option<&str>) -> AuthenticatorParams {
        AuthenticatorParams {
            domain: domain.into(),
            session_id: session.map(str::to_string),
            require_session: true,
            max_nullifiers_per_tx: 8,
        }
    }

    fn params_bin(p: &AuthenticatorParams) -> Binary {
        to_json_binary(p).unwrap()
    }

    fn payload(session: Option<&str>, nfs: &[&[u8]]) -> NullifierAuthPayload {
        NullifierAuthPayload {
            session_id: session.map(str::to_string),
            nullifiers: nfs.iter().map(|b| Binary::from(*b)).collect(),
        }
    }

    fn payload_bin(p: &NullifierAuthPayload) -> Binary {
        to_json_binary(p).unwrap()
    }

    #[test]
    fn validate_params_rejects_empty_domain() {
        let p = params("", Some("s1"));
        assert!(matches!(
            validate_params(&p),
            Err(ContractError::InvalidParams(_))
        ));
    }

    #[test]
    fn resolve_session_inheritance_and_mismatch() {
        let p = params("vote.v1", Some("round-1"));
        let ok = payload(None, &[b"nf1"]);
        assert_eq!(resolve_session(&p, &ok).unwrap(), "round-1");

        let bad = payload(Some("round-2"), &[b"nf1"]);
        assert!(matches!(
            resolve_session(&p, &bad),
            Err(ContractError::SessionMismatch { .. })
        ));

        let p2 = params("vote.v1", None);
        let from_payload = payload(Some("sess"), &[b"nf1"]);
        assert_eq!(resolve_session(&p2, &from_payload).unwrap(), "sess");

        let missing = payload(None, &[b"nf1"]);
        assert!(matches!(
            resolve_session(&p2, &missing),
            Err(ContractError::MissingSession)
        ));
    }

    #[test]
    fn authenticate_allows_fresh_rejects_spent() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg { admin: None },
        )
        .unwrap();

        let p = params("vote.v1.cast", Some("s1"));
        let pl = payload(None, &[b"nullifier-aaa"]);
        let auth_params = params_bin(&p);

        // first authenticate ok
        check_nullifiers_fresh(
            deps.as_ref(),
            &p,
            "s1",
            &pl.nullifiers,
        )
        .unwrap();

        // spend via ConfirmExecution logic
        spend_nullifiers(deps.as_mut(), &p, "s1", &pl.nullifiers).unwrap();

        // second authenticate fails
        let err = check_nullifiers_fresh(deps.as_ref(), &p, "s1", &pl.nullifiers).unwrap_err();
        assert!(matches!(err, ContractError::AlreadySpent { .. }));

        // sudo Authenticate also fails
        let req = AuthenticationRequest {
            authenticator_id: "0".into(),
            account: "terp1abc".into(),
            fee_payer: "terp1abc".into(),
            fee_granter: None,
            fee: vec![],
            msg: LocalAny {
                type_url: "/test.MsgCarrier".into(),
                value: payload_bin(&pl),
            },
            msg_index: 0,
            signature: payload_bin(&pl),
            sign_mode_tx_data: crate::auth_types::SignModeData {
                sign_mode_direct: Binary::from(b"x"),
                sign_mode_textual: String::new(),
            },
            tx_data: crate::auth_types::ExplicitTxData {
                chain_id: "test".into(),
                account_number: 0,
                sequence: 0,
                timeout_height: 0,
                msgs: vec![],
                memo: String::new(),
            },
            signature_data: crate::auth_types::SimplifiedSignatureData {
                signers: vec![],
                signatures: vec![],
            },
            simulate: false,
            authenticator_params: Some(auth_params.clone()),
        };
        let err = authenticate(deps.as_mut(), req).unwrap_err();
        assert!(matches!(err, ContractError::AlreadySpent { .. }));
    }

    #[test]
    fn track_does_not_spend() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg { admin: None },
        )
        .unwrap();

        let p = params("vote.v1", Some("s1"));
        let pl = payload(None, &[b"nf-track"]);
        let req = TrackRequest {
            authenticator_id: "0".into(),
            account: "terp1abc".into(),
            fee_payer: "terp1abc".into(),
            fee_granter: None,
            fee: vec![],
            msg: LocalAny {
                type_url: "/test.MsgCarrier".into(),
                value: payload_bin(&pl),
            },
            msg_index: 0,
            authenticator_params: Some(params_bin(&p)),
        };
        track(deps.as_mut(), req).unwrap();

        assert!(!SPENT.has(
            deps.as_ref().storage,
            spent_key("vote.v1", "s1", b"nf-track"),
        ));
    }

    #[test]
    fn confirm_execution_marks_spent_and_domain_session_isolation() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg { admin: None },
        )
        .unwrap();

        let nf = b"same-bytes";
        let pl = payload(Some("sess-a"), &[nf]);

        // domain A / sess-a
        let p_a = params("domain-a", None);
        confirm_execution(
            deps.as_mut(),
            ConfirmExecutionRequest {
                authenticator_id: "0".into(),
                account: "terp1".into(),
                fee_payer: "terp1".into(),
                fee_granter: None,
                fee: vec![],
                msg: LocalAny {
                    type_url: "/test.Carrier".into(),
                    value: payload_bin(&pl),
                },
                msg_index: 0,
                authenticator_params: Some(params_bin(&p_a)),
            },
        )
        .unwrap();

        // same nf, different session still free
        let pl_b = payload(Some("sess-b"), &[nf]);
        check_nullifiers_fresh(deps.as_ref(), &p_a, "sess-b", &pl_b.nullifiers).unwrap();

        // same nf, different domain still free
        let p_b = params("domain-b", None);
        check_nullifiers_fresh(deps.as_ref(), &p_b, "sess-a", &pl.nullifiers).unwrap();

        // original spent
        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::IsSpent {
                domain: "domain-a".into(),
                session_id: "sess-a".into(),
                nullifier: Binary::from(nf.as_slice()),
            },
        )
        .unwrap();
        let resp: IsSpentResponse = from_json(q).unwrap();
        assert!(resp.spent);
    }

    #[test]
    fn max_nullifiers_enforced() {
        let p = AuthenticatorParams {
            domain: "d".into(),
            session_id: Some("s".into()),
            require_session: true,
            max_nullifiers_per_tx: 2,
        };
        let nfs: Vec<Binary> = (0..3).map(|i| Binary::from(vec![i])).collect();
        let err = validate_nullifier_list(&p, &nfs).unwrap_err();
        assert!(matches!(
            err,
            ContractError::TooManyNullifiers { got: 3, max: 2 }
        ));
    }

    #[test]
    fn on_authenticator_added_validates_params() {
        let mut deps = mock_dependencies();
        let bad = AuthenticatorParams {
            domain: "".into(),
            ..params("x", None)
        };
        let err = on_authenticator_added(
            deps.as_mut(),
            OnAuthenticatorAddedRequest {
                account: "terp1".into(),
                authenticator_params: Some(params_bin(&bad)),
                authenticator_id: "0".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidParams(_)));
    }

    #[test]
    fn full_sudo_roundtrip_auth_then_confirm() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg {
                admin: Some(human("terp1admin")),
            },
        )
        .unwrap();

        let p = params("vote.v1.delegate", Some("round-9"));
        let pl = payload(None, &[b"nf-1", b"nf-2"]);
        let auth_params = params_bin(&p);
        let pl_bin = payload_bin(&pl);

        // Authenticate fresh
        sudo(
            deps.as_mut(),
            mock_env(),
            AuthenticatorSudoMsg::Authenticate(AuthenticationRequest {
                authenticator_id: "1".into(),
                account: "terp1user".into(),
                fee_payer: "terp1user".into(),
                fee_granter: None,
                fee: vec![],
                msg: LocalAny {
                    type_url: "/test.MsgCarrier".into(),
                    value: pl_bin.clone(),
                },
                msg_index: 0,
                signature: pl_bin.clone(),
                sign_mode_tx_data: crate::auth_types::SignModeData {
                    sign_mode_direct: Binary::from(b"tx"),
                    sign_mode_textual: String::new(),
                },
                tx_data: crate::auth_types::ExplicitTxData {
                    chain_id: "terp-1".into(),
                    account_number: 1,
                    sequence: 2,
                    timeout_height: 0,
                    msgs: vec![],
                    memo: String::new(),
                },
                signature_data: crate::auth_types::SimplifiedSignatureData {
                    signers: vec!["terp1user".into()],
                    signatures: vec![pl_bin.clone()],
                },
                simulate: false,
                authenticator_params: Some(auth_params.clone()),
            }),
        )
        .unwrap();

        // Track no-op
        sudo(
            deps.as_mut(),
            mock_env(),
            AuthenticatorSudoMsg::Track(TrackRequest {
                authenticator_id: "1".into(),
                account: "terp1user".into(),
                fee_payer: "terp1user".into(),
                fee_granter: None,
                fee: vec![],
                msg: LocalAny {
                    type_url: "/test.MsgCarrier".into(),
                    value: pl_bin.clone(),
                },
                msg_index: 0,
                authenticator_params: Some(auth_params.clone()),
            }),
        )
        .unwrap();
        assert!(!SPENT.has(
            deps.as_ref().storage,
            spent_key("vote.v1.delegate", "round-9", b"nf-1"),
        ));

        // ConfirmExecution spends
        sudo(
            deps.as_mut(),
            mock_env(),
            AuthenticatorSudoMsg::ConfirmExecution(ConfirmExecutionRequest {
                authenticator_id: "1".into(),
                account: "terp1user".into(),
                fee_payer: "terp1user".into(),
                fee_granter: None,
                fee: vec![],
                msg: LocalAny {
                    type_url: "/test.MsgCarrier".into(),
                    value: pl_bin,
                },
                msg_index: 0,
                authenticator_params: Some(auth_params),
            }),
        )
        .unwrap();

        assert!(SPENT.has(
            deps.as_ref().storage,
            spent_key("vote.v1.delegate", "round-9", b"nf-1"),
        ));
        assert!(SPENT.has(
            deps.as_ref().storage,
            spent_key("vote.v1.delegate", "round-9", b"nf-2"),
        ));
    }
}
