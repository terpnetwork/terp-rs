//! cw-headstash-frontend: content-addressed on-chain host for the Headstash claim UI.
//! Gov publishes a manifest (file + chunk sha256s); anyone uploads matching chunks.
pub mod msg;
#[cfg(feature = "interface")]
pub mod interface;

use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, MigrateInfo, Order, Response, StdError, StdResult,
    Storage,
};
use cw_storage_plus::{Bound, Item, Map};
use msg::*;
use sha2::{Digest, Sha256};

const MAX_CHUNK: Item<u32> = Item::new("max_chunk");
const CURRENT: Item<Option<u32>> = Item::new("current");
const NEXT_VERSION: Item<u32> = Item::new("next_version");
const MANIFESTS: Map<u32, Manifest> = Map::new("manifests");
const PROGRESS: Map<u32, u32> = Map::new("progress");
/// Prune cursor for a version being deleted: (next file, next index, keys removed so far).
const PRUNING: Map<u32, (u16, u32, u32)> = Map::new("pruning");
/// Fully pruned versions -> chunk keys removed (tombstone; version numbers are never reused).
const PRUNED: Map<u32, u32> = Map::new("pruned");

pub const CONTRACT_NAME: &str = "crates.io:cw-headstash-frontend";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn chunk_key(version: u32, file: u16, index: u32) -> Vec<u8> {
    let mut k = Vec::with_capacity(11);
    k.push(b'c');
    k.extend_from_slice(&version.to_be_bytes());
    k.extend_from_slice(&file.to_be_bytes());
    k.extend_from_slice(&index.to_be_bytes());
    k
}

fn err(m: impl Into<String>) -> StdError {
    StdError::msg(m.into())
}

fn total_chunks(m: &Manifest) -> u32 {
    m.files.iter().map(|f| f.chunks.len() as u32).sum()
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(deps: DepsMut, _env: Env, info: MessageInfo, msg: InstantiateMsg) -> StdResult<Response> {
    let owner = msg.owner.unwrap_or_else(|| info.sender.to_string());
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(&owner))?;
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let max = msg.max_chunk_bytes.unwrap_or(128 * 1024);
    if max == 0 || max > HARD_MAX_CHUNK {
        return Err(err(format!("max_chunk_bytes must be 1..={HARD_MAX_CHUNK}")));
    }
    MAX_CHUNK.save(deps.storage, &max)?;
    CURRENT.save(deps.storage, &None)?;
    NEXT_VERSION.save(deps.storage, &1)?;
    Ok(Response::new().add_attribute("action", "instantiate").add_attribute("owner", owner))
}

pub fn validate_manifest(m: &Manifest, next: u32, max_chunk: u32) -> StdResult<()> {
    if m.version != next {
        return Err(err(format!("version must be {next}")));
    }
    if m.files.is_empty() || m.files.len() > MAX_FILES {
        return Err(err(format!("files must be 1..={MAX_FILES}")));
    }
    let mut seen = std::collections::BTreeSet::new();
    for f in &m.files {
        let p = f.path.as_bytes();
        if p.is_empty()
            || p.len() > MAX_PATH
            || p[0] == b'/'
            || f.path.contains("..")
            || !p.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._/-".contains(c))
        {
            return Err(err(format!("bad path {:?}", f.path)));
        }
        if !seen.insert(f.path.clone()) {
            return Err(err(format!("duplicate path {}", f.path)));
        }
        if f.encoding != "gzip" && f.encoding != "identity" {
            return Err(err("encoding must be gzip|identity"));
        }
        if f.content_type.is_empty() || f.content_type.len() > 64 || !f.content_type.is_ascii() {
            return Err(err("bad content_type"));
        }
        if f.sha256.len() != 32 {
            return Err(err("sha256 must be 32 bytes"));
        }
        if f.chunks.is_empty() || f.chunks.len() > MAX_CHUNKS_PER_FILE {
            return Err(err(format!("chunks per file must be 1..={MAX_CHUNKS_PER_FILE}")));
        }
        if f.chunks.iter().any(|c| c.len() != 32) {
            return Err(err("chunk hash must be 32 bytes"));
        }
        if f.stored_size > f.chunks.len() as u64 * max_chunk as u64 || f.stored_size < f.chunks.len() as u64 {
            return Err(err("stored_size inconsistent with chunk count / max_chunk_bytes"));
        }
        if f.encoding == "identity" && f.size != f.stored_size {
            return Err(err("identity: size must equal stored_size"));
        }
    }
    if !seen.contains(&m.entry) {
        return Err(err("entry not in files"));
    }
    if m.note.as_ref().map_or(false, |n| n.len() > 512) {
        return Err(err("note too long"));
    }
    Ok(())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::PublishManifest { manifest } => {
            cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| err(e.to_string()))?;
            let next = NEXT_VERSION.load(deps.storage)?;
            validate_manifest(&manifest, next, MAX_CHUNK.load(deps.storage)?)?;
            MANIFESTS.save(deps.storage, next, &manifest)?;
            PROGRESS.save(deps.storage, next, &0)?;
            NEXT_VERSION.save(deps.storage, &(next + 1))?;
            Ok(Response::new()
                .add_attribute("action", "publish_manifest")
                .add_attribute("version", next.to_string())
                .add_attribute("chunks", total_chunks(&manifest).to_string()))
        }
        ExecuteMsg::UploadChunk { version, file, index, data } => {
            if PRUNING.has(deps.storage, version) {
                return Err(err("version is being pruned"));
            }
            let m = MANIFESTS.may_load(deps.storage, version)?.ok_or_else(|| err("unknown version"))?;
            let f = m.files.get(file as usize).ok_or_else(|| err("unknown file"))?;
            let want = f.chunks.get(index as usize).ok_or_else(|| err("unknown chunk"))?;
            if data.len() > MAX_CHUNK.load(deps.storage)? as usize || data.is_empty() {
                return Err(err("chunk size out of range"));
            }
            if Sha256::digest(data.as_slice()).as_slice() != want.as_slice() {
                return Err(err("chunk sha256 mismatch"));
            }
            let key = chunk_key(version, file, index);
            if deps.storage.get(&key).is_some() {
                return Err(err("chunk already stored"));
            }
            deps.storage.set(&key, data.as_slice());
            let done = PROGRESS.load(deps.storage, version)? + 1;
            PROGRESS.save(deps.storage, version, &done)?;
            let complete = done == total_chunks(&m);
            let mut r = Response::new()
                .add_attribute("action", "upload_chunk")
                .add_attribute("version", version.to_string())
                .add_attribute("file", file.to_string())
                .add_attribute("index", index.to_string())
                .add_attribute("complete", complete.to_string());
            // Auto-activation never moves `current` backwards (an old activate=true version
            // finishing late must not replace a newer current).
            let cur = CURRENT.load(deps.storage)?;
            if complete && m.activate && cur.map_or(true, |c| version > c) {
                CURRENT.save(deps.storage, &Some(version))?;
                r = r.add_attribute("current", version.to_string());
            }
            Ok(r)
        }
        ExecuteMsg::SetCurrent { version } => {
            cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| err(e.to_string()))?;
            if PRUNING.has(deps.storage, version) {
                return Err(err("version is being pruned"));
            }
            let s = status(deps.storage, version)?;
            if !s.complete {
                return Err(err("version incomplete"));
            }
            CURRENT.save(deps.storage, &Some(version))?;
            Ok(Response::new().add_attribute("action", "set_current").add_attribute("version", version.to_string()))
        }
        ExecuteMsg::PruneVersion { version, limit } => {
            cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| err(e.to_string()))?;
            prune(deps.storage, version, limit)
        }
        ExecuteMsg::UpdateOwnership(a) => {
            let o = cw_ownable::update_ownership(deps, &env.block, &info.sender, a).map_err(|e| err(e.to_string()))?;
            Ok(Response::new().add_attributes(o.into_attributes()))
        }
    }
}

fn prune(storage: &mut dyn Storage, version: u32, limit: Option<u32>) -> StdResult<Response> {
    let current = CURRENT.load(storage)?;
    if current == Some(version) {
        return Err(err("refusing to prune the current version"));
    }
    if PRUNED.has(storage, version) {
        return Err(err("version already pruned"));
    }
    let m = MANIFESTS.may_load(storage, version)?.ok_or_else(|| err("unknown version"))?;
    let cursor = PRUNING.may_load(storage, version)?;
    if cursor.is_none() && m.activate && version > current.unwrap_or(0) {
        // Still a candidate to become current by auto-activation: refuse until a newer
        // version is current (then it can never activate, see UploadChunk).
        let st = status(storage, version)?;
        if !st.complete {
            return Err(err("refusing to prune a pending activation target; make a newer version current first"));
        }
    }
    let limit = limit.unwrap_or(DEFAULT_PRUNE_LIMIT).clamp(1, MAX_PRUNE_LIMIT);
    let (mut file, mut index, mut removed) = cursor.unwrap_or((0, 0, 0));
    let mut visited = 0u32;
    while (file as usize) < m.files.len() && visited < limit {
        let n = m.files[file as usize].chunks.len() as u32;
        if index >= n {
            file += 1;
            index = 0;
            continue;
        }
        let k = chunk_key(version, file, index);
        if storage.get(&k).is_some() {
            storage.remove(&k);
            removed += 1;
        }
        visited += 1;
        index += 1;
    }
    while (file as usize) < m.files.len() && index >= m.files[file as usize].chunks.len() as u32 {
        file += 1;
        index = 0;
    }
    let done = file as usize >= m.files.len();
    let r = Response::new()
        .add_attribute("action", "prune_version")
        .add_attribute("version", version.to_string())
        .add_attribute("visited", visited.to_string())
        .add_attribute("removed", removed.to_string())
        .add_attribute("done", done.to_string());
    if done {
        MANIFESTS.remove(storage, version);
        PROGRESS.remove(storage, version);
        PRUNING.remove(storage, version);
        PRUNED.save(storage, version, &removed)?;
    } else {
        PRUNING.save(storage, version, &(file, index, removed))?;
    }
    Ok(r)
}

fn prune_status(storage: &dyn Storage, version: u32) -> StdResult<PruneStatusResponse> {
    let blank = |state: &str, removed| PruneStatusResponse { version, state: state.into(), next_file: 0, next_index: 0, removed, remaining: 0 };
    if let Some(removed) = PRUNED.may_load(storage, version)? {
        return Ok(blank("pruned", removed));
    }
    let Some(m) = MANIFESTS.may_load(storage, version)? else { return Ok(blank("unknown", 0)) };
    let total = total_chunks(&m);
    match PRUNING.may_load(storage, version)? {
        None => Ok(PruneStatusResponse { remaining: total, ..blank("live", 0) }),
        Some((file, index, removed)) => {
            let before: u32 = m.files.iter().take(file as usize).map(|f| f.chunks.len() as u32).sum::<u32>() + index;
            Ok(PruneStatusResponse { version, state: "pruning".into(), next_file: file, next_index: index, removed, remaining: total - before })
        }
    }
}

fn parse_ver(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
    let r = (it.next()??, it.next()??, it.next()??);
    if it.next().is_some() { None } else { Some(r) }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg, _info: MigrateInfo) -> StdResult<Response> {
    let from = match cw2::CONTRACT.may_load(deps.storage)? {
        Some(v) => {
            if v.contract != CONTRACT_NAME {
                return Err(err(format!("cannot migrate from contract {:?}", v.contract)));
            }
            let (old, new) = (parse_ver(&v.version), parse_ver(CONTRACT_VERSION));
            match (old, new) {
                (Some(o), Some(n)) if o <= n => {}
                _ => return Err(err(format!("cannot migrate from version {} to {}", v.version, CONTRACT_VERSION))),
            }
            v.version
        }
        None => {
            // v1 (code 106) never wrote cw2: accept only a state that looks like this contract.
            if !MAX_CHUNK.exists(deps.storage) || !NEXT_VERSION.exists(deps.storage) || !CURRENT.exists(deps.storage) {
                return Err(err("no cw2 version and not a cw-headstash-frontend v1 state"));
            }
            "v1-no-cw2".to_string()
        }
    };
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "migrate").add_attribute("from", from).add_attribute("to", CONTRACT_VERSION))
}

fn status(storage: &dyn Storage, version: u32) -> StdResult<StatusResponse> {
    let m = MANIFESTS.may_load(storage, version)?.ok_or_else(|| err("unknown version"))?;
    let total = total_chunks(&m);
    let uploaded = PROGRESS.load(storage, version)?;
    Ok(StatusResponse { version, uploaded, total, complete: uploaded == total })
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&ConfigResponse {
            max_chunk_bytes: MAX_CHUNK.load(deps.storage)?,
            current: CURRENT.load(deps.storage)?,
            next_version: NEXT_VERSION.load(deps.storage)?,
        }),
        QueryMsg::Manifest { version } => {
            let v = match version {
                Some(v) => v,
                None => CURRENT.load(deps.storage)?.ok_or_else(|| err("no current version"))?,
            };
            to_json_binary(&MANIFESTS.may_load(deps.storage, v)?.ok_or_else(|| err("unknown version"))?)
        }
        QueryMsg::Chunk { version, file, index } => {
            let b = deps.storage.get(&chunk_key(version, file, index)).ok_or_else(|| err("chunk not stored"))?;
            to_json_binary(&Binary::from(b))
        }
        QueryMsg::Status { version } => to_json_binary(&status(deps.storage, version)?),
        QueryMsg::Versions { start_after, limit } => {
            let limit = limit.unwrap_or(30).min(100) as usize;
            let versions = MANIFESTS
                .keys(deps.storage, start_after.map(Bound::exclusive), None, Order::Ascending)
                .take(limit)
                .map(|v| status(deps.storage, v?))
                .collect::<StdResult<_>>()?;
            to_json_binary(&VersionsResponse { versions })
        }
        QueryMsg::PruneStatus { version } => to_json_binary(&prune_status(deps.storage, version)?),
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
    }
}

#[cfg(test)]
mod tests;
