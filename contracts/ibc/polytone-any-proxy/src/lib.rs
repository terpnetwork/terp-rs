//! Voice-side **Any** dispatcher for polytone.
//!
//! Stock polytone note/proxy speak `JSON-CosmosMsg` without `CosmosMsg::Any`.
//! This contract is invoked via `WasmMsg::Execute` from the ICA proxy, then
//! emits `CosmosMsg::Any` so wasmd runs protobuf `MsgExecuteContract` (circuits).

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    to_json_binary, AnyMsg, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;

const CONTRACT_NAME: &str = "crates.io:polytone-any-proxy";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// wasmd execute (ADR-020 / CosmWasm 2 Any).
pub const MSG_EXECUTE_CONTRACT_TYPE_URL: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";

pub const INSTANTIATOR: Item<cosmwasm_std::Addr> = Item::new("inst");
/// Lab: ICA proxy is not known until first polytone execute. When true, any sender may Dispatch.
pub const UNRESTRICTED: Item<bool> = Item::new("open");

#[cw_serde]
pub struct InstantiateMsg {
    /// If true, skip instantiator check (ict-rs / lab). Production: false, then `SetInstantiator` to the ICA.
    pub unrestricted: Option<bool>,
}

#[cw_serde]
pub struct AnyPacket {
    pub type_url: String,
    pub value: Binary,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Dispatch protobuf Anys. Instantiator only unless `unrestricted`.
    Dispatch { msgs: Vec<AnyPacket> },
    /// Hand the gate to the polytone ICA address (after `RemoteAddress` query).
    SetInstantiator { addr: String },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(cosmwasm_std::Addr)]
    Instantiator {},
}

#[derive(thiserror::Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] cosmwasm_std::StdError),
    #[error("not instantiator")]
    NotInstantiator,
    #[error("empty type_url")]
    EmptyTypeUrl,
}

/// Protobuf3 length-delimited field (wire type 2).
fn proto_ld(field: u32, data: &[u8]) -> Vec<u8> {
    let tag = (field << 3) | 2;
    let mut out = Vec::with_capacity(2 + data.len() + 8);
    proto_varint(&mut out, tag as u64);
    proto_varint(&mut out, data.len() as u64);
    out.extend_from_slice(data);
    out
}

fn proto_varint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push(((v as u8) & 0x7f) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// Encode `cosmwasm.wasm.v1.MsgExecuteContract` (sender, contract, msg; no funds).
pub fn encode_msg_execute_contract(sender: &str, contract: &str, msg: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(proto_ld(1, sender.as_bytes()));
    body.extend(proto_ld(2, contract.as_bytes()));
    body.extend(proto_ld(3, msg));
    body
}

pub fn wasm_execute_any(sender: &str, contract: &str, json_msg: &[u8]) -> AnyPacket {
    AnyPacket {
        type_url: MSG_EXECUTE_CONTRACT_TYPE_URL.to_string(),
        value: Binary::from(encode_msg_execute_contract(sender, contract, json_msg)),
    }
}

pub fn any_cosmos(p: AnyPacket) -> Result<CosmosMsg, ContractError> {
    if p.type_url.is_empty() {
        return Err(ContractError::EmptyTypeUrl);
    }
    Ok(CosmosMsg::Any(AnyMsg {
        type_url: p.type_url,
        value: p.value,
    }))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    INSTANTIATOR.save(deps.storage, &info.sender)?;
    UNRESTRICTED.save(deps.storage, &msg.unrestricted.unwrap_or(false))?;
    Ok(Response::new()
        .add_attribute("method", "instantiate")
        .add_attribute("owner", info.sender)
        .add_attribute(
            "unrestricted",
            msg.unrestricted.unwrap_or(false).to_string(),
        ))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Dispatch { msgs } => {
            let open = UNRESTRICTED.load(deps.storage).unwrap_or(false);
            if !open && info.sender != INSTANTIATOR.load(deps.storage)? {
                return Err(ContractError::NotInstantiator);
            }
            let mut resp = Response::new()
                .add_attribute("method", "dispatch_any")
                .add_attribute("n", msgs.len().to_string());
            for p in msgs {
                resp = resp.add_message(any_cosmos(p)?);
            }
            Ok(resp)
        }
        ExecuteMsg::SetInstantiator { addr } => {
            if info.sender != INSTANTIATOR.load(deps.storage)? {
                return Err(ContractError::NotInstantiator);
            }
            let a = deps.api.addr_validate(&addr)?;
            INSTANTIATOR.save(deps.storage, &a)?;
            UNRESTRICTED.save(deps.storage, &false)?;
            Ok(Response::new()
                .add_attribute("method", "set_instantiator")
                .add_attribute("owner", a))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Instantiator {} => to_json_binary(&INSTANTIATOR.load(deps.storage)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{from_json, Addr};

    #[test]
    fn proto_execute_round_fields() {
        let bytes = encode_msg_execute_contract("terp1sender", "terp1contract", b"{\"proove\":{}}");
        assert!(bytes.starts_with(&[0x0a])); // field 1 string
        assert!(bytes.len() > 20);
    }

    #[test]
    fn any_json_matches_cosmwasm2() {
        let p = wasm_execute_any("s", "c", b"{}");
        let msg = any_cosmos(p.clone()).unwrap();
        let json = cosmwasm_std::to_json_string(&msg).unwrap();
        assert!(json.starts_with(r#"{"any":{"type_url":"/cosmwasm.wasm.v1.MsgExecuteContract""#));
        assert_eq!(p.type_url, MSG_EXECUTE_CONTRACT_TYPE_URL);
    }

    #[test]
    fn instantiator_only() {
        let mut deps = mock_dependencies();
        let owner = Addr::unchecked("owner");
        let other = Addr::unchecked("other");
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                unrestricted: None,
            },
        )
        .unwrap();

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&other, &[]),
            ExecuteMsg::Dispatch { msgs: vec![] },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::NotInstantiator));

        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::Dispatch { msgs: vec![] },
        )
        .unwrap();
    }

    #[test]
    fn dispatch_emits_any_submsg() {
        let mut deps = mock_dependencies();
        let owner = Addr::unchecked("ica");
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                unrestricted: None,
            },
        )
        .unwrap();
        let packet = wasm_execute_any("ica", "norick", br#"{"proove":{"cid":1,"forbidden":"rick","proof":"AA=="}}"#);
        let resp = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::Dispatch {
                msgs: vec![packet],
            },
        )
        .unwrap();
        assert_eq!(resp.messages.len(), 1);
        match &resp.messages[0].msg {
            CosmosMsg::Any(a) => assert_eq!(a.type_url, MSG_EXECUTE_CONTRACT_TYPE_URL),
            other => panic!("expected Any, got {other:?}"),
        }
    }

    #[test]
    fn query_instantiator() {
        let mut deps = mock_dependencies();
        let owner = Addr::unchecked("voice");
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                unrestricted: None,
            },
        )
        .unwrap();
        let bin = query(deps.as_ref(), mock_env(), QueryMsg::Instantiator {}).unwrap();
        let got: Addr = from_json(bin).unwrap();
        assert_eq!(got, owner);
    }
}
