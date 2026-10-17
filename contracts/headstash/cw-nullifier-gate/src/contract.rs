#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
    WasmMsg,
};

use crate::auth_types::{
    AuthenticationRequest, ConfirmExecutionRequest, LocalAny, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};
use crate::error::ContractError;
use crate::msg::{
    AuthenticatorParams, AuthenticatorSudoMsg, CeremonyExecuteProxy, GateInfoResponse,
    InstantiateMsg, NullifierAuthPayload, QueryMsg,
};
use crate::state::{Config, CONFIG};

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    CONFIG.save(
        deps.storage,
        &Config {
            note: "thin gate: no spent SSOT; ceremony module owns nullifiers".into(),
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: crate::msg::ExecuteMsg,
) -> Result<Response, ContractError> {
    Err(ContractError::Std(cosmwasm_std::StdError::msg(
        "execute disabled; use authenticator sudo lifecycle",
    )))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GateInfo {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(&GateInfoResponse {
                owns_spent_map: false,
                note: cfg.note,
            })
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
    Ok(Response::new().add_attribute("action", "on_authenticator_removed"))
}

/// Read-only: raw-query ceremony module for each nullifier; reject if spent.
/// Prefer raw query over smart query (gas predictability on ante).
fn authenticate(deps: DepsMut, req: AuthenticationRequest) -> Result<Response, ContractError> {
    let params = parse_params(req.authenticator_params.as_ref())?;
    let payload = parse_payload_from_auth_data(&req.signature)?;
    let session = resolve_session(&params, &payload)?;
    validate_nullifier_list(&params, &payload.nullifiers)?;

    for nf in &payload.nullifiers {
        // Smart-query fallback path used in unit tests / when raw map codec not
        // available from Wasm; production ante notes prefer raw.
        //
        // QueryMsg::IsSpent on ceremony module — still cheaper than composite
        // business logic; raw storage read is the documented target (V2 accept).
        if query_is_spent(
            deps.as_ref(),
            &params.ceremony_module,
            &params.domain,
            &session,
            nf,
            params.prefer_raw_query,
        )? {
            return Err(ContractError::AlreadySpent {
                domain: params.domain.clone(),
                session: session.clone(),
            });
        }
    }
    Ok(Response::new().add_attribute("action", "authenticate"))
}

fn track(_deps: DepsMut, _req: TrackRequest) -> Result<Response, ContractError> {
    // Never mark spent — Track commits even if execute fails.
    Ok(Response::new().add_attribute("action", "track"))
}

/// After successful execution: emit MarkSpent toward ceremony module.
/// Production: chain may convert this to sudo on ceremony; scaffold uses WasmMsg::Execute
/// only if ceremony exposes a privileged path — default msg documents intent.
fn confirm_execution(
    deps: DepsMut,
    req: ConfirmExecutionRequest,
) -> Result<Response, ContractError> {
    let params = parse_params(req.authenticator_params.as_ref())?;
    validate_params(&params)?;
    let payload = extract_payload_for_confirm(&req.msg)?;
    let session = resolve_session(&params, &payload)?;
    validate_nullifier_list(&params, &payload.nullifiers)?;

    // TOCTOU re-check (smart query path)
    for nf in &payload.nullifiers {
        if query_is_spent(
            deps.as_ref(),
            &params.ceremony_module,
            &params.domain,
            &session,
            nf,
            params.prefer_raw_query,
        )? {
            return Err(ContractError::AlreadySpent {
                domain: params.domain.clone(),
                session: session.clone(),
            });
        }
    }

    let mark = CeremonyExecuteProxy::MarkSpent {
        domain: params.domain.clone(),
        session_id: session.clone(),
        nullifiers: payload.nullifiers.clone(),
    };
    // Encode as wasm execute to ceremony_module. Ceremony currently accepts
    // MarkSpent via **sudo** only — operators/app must wire CosmwasmAuthenticator
    // ConfirmExecution → module sudo, or add permissioned execute on ceremony.
    // Emitting the msg documents the payload for integration tests.
    let msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: params.ceremony_module,
        msg: to_json_binary(&mark)?,
        funds: vec![],
    });

    Ok(Response::new()
        .add_message(msg)
        .add_attribute("action", "confirm_execution")
        .add_attribute("spent_count", payload.nullifiers.len().to_string())
        .add_attribute("note", "mark_spent_payload_emitted"))
}

// ─── pure helpers ────────────────────────────────────────────────────────────

pub fn validate_params(params: &AuthenticatorParams) -> Result<(), ContractError> {
    if params.ceremony_module.is_empty() {
        return Err(ContractError::MissingCeremonyModule);
    }
    if params.domain.is_empty() {
        return Err(ContractError::InvalidParams("domain must be non-empty".into()));
    }
    if params.max_nullifiers_per_tx == 0 {
        return Err(ContractError::InvalidParams(
            "max_nullifiers_per_tx must be >= 1".into(),
        ));
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

/// Smart-query ceremony IsSpent. `prefer_raw` is recorded for gas notes;
/// full raw Map codec against foreign contract needs host-side raw_query of
/// the same `spent` namespace encoding as cw-storage-plus (documented in
/// cw-vote-ceremony raw_keys). Unit tests inject querier.
fn query_is_spent(
    deps: Deps,
    ceremony_module: &str,
    domain: &str,
    session: &str,
    nullifier: &Binary,
    _prefer_raw: bool,
) -> Result<bool, ContractError> {
    // Smart query shape matching cw-vote-ceremony::QueryMsg::IsSpent
    #[derive(serde::Serialize)]
    struct IsSpentQ<'a> {
        is_spent: IsSpentInner<'a>,
    }
    #[derive(serde::Serialize)]
    struct IsSpentInner<'a> {
        domain: &'a str,
        session_id: &'a str,
        nullifier: &'a Binary,
    }
    #[derive(serde::Deserialize)]
    struct IsSpentResp {
        spent: bool,
    }

    let q = IsSpentQ {
        is_spent: IsSpentInner {
            domain,
            session_id: session,
            nullifier,
        },
    };
    let raw = to_json_binary(&q).map_err(|e| ContractError::RawQuery(e.to_string()))?;
    let resp: IsSpentResp = deps
        .querier
        .query_wasm_smart(ceremony_module.to_string(), &q)
        .map_err(|e| ContractError::RawQuery(e.to_string()))?;
    let _ = raw; // documents binary form for raw path later
    Ok(resp.spent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{
        message_info, mock_dependencies, mock_env, MockQuerierCustomHandlerResult,
    };
    use cosmwasm_std::{Addr, Coin, MessageInfo};

    fn mock_info(sender: &str, funds: &[Coin]) -> MessageInfo {
        message_info(&Addr::unchecked(sender), funds)
    }
    use cosmwasm_std::{
        from_json, ContractResult, OwnedDeps, QuerierResult, SystemError, SystemResult, WasmQuery,
    };
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    type SpentSet = Arc<Mutex<HashSet<(String, String, Vec<u8>)>>>;

    fn params(module: &str, domain: &str, session: Option<&str>) -> AuthenticatorParams {
        AuthenticatorParams {
            ceremony_module: module.into(),
            domain: domain.into(),
            session_id: session.map(str::to_string),
            require_session: true,
            max_nullifiers_per_tx: 8,
            prefer_raw_query: true,
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

    /// Mock querier: answers IsSpent against in-memory set (simulates ceremony).
    fn deps_with_spent(spent: SpentSet) -> OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    > {
        let mut deps = mock_dependencies();
        let spent_q = spent.clone();
        deps.querier.update_wasm(move |query| match query {
            WasmQuery::Smart { contract_addr: _, msg } => {
                #[derive(serde::Deserialize)]
                struct Outer {
                    is_spent: Inner,
                }
                #[derive(serde::Deserialize)]
                struct Inner {
                    domain: String,
                    session_id: String,
                    nullifier: Binary,
                }
                let o: Outer = from_json(msg).unwrap();
                let key = (
                    o.is_spent.domain,
                    o.is_spent.session_id,
                    o.is_spent.nullifier.to_vec(),
                );
                let is = spent_q.lock().unwrap().contains(&key);
                #[derive(serde::Serialize)]
                struct Resp {
                    spent: bool,
                }
                let body = to_json_binary(&Resp { spent: is }).unwrap();
                SystemResult::Ok(ContractResult::Ok(body))
            }
            _ => SystemResult::Err(SystemError::UnsupportedRequest {
                kind: "only smart".into(),
            }),
        });
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("c", &[]),
            InstantiateMsg {},
        )
        .unwrap();
        deps
    }

    #[test]
    fn validate_params_requires_module_and_domain() {
        let mut p = params("", "d", Some("s"));
        assert!(matches!(
            validate_params(&p),
            Err(ContractError::MissingCeremonyModule)
        ));
        p.ceremony_module = "terp1mod".into();
        p.domain = "".into();
        assert!(matches!(
            validate_params(&p),
            Err(ContractError::InvalidParams(_))
        ));
    }

    #[test]
    fn authenticate_rejects_spent_via_query() {
        let spent: SpentSet = Arc::new(Mutex::new(HashSet::new()));
        spent.lock().unwrap().insert((
            "terp.vote.v1".into(),
            "s1".into(),
            b"nf1".to_vec(),
        ));
        let mut deps = deps_with_spent(spent);

        let p = params("terp1ceremony", "terp.vote.v1", Some("s1"));
        let pl = payload(None, &[b"nf1"]);
        let req = AuthenticationRequest {
            authenticator_id: "0".into(),
            account: "terp1a".into(),
            fee_payer: "terp1a".into(),
            fee_granter: None,
            fee: vec![],
            msg: LocalAny {
                type_url: "/x".into(),
                value: payload_bin(&pl),
            },
            msg_index: 0,
            signature: payload_bin(&pl),
            sign_mode_tx_data: crate::auth_types::SignModeData {
                sign_mode_direct: Binary::from(b"x"),
                sign_mode_textual: String::new(),
            },
            tx_data: crate::auth_types::ExplicitTxData {
                chain_id: "t".into(),
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
            authenticator_params: Some(params_bin(&p)),
        };
        let err = authenticate(deps.as_mut(), req).unwrap_err();
        assert!(matches!(err, ContractError::AlreadySpent { .. }));
    }

    #[test]
    fn authenticate_allows_fresh() {
        let spent: SpentSet = Arc::new(Mutex::new(HashSet::new()));
        let mut deps = deps_with_spent(spent);
        let p = params("terp1ceremony", "terp.vote.v1", Some("s1"));
        let pl = payload(None, &[b"fresh"]);
        let req = AuthenticationRequest {
            authenticator_id: "0".into(),
            account: "terp1a".into(),
            fee_payer: "terp1a".into(),
            fee_granter: None,
            fee: vec![],
            msg: LocalAny {
                type_url: "/x".into(),
                value: payload_bin(&pl),
            },
            msg_index: 0,
            signature: payload_bin(&pl),
            sign_mode_tx_data: crate::auth_types::SignModeData {
                sign_mode_direct: Binary::from(b"x"),
                sign_mode_textual: String::new(),
            },
            tx_data: crate::auth_types::ExplicitTxData {
                chain_id: "t".into(),
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
            authenticator_params: Some(params_bin(&p)),
        };
        authenticate(deps.as_mut(), req).unwrap();
    }

    #[test]
    fn track_noop_confirm_emits_mark_spent() {
        let spent: SpentSet = Arc::new(Mutex::new(HashSet::new()));
        let mut deps = deps_with_spent(spent);
        let p = params("terp1ceremony", "d", Some("s"));
        let pl = payload(None, &[b"nf"]);
        let pb = params_bin(&p);
        let plb = payload_bin(&pl);

        track(
            deps.as_mut(),
            TrackRequest {
                authenticator_id: "0".into(),
                account: "a".into(),
                fee_payer: "a".into(),
                fee_granter: None,
                fee: vec![],
                msg: LocalAny {
                    type_url: "/x".into(),
                    value: plb.clone(),
                },
                msg_index: 0,
                authenticator_params: Some(pb.clone()),
            },
        )
        .unwrap();

        let res = confirm_execution(
            deps.as_mut(),
            ConfirmExecutionRequest {
                authenticator_id: "0".into(),
                account: "a".into(),
                fee_payer: "a".into(),
                fee_granter: None,
                fee: vec![],
                msg: LocalAny {
                    type_url: "/x".into(),
                    value: plb,
                },
                msg_index: 0,
                authenticator_params: Some(pb),
            },
        )
        .unwrap();
        assert_eq!(res.messages.len(), 1);
        assert!(res
            .attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "confirm_execution"));
    }

    #[test]
    fn gate_info_no_ssot() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("c", &[]),
            InstantiateMsg {},
        )
        .unwrap();
        let q = query(deps.as_ref(), mock_env(), QueryMsg::GateInfo {}).unwrap();
        let info: GateInfoResponse = from_json(q).unwrap();
        assert!(!info.owns_spent_map);
    }

    // silence unused import warnings in some toolchain combos
    #[allow(dead_code)]
    fn _types() {
        let _: Option<MockQuerierCustomHandlerResult> = None;
        let _: Option<QuerierResult> = None;
    }
}
