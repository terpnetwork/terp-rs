use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Order, Response,
    StdResult,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::error::ContractError;
use crate::msg::*;
use crate::state::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Trim, strip trailing `/`, lowercase scheme + host when `://` present.
pub fn normalize_url(url: &str) -> String {
    let s = url.trim();
    let mut s = s.to_string();
    while s.ends_with('/') && s.len() > 1 {
        s.pop();
    }
    if let Some(scheme_end) = s.find("://") {
        let scheme = s[..scheme_end].to_ascii_lowercase();
        let rest = &s[scheme_end + 3..];
        // Split host (incl. port / userinfo) from path/query/fragment.
        let split_at = rest
            .find(['/', '?', '#'])
            .unwrap_or(rest.len());
        let host = rest[..split_at].to_ascii_lowercase();
        let path = &rest[split_at..];
        format!("{scheme}://{host}{path}")
    } else {
        s
    }
}

/// Charset: `[A-Za-z0-9._-]{1,200}`, no `..` (notes `validate_id`).
pub fn validate_mint_id(id: &str) -> Result<(), ContractError> {
    if id.is_empty() || id.len() > MAX_MINT_ID_LEN {
        return Err(ContractError::InvalidMintId { id: id.to_string() });
    }
    if id.contains("..") {
        return Err(ContractError::InvalidMintId { id: id.to_string() });
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(ContractError::InvalidMintId { id: id.to_string() });
    }
    Ok(())
}

pub fn mint_id_from_url(normalized_url: &str) -> String {
    let digest = Sha256::digest(normalized_url.as_bytes());
    hex::encode(digest)
}

fn validate_url(url: &str) -> Result<String, ContractError> {
    let normalized = normalize_url(url);
    if normalized.is_empty() {
        return Err(ContractError::EmptyUrl {});
    }
    if normalized.len() > MAX_URL_LEN {
        return Err(ContractError::UrlTooLong { max: MAX_URL_LEN });
    }
    Ok(normalized)
}

fn validate_content_sha256(v: &Option<String>) -> Result<(), ContractError> {
    if let Some(h) = v {
        if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ContractError::InvalidContentSha256 {});
        }
        // Prefer lowercase storage consistency: reject mixed if not all hex is fine,
        // but we accept any case hex digits.
    }
    Ok(())
}

fn metadata_size(meta: &BTreeMap<String, String>) -> usize {
    meta.iter().map(|(k, v)| k.len() + v.len()).sum()
}

fn check_metadata(
    meta: &BTreeMap<String, String>,
    max: u32,
) -> Result<(), ContractError> {
    let got = metadata_size(meta);
    if got > max as usize {
        return Err(ContractError::MetadataTooLarge { got, max });
    }
    Ok(())
}

fn assert_admin(info: &MessageInfo, cfg: &Config) -> Result<(), ContractError> {
    if info.sender != cfg.admin {
        return Err(ContractError::Unauthorized {});
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Instantiate
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let admin = match msg.admin {
        Some(a) => deps.api.addr_validate(&a)?,
        None => info.sender.clone(),
    };
    let max_metadata_bytes = msg
        .max_metadata_bytes
        .unwrap_or(DEFAULT_MAX_METADATA_BYTES);

    CONFIG.save(
        deps.storage,
        &Config {
            admin: admin.clone(),
            max_metadata_bytes,
            allow_public_register: msg.allow_public_register,
        },
    )?;

    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("admin", admin)
        .add_attribute("max_metadata_bytes", max_metadata_bytes.to_string())
        .add_attribute(
            "allow_public_register",
            msg.allow_public_register.to_string(),
        ))
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::RegisterMint {
            mint_id,
            url,
            name,
            units,
            keyset_ids,
            nuts,
            pubkey,
            status,
            content_sha256,
            metadata,
        } => execute_register(
            deps,
            env,
            info,
            mint_id,
            url,
            name,
            units,
            keyset_ids,
            nuts,
            pubkey,
            status,
            content_sha256,
            metadata,
        ),
        ExecuteMsg::UpdateMint {
            mint_id,
            url,
            name,
            units,
            keyset_ids,
            nuts,
            pubkey,
            content_sha256,
            metadata,
        } => execute_update(
            deps,
            env,
            info,
            mint_id,
            url,
            name,
            units,
            keyset_ids,
            nuts,
            pubkey,
            content_sha256,
            metadata,
        ),
        ExecuteMsg::SetStatus { mint_id, status } => {
            execute_set_status(deps, env, info, mint_id, status)
        }
        ExecuteMsg::RemoveMint { mint_id } => execute_remove(deps, env, info, mint_id),
        ExecuteMsg::UpdateConfig {
            admin,
            max_metadata_bytes,
            allow_public_register,
        } => execute_update_config(deps, info, admin, max_metadata_bytes, allow_public_register),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_register(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mint_id: Option<String>,
    url: String,
    name: Option<String>,
    units: Vec<String>,
    keyset_ids: Vec<String>,
    nuts: BTreeMap<String, String>,
    pubkey: Option<String>,
    status: Option<MintStatus>,
    content_sha256: Option<String>,
    metadata: BTreeMap<String, String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if !cfg.allow_public_register {
        assert_admin(&info, &cfg)?;
    }

    let stored_url = validate_url(&url)?;
    validate_content_sha256(&content_sha256)?;
    check_metadata(&metadata, cfg.max_metadata_bytes)?;

    let id = match mint_id {
        Some(ref s) if !s.is_empty() => {
            validate_mint_id(s)?;
            s.clone()
        }
        _ => mint_id_from_url(&stored_url),
    };
    // Auto-derived id is always valid hex; explicit still validated above.
    validate_mint_id(&id)?;

    if MINTS.has(deps.storage, &id) {
        return Err(ContractError::MintIdExists { mint_id: id });
    }
    if let Some(existing) = URL_INDEX.may_load(deps.storage, &stored_url)? {
        return Err(ContractError::UrlExists { mint_id: existing });
    }

    let now = env.block.time.seconds();
    let status = status.unwrap_or(MintStatus::Active);
    let desc = MintDescriptor {
        mint_id: id.clone(),
        url: stored_url.clone(),
        name,
        units,
        keyset_ids,
        nuts,
        pubkey,
        status: status.clone(),
        content_sha256,
        registered_at: now,
        updated_at: now,
        registrar: info.sender.clone(),
        metadata,
    };

    MINTS.save(deps.storage, &id, &desc)?;
    URL_INDEX.save(deps.storage, &stored_url, &id)?;

    Ok(Response::new()
        .add_attribute("action", "register")
        .add_attribute("mint_id", id)
        .add_attribute("url", stored_url)
        .add_attribute("status", status.as_str())
        .add_attribute("registrar", info.sender))
}

#[allow(clippy::too_many_arguments)]
fn execute_update(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mint_id: String,
    url: Option<String>,
    name: Option<Option<String>>,
    units: Option<Vec<String>>,
    keyset_ids: Option<Vec<String>>,
    nuts: Option<BTreeMap<String, String>>,
    pubkey: Option<Option<String>>,
    content_sha256: Option<Option<String>>,
    metadata: Option<BTreeMap<String, String>>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut desc = MINTS
        .may_load(deps.storage, &mint_id)?
        .ok_or_else(|| ContractError::MintNotFound {
            mint_id: mint_id.clone(),
        })?;

    if info.sender != cfg.admin && info.sender != desc.registrar {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(new_url) = url {
        let normalized = validate_url(&new_url)?;
        if normalized != desc.url {
            if let Some(existing) = URL_INDEX.may_load(deps.storage, &normalized)? {
                if existing != mint_id {
                    return Err(ContractError::UrlExists { mint_id: existing });
                }
            }
            URL_INDEX.remove(deps.storage, &desc.url);
            URL_INDEX.save(deps.storage, &normalized, &mint_id)?;
            desc.url = normalized;
        }
    }
    if let Some(n) = name {
        desc.name = n;
    }
    if let Some(u) = units {
        desc.units = u;
    }
    if let Some(k) = keyset_ids {
        desc.keyset_ids = k;
    }
    if let Some(n) = nuts {
        desc.nuts = n;
    }
    if let Some(p) = pubkey {
        desc.pubkey = p;
    }
    if let Some(c) = content_sha256 {
        validate_content_sha256(&c)?;
        desc.content_sha256 = c;
    }
    if let Some(m) = metadata {
        check_metadata(&m, cfg.max_metadata_bytes)?;
        desc.metadata = m;
    }

    desc.updated_at = env.block.time.seconds();
    let status_str = desc.status.as_str().to_string();
    let url_out = desc.url.clone();
    MINTS.save(deps.storage, &mint_id, &desc)?;

    Ok(Response::new()
        .add_attribute("action", "update")
        .add_attribute("mint_id", mint_id)
        .add_attribute("url", url_out)
        .add_attribute("status", status_str))
}

fn execute_set_status(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mint_id: String,
    status: MintStatus,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    assert_admin(&info, &cfg)?;

    let mut desc = MINTS
        .may_load(deps.storage, &mint_id)?
        .ok_or_else(|| ContractError::MintNotFound {
            mint_id: mint_id.clone(),
        })?;

    desc.status = status.clone();
    desc.updated_at = env.block.time.seconds();
    let url = desc.url.clone();
    MINTS.save(deps.storage, &mint_id, &desc)?;

    Ok(Response::new()
        .add_attribute("action", "set_status")
        .add_attribute("mint_id", mint_id)
        .add_attribute("url", url)
        .add_attribute("status", status.as_str()))
}

/// Soft-revoke: set status to `revoked`, keep row + url index for audit.
fn execute_remove(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mint_id: String,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    assert_admin(&info, &cfg)?;

    let mut desc = MINTS
        .may_load(deps.storage, &mint_id)?
        .ok_or_else(|| ContractError::MintNotFound {
            mint_id: mint_id.clone(),
        })?;

    desc.status = MintStatus::Revoked;
    desc.updated_at = env.block.time.seconds();
    let url = desc.url.clone();
    MINTS.save(deps.storage, &mint_id, &desc)?;

    Ok(Response::new()
        .add_attribute("action", "remove")
        .add_attribute("mint_id", mint_id)
        .add_attribute("url", url)
        .add_attribute("status", MintStatus::Revoked.as_str()))
}

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    admin: Option<String>,
    max_metadata_bytes: Option<u32>,
    allow_public_register: Option<bool>,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    assert_admin(&info, &cfg)?;

    if let Some(a) = admin {
        cfg.admin = deps.api.addr_validate(&a)?;
    }
    if let Some(m) = max_metadata_bytes {
        cfg.max_metadata_bytes = m;
    }
    if let Some(p) = allow_public_register {
        cfg.allow_public_register = p;
    }
    CONFIG.save(deps.storage, &cfg)?;

    Ok(Response::new()
        .add_attribute("action", "update_config")
        .add_attribute("admin", cfg.admin)
        .add_attribute("max_metadata_bytes", cfg.max_metadata_bytes.to_string())
        .add_attribute(
            "allow_public_register",
            cfg.allow_public_register.to_string(),
        ))
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse::from(cfg))
        }
        QueryMsg::Mint { mint_id } => {
            let m = MINTS.may_load(deps.storage, &mint_id)?;
            to_json_binary(&m)
        }
        QueryMsg::MintByUrl { url } => {
            let normalized = normalize_url(&url);
            let mint_id = URL_INDEX.may_load(deps.storage, &normalized)?;
            let m = match mint_id {
                Some(id) => MINTS.may_load(deps.storage, &id)?,
                None => None,
            };
            to_json_binary(&m)
        }
        QueryMsg::ListMints {
            start_after,
            limit,
            status_filter,
        } => to_json_binary(&query_list_mints(deps, start_after, limit, status_filter)?),
        QueryMsg::IsRegistered { mint_id, url } => {
            to_json_binary(&query_is_registered(deps, mint_id, url)?)
        }
    }
}

fn query_list_mints(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
    status_filter: Option<MintStatus>,
) -> StdResult<ListMintsResponse> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    // Default filter: active only.
    let want = status_filter.unwrap_or(MintStatus::Active);

    let start = start_after.as_deref().map(cw_storage_plus::Bound::exclusive);
    let mints: Vec<MintDescriptor> = MINTS
        .range(deps.storage, start, None, Order::Ascending)
        .filter_map(|item| {
            let (_k, v) = item.ok()?;
            if v.status == want {
                Some(v)
            } else {
                None
            }
        })
        .take(limit)
        .collect();

    Ok(ListMintsResponse { mints })
}

fn query_is_registered(
    deps: Deps,
    mint_id: Option<String>,
    url: Option<String>,
) -> StdResult<IsRegisteredResponse> {
    match (mint_id, url) {
        (Some(id), None) => {
            let registered = MINTS.has(deps.storage, &id);
            Ok(IsRegisteredResponse {
                registered,
                mint_id: if registered { Some(id) } else { None },
            })
        }
        (None, Some(u)) => {
            let normalized = normalize_url(&u);
            match URL_INDEX.may_load(deps.storage, &normalized)? {
                Some(id) => Ok(IsRegisteredResponse {
                    registered: true,
                    mint_id: Some(id),
                }),
                None => Ok(IsRegisteredResponse {
                    registered: false,
                    mint_id: None,
                }),
            }
        }
        _ => Err(cosmwasm_std::StdError::msg(
            "IsRegistered requires exactly one of mint_id or url",
        )),
    }
}
