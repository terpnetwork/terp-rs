//! cw-headstash-frontend: content-addressed on-chain host for the Headstash claim UI.
//! Gov publishes a manifest (file + chunk sha256s); anyone uploads matching chunks.
pub mod msg;
#[cfg(feature = "interface")]
pub mod interface;

use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Order, Response, StdError, StdResult,
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
            if complete && m.activate {
                CURRENT.save(deps.storage, &Some(version))?;
                r = r.add_attribute("current", version.to_string());
            }
            Ok(r)
        }
        ExecuteMsg::SetCurrent { version } => {
            cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| err(e.to_string()))?;
            let s = status(deps.storage, version)?;
            if !s.complete {
                return Err(err("version incomplete"));
            }
            CURRENT.save(deps.storage, &Some(version))?;
            Ok(Response::new().add_attribute("action", "set_current").add_attribute("version", version.to_string()))
        }
        ExecuteMsg::UpdateOwnership(a) => {
            let o = cw_ownable::update_ownership(deps, &env.block, &info.sender, a).map_err(|e| err(e.to_string()))?;
            Ok(Response::new().add_attributes(o.into_attributes()))
        }
    }
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
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
    }
}

#[cfg(test)]
mod tests;
