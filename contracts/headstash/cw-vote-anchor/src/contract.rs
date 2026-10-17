#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};

use crate::error::ContractError;
use crate::msg::{
    AnchorResponse, ConfigResponse, ExecuteMsg, InstantiateMsg, LatestResponse, QueryMsg,
};
use crate::state::{Config, SessionMeta, ANCHORS, CONFIG, ROOT_LEN, SESSION_META};

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
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::SetAnchor {
            session_id,
            height,
            root,
        } => set_anchor(deps, info, session_id, height, root),
        ExecuteMsg::UpdateAdmin { admin } => update_admin(deps, info, admin),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse { admin: cfg.admin })
        }
        QueryMsg::GetAnchorAtHeight {
            session_id,
            height,
        } => {
            let root = ANCHORS
                .may_load(deps.storage, (&session_id, height))?
                .map(|b| b);
            to_json_binary(&AnchorResponse {
                session_id,
                height,
                root,
            })
        }
        QueryMsg::GetLatest { session_id } => {
            let meta = SESSION_META.may_load(deps.storage, &session_id)?;
            to_json_binary(&LatestResponse {
                session_id,
                latest_height: meta.as_ref().map(|m| m.latest_height),
                latest_root: meta.map(|m| m.latest_root),
            })
        }
    }
}

fn ensure_admin(deps: Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    match cfg.admin {
        Some(ref a) if a == info.sender.as_str() => Ok(()),
        Some(_) => Err(ContractError::Unauthorized),
        None => Ok(()),
    }
}

pub fn set_anchor(
    deps: DepsMut,
    info: MessageInfo,
    session_id: String,
    height: u64,
    root: Binary,
) -> Result<Response, ContractError> {
    ensure_admin(deps.as_ref(), &info)?;
    if session_id.is_empty() {
        return Err(ContractError::EmptySession);
    }
    if root.len() != ROOT_LEN {
        return Err(ContractError::BadRootLen { got: root.len() });
    }
    if ANCHORS.has(deps.storage, (&session_id, height)) {
        return Err(ContractError::AnchorExists {
            session_id,
            height,
        });
    }
    ANCHORS.save(deps.storage, (&session_id, height), &root)?;

    // Update latest if this height is greater or first.
    let update = match SESSION_META.may_load(deps.storage, &session_id)? {
        Some(m) if height < m.latest_height => false,
        _ => true,
    };
    if update {
        SESSION_META.save(
            deps.storage,
            &session_id,
            &SessionMeta {
                latest_height: height,
                latest_root: root.clone(),
            },
        )?;
    }

    Ok(Response::new()
        .add_attribute("action", "set_anchor")
        .add_attribute("session_id", session_id)
        .add_attribute("height", height.to_string()))
}

fn update_admin(
    deps: DepsMut,
    info: MessageInfo,
    admin: Option<String>,
) -> Result<Response, ContractError> {
    ensure_admin(deps.as_ref(), &info)?;
    let admin = match admin {
        Some(a) => Some(deps.api.addr_validate(&a)?.to_string()),
        None => None,
    };
    CONFIG.save(deps.storage, &Config { admin })?;
    Ok(Response::new().add_attribute("action", "update_admin"))
}

/// Pure helper: cast binding rule (fail-closed if missing or mismatch).
pub fn anchors_match(stored: Option<&[u8]>, pi_root: &[u8]) -> bool {
    match stored {
        Some(r) if r.len() == ROOT_LEN && r == pi_root => true,
        _ => false,
    }
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

    fn root32(b: u8) -> Binary {
        Binary::from(vec![b; 32])
    }

    #[test]
    fn set_and_get_anchor() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("c", &[]),
            InstantiateMsg {
                admin: Some(human("terp1admin")),
            },
        )
        .unwrap();

        let r = root32(0xab);
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1admin", &[]),
            ExecuteMsg::SetAnchor {
                session_id: "s1".into(),
                height: 100,
                root: r.clone(),
            },
        )
        .unwrap();

        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::GetAnchorAtHeight {
                session_id: "s1".into(),
                height: 100,
            },
        )
        .unwrap();
        let resp: AnchorResponse = from_json(q).unwrap();
        assert_eq!(resp.root.unwrap(), r);

        // missing height
        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::GetAnchorAtHeight {
                session_id: "s1".into(),
                height: 99,
            },
        )
        .unwrap();
        let resp: AnchorResponse = from_json(q).unwrap();
        assert!(resp.root.is_none());

        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::GetLatest {
                session_id: "s1".into(),
            },
        )
        .unwrap();
        let latest: LatestResponse = from_json(q).unwrap();
        assert_eq!(latest.latest_height, Some(100));
    }

    #[test]
    fn rejects_bad_root_len_and_dup() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("c", &[]),
            InstantiateMsg { admin: None },
        )
        .unwrap();

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("anyone", &[]),
            ExecuteMsg::SetAnchor {
                session_id: "s".into(),
                height: 1,
                root: Binary::from(vec![1, 2, 3]),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::BadRootLen { got: 3 }));

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("anyone", &[]),
            ExecuteMsg::SetAnchor {
                session_id: "s".into(),
                height: 1,
                root: root32(1),
            },
        )
        .unwrap();

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("anyone", &[]),
            ExecuteMsg::SetAnchor {
                session_id: "s".into(),
                height: 1,
                root: root32(2),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::AnchorExists { .. }));
    }

    #[test]
    fn cast_binding_helper() {
        let r = vec![7u8; 32];
        assert!(anchors_match(Some(&r), &r));
        assert!(!anchors_match(None, &r));
        assert!(!anchors_match(Some(&r), &vec![8u8; 32]));
    }
}
