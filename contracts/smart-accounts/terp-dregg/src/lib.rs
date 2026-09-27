//! # terp-dregg
//!
//! Smart-account authenticator dual-use with the dregg B2 light client.
//! Gates on **claim-limited settlement head** (height/root), never Discord/caps identity.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    to_json_binary, Addr, Binary, Deps, DepsMut, Env, MessageInfo, QueryRequest, Response,
    StdResult, WasmQuery,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::{
    AuthSudoMsg, AuthenticationRequest, ConfirmExecutionRequest, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};

use cw_ics08_wasm_dregg::msg::{BoolResponse, HeightResponse, QueryMsg as LcQueryMsg};

const CONTRACT_NAME: &str = "crates.io:terp-dregg";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const CONFIG: Item<AuthConfig> = Item::new("cfg");

#[cw_serde]
pub enum AuthMode {
    QueryLc,
    EmbeddedHead,
}

#[cw_serde]
pub struct AuthConfig {
    pub mode: AuthMode,
    pub lc_contract: Option<Addr>,
    pub min_proven_height: u64,
    pub require_root_in_history: bool,
    pub claim_ceiling: String,
}

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    pub mode: AuthMode,
    pub lc_contract: Option<String>,
    pub min_proven_height: Option<u64>,
    pub require_root_in_history: Option<bool>,
}

#[cw_serde]
pub enum ExecuteMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(AuthConfig)]
    Config {},
}

#[cw_serde]
pub struct AuthEvidence {
    pub proven_root_key: String,
    pub proven_height: u64,
}

pub type SudoMsg = AuthSudoMsg;

#[derive(thiserror::Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] cosmwasm_std::StdError),
    #[error("invalid authenticator config: {0}")]
    BadConfig(String),
    #[error("authenticate rejected: {0}")]
    AuthRejected(String),
    #[error("claim_ceiling forbids principal-authorization language")]
    ClaimCeiling,
}

pub struct DreggAuthenticator;
#[derive(Clone, Debug)]
pub struct DreggAuthStructs {}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let lc = match msg.lc_contract {
        Some(s) => Some(deps.api.addr_validate(&s)?),
        None => None,
    };
    if matches!(msg.mode, AuthMode::QueryLc) && lc.is_none() {
        return Err(ContractError::BadConfig(
            "query_lc requires lc_contract".into(),
        ));
    }
    CONFIG.save(
        deps.storage,
        &AuthConfig {
            mode: msg.mode,
            lc_contract: lc,
            min_proven_height: msg.min_proven_height.unwrap_or(0),
            require_root_in_history: msg.require_root_in_history.unwrap_or(true),
            claim_ceiling: "settlement_only".into(),
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("claim_class", "settlement_only")
        .add_attribute("authenticator", "terp-dregg"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    DreggAuthenticator::process_sudo_auth(deps, env, &msg)
}

impl TerpAccountTrait for DreggAuthenticator {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = SudoMsg;
    type ContractError = ContractError;
    type AuthMethodStructs = DreggAuthStructs;
    type AuthProcessResult = Result<Response, ContractError>;

    fn extended_authenticate(
        _deps: DepsMut,
        _auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        Ok(Response::default())
    }

    fn process_sudo_auth(deps: DepsMut, env: Env, req: &Self::SudoMsg) -> Self::AuthProcessResult {
        match req {
            AuthSudoMsg::OnAuthAdded(r) => Self::on_auth_added(deps, env, r),
            AuthSudoMsg::OnAuthRemoved(r) => Self::on_auth_removed(deps, env, r),
            AuthSudoMsg::Authenticate(r) => Self::on_auth_request(deps, env, r),
            AuthSudoMsg::Track(r) => Self::on_auth_track(deps, env, r),
            AuthSudoMsg::ConfirmExecution(r) => Self::on_auth_confirm(deps, env, r),
        }
    }

    fn on_auth_added(
        deps: DepsMut,
        _env: Env,
        req: &OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        if let Some(params) = &req.authenticator_params {
            if let Ok(cfg) = cosmwasm_std::from_json::<AuthConfig>(params) {
                if cfg.claim_ceiling != "settlement_only" {
                    return Err(ContractError::ClaimCeiling);
                }
                CONFIG.save(deps.storage, &cfg)?;
            }
        }
        Ok(Response::new().add_attribute("action", "on_auth_added"))
    }

    fn on_auth_removed(
        _deps: DepsMut,
        _env: Env,
        _req: &OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "on_auth_removed"))
    }

    fn on_auth_request(
        deps: DepsMut,
        _env: Env,
        req: &Box<AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let cfg = CONFIG.load(deps.storage)?;
        if cfg.claim_ceiling != "settlement_only" {
            return Err(ContractError::ClaimCeiling);
        }

        let evidence = parse_evidence(req)?;
        if evidence.proven_height < cfg.min_proven_height {
            return Err(ContractError::AuthRejected(format!(
                "height {} < min {}",
                evidence.proven_height, cfg.min_proven_height
            )));
        }

        match cfg.mode {
            AuthMode::EmbeddedHead => {}
            AuthMode::QueryLc => {
                let lc = cfg
                    .lc_contract
                    .as_ref()
                    .ok_or_else(|| ContractError::BadConfig("missing lc".into()))?;
                let height: HeightResponse =
                    deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                        contract_addr: lc.to_string(),
                        msg: to_json_binary(&LcQueryMsg::ProvenHeight {})?,
                    }))?;
                if height.height < cfg.min_proven_height {
                    return Err(ContractError::AuthRejected(format!(
                        "lc height {} < min {}",
                        height.height, cfg.min_proven_height
                    )));
                }
                if evidence.proven_height > height.height {
                    return Err(ContractError::AuthRejected(
                        "evidence height above LC tip".into(),
                    ));
                }
                if cfg.require_root_in_history {
                    let is: BoolResponse =
                        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                            contract_addr: lc.to_string(),
                            msg: to_json_binary(&LcQueryMsg::IsProvenRoot {
                                root: evidence.proven_root_key.clone(),
                            })?,
                        }))?;
                    if !is.yes {
                        return Err(ContractError::AuthRejected(
                            "root not in LC proven history".into(),
                        ));
                    }
                }
            }
        }

        Ok(Response::new()
            .add_attribute("action", "authenticate")
            .add_attribute("claim_class", "settlement_only")
            .add_attribute("proven_height", evidence.proven_height.to_string())
            .add_attribute("proven_root", evidence.proven_root_key))
    }

    fn on_auth_track(
        _deps: DepsMut,
        _env: Env,
        _req: &TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::default())
    }

    fn on_auth_confirm(
        _deps: DepsMut,
        _env: Env,
        _req: &ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::default())
    }

    fn on_hooks(_deps: DepsMut, _env: Env) -> Self::AuthProcessResult {
        Ok(Response::default())
    }
}

fn parse_evidence(req: &AuthenticationRequest) -> Result<AuthEvidence, ContractError> {
    cosmwasm_std::from_json::<AuthEvidence>(&req.signature).map_err(|e| {
        ContractError::AuthRejected(format!(
            "signature must be AuthEvidence JSON: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env};
    use cosmwasm_std::{Addr, MessageInfo};

    fn info(sender: &str) -> MessageInfo {
        MessageInfo {
            sender: Addr::unchecked(sender),
            funds: vec![],
        }
    }

    fn auth_req(
        account: Addr,
        evidence: &AuthEvidence,
    ) -> Box<AuthenticationRequest> {
        Box::new(AuthenticationRequest {
            authenticator_id: "1".into(),
            account: account.clone(),
            fee_payer: account,
            fee_granter: None,
            fee: vec![],
            msg: terp_auth::Any {
                type_url: "".into(),
                value: Binary::default(),
            },
            msg_index: 0,
            signature: to_json_binary(evidence).unwrap(),
            sign_mode_tx_data: terp_auth::SignModeTxData {
                sign_mode_direct: Binary::default(),
                sign_mode_textual: None,
            },
            tx_data: terp_auth::TxData {
                chain_id: "t".into(),
                account_number: 0,
                sequence: 0,
                timeout_height: 0,
                msgs: vec![],
                memo: "".into(),
            },
            signature_data: terp_auth::SignatureData {
                signers: vec![],
                signatures: vec![],
            },
            simulate: false,
            authenticator_params: None,
        })
    }

    #[test]
    fn instantiate_embedded() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            info("creator"),
            InstantiateMsg {
                owner: None,
                mode: AuthMode::EmbeddedHead,
                lc_contract: None,
                min_proven_height: Some(2),
                require_root_in_history: Some(false),
            },
        )
        .unwrap();
        let cfg = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(cfg.min_proven_height, 2);
    }

    #[test]
    fn auth_embedded_accepts_floor() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            info("creator"),
            InstantiateMsg {
                owner: None,
                mode: AuthMode::EmbeddedHead,
                lc_contract: None,
                min_proven_height: Some(2),
                require_root_in_history: Some(false),
            },
        )
        .unwrap();

        let evidence = AuthEvidence {
            proven_root_key: "ab".into(),
            proven_height: 3,
        };
        let acct = deps.api.addr_make("acct");
        DreggAuthenticator::on_auth_request(
            deps.as_mut(),
            mock_env(),
            &auth_req(acct, &evidence),
        )
        .unwrap();
    }

    #[test]
    fn auth_embedded_rejects_below_floor() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            info("creator"),
            InstantiateMsg {
                owner: None,
                mode: AuthMode::EmbeddedHead,
                lc_contract: None,
                min_proven_height: Some(10),
                require_root_in_history: Some(false),
            },
        )
        .unwrap();
        let evidence = AuthEvidence {
            proven_root_key: "ab".into(),
            proven_height: 3,
        };
        let acct = deps.api.addr_make("acct");
        let err = DreggAuthenticator::on_auth_request(
            deps.as_mut(),
            mock_env(),
            &auth_req(acct, &evidence),
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::AuthRejected(_)));
    }
}
