//! silo-cap-root: the smallest contract that holds the silo-auth capability
//! Merkle root on chain.
//!
//! * instantiate {admin?, merkle_root, metadata?}  (admin defaults to the sender)
//! * execute UpdateRoot{root} / UpdateAdmin{admin}   (admin only)
//! * query Root{} / Config{}
//!
//! The root is stored as a JSON string of 64 lowercase hex chars under the raw
//! storage key `merkle_root` (cw-storage-plus `Item`), so the x/wasm state key
//! is `0x03 || contract_addr || "merkle_root"`, exactly what dregg silo-auth's
//! `root_storage_key` proves with ICS-23 against a light-client app hash.
//! No proof verification, claims, or funds handling happens on chain.
use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{
    Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, entry_point, to_json_binary,
};
use cw_storage_plus::Item;

pub const CONTRACT_NAME: &str = "crates.io:silo-cap-root";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Max metadata length (bytes): a URI or short note, not data.
pub const MAX_METADATA: usize = 512;

/// Stable raw key; must stay `merkle_root` (dregg silo-auth proves this key).
pub const ROOT_KEY: &str = "merkle_root";
pub const MERKLE_ROOT: Item<String> = Item::new(ROOT_KEY);
pub const CONFIG: Item<Config> = Item::new("config");

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub metadata: Option<String>,
}

#[cw_serde]
pub struct InstantiateMsg {
    /// Defaults to the instantiating sender.
    pub admin: Option<String>,
    /// blake3 sorted-pair root, 64 lowercase hex.
    pub merkle_root: String,
    pub metadata: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    UpdateRoot { root: String },
    UpdateAdmin { admin: String },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(RootResponse)]
    Root {},
    #[returns(Config)]
    Config {},
}

#[cw_serde]
pub struct RootResponse {
    pub root: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ContractError {
    #[error(transparent)]
    Std(#[from] cosmwasm_std::StdError),
    #[error("unauthorized")]
    Unauthorized,
    #[error("root must be 64 lowercase hex chars")]
    InvalidRoot,
    #[error("metadata too long")]
    MetadataTooLong,
}

fn check_root(r: &str) -> Result<(), ContractError> {
    if r.len() == 64
        && r.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(ContractError::InvalidRoot)
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    check_root(&msg.merkle_root)?;
    if msg
        .metadata
        .as_ref()
        .is_some_and(|m| m.len() > MAX_METADATA)
    {
        return Err(ContractError::MetadataTooLong);
    }
    let admin = match msg.admin {
        Some(a) => deps.api.addr_validate(&a)?,
        None => info.sender,
    };
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    MERKLE_ROOT.save(deps.storage, &msg.merkle_root)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin: admin.clone(),
            metadata: msg.metadata,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("admin", admin)
        .add_attribute("merkle_root", msg.merkle_root))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.admin {
        return Err(ContractError::Unauthorized);
    }
    match msg {
        ExecuteMsg::UpdateRoot { root } => {
            check_root(&root)?;
            MERKLE_ROOT.save(deps.storage, &root)?;
            Ok(Response::new()
                .add_attribute("action", "update_root")
                .add_attribute("merkle_root", root))
        }
        ExecuteMsg::UpdateAdmin { admin } => {
            cfg.admin = deps.api.addr_validate(&admin)?;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new()
                .add_attribute("action", "update_admin")
                .add_attribute("admin", admin))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Root {} => to_json_binary(&RootResponse {
            root: MERKLE_ROOT.load(deps.storage)?,
        }),
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    const R1: &str = "9b95a225000000000000000000000000000000000000000000000000000000ab";

    #[test]
    fn instantiate_defaults_admin_and_stores_raw_key() {
        let mut d = mock_dependencies();
        let sender = d.api.addr_make("deployer");
        instantiate(
            d.as_mut(),
            mock_env(),
            message_info(&sender, &[]),
            InstantiateMsg {
                admin: None,
                merkle_root: R1.into(),
                metadata: None,
            },
        )
        .unwrap();
        assert_eq!(CONFIG.load(&d.storage).unwrap().admin, sender);
        // raw storage: key "merkle_root", value JSON string (what silo-auth proves)
        use cosmwasm_std::Storage;
        assert_eq!(
            d.storage.get(b"merkle_root").unwrap(),
            format!("\"{R1}\"").into_bytes()
        );
    }

    #[test]
    fn rejects_bad_roots_and_long_metadata() {
        let mut d = mock_dependencies();
        let s = d.api.addr_make("deployer");
        for bad in [
            "",
            "ABCD",
            &R1.to_uppercase(),
            &R1[..62],
            &format!("{R1}00"),
        ] {
            assert!(matches!(
                instantiate(
                    d.as_mut(),
                    mock_env(),
                    message_info(&s, &[]),
                    InstantiateMsg {
                        admin: None,
                        merkle_root: bad.into(),
                        metadata: None
                    }
                )
                .unwrap_err(),
                ContractError::InvalidRoot
            ));
        }
        assert!(matches!(
            instantiate(
                d.as_mut(),
                mock_env(),
                message_info(&s, &[]),
                InstantiateMsg {
                    admin: None,
                    merkle_root: R1.into(),
                    metadata: Some("x".repeat(513))
                }
            )
            .unwrap_err(),
            ContractError::MetadataTooLong
        ));
    }
}
