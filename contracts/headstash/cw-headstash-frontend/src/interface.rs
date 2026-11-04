//! cw-orch interface + release helper: gzip -> chunk -> publish -> upload -> verify.
use crate::{execute, instantiate, migrate, msg::*, query};
use cosmwasm_std::{Binary, Empty, HexBinary};
use cw_orch::prelude::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg, id = "cw_headstash_frontend")]
pub struct FrontendContract;

impl<Chain: CwEnv> Uploadable for FrontendContract<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!().find_wasm_path_from_crates_label("cw_headstash_frontend").unwrap()
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_migrate(migrate))
    }
}

/// One input file for a release.
pub struct ReleaseFile {
    pub path: String,
    pub content_type: String,
    pub raw: Vec<u8>,
    /// gzip (level 9) when true; stored verbatim when false.
    pub gzip: bool,
}

/// A prepared release: the manifest plus every chunk as (file, index, bytes).
pub struct Release {
    pub manifest: Manifest,
    pub chunks: Vec<(u16, u32, Vec<u8>)>,
}

fn sha(b: &[u8]) -> HexBinary {
    HexBinary::from(Sha256::digest(b).to_vec())
}

pub fn gzip(raw: &[u8]) -> Vec<u8> {
    // mtime 0 + no filename: deterministic output for reproducible manifests.
    let mut e = flate2::GzBuilder::new().mtime(0).write(Vec::new(), flate2::Compression::best());
    e.write_all(raw).unwrap();
    e.finish().unwrap()
}

pub fn gunzip(gz: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut out)?;
    Ok(out)
}

pub fn build_release(version: u32, entry: &str, files: Vec<ReleaseFile>, chunk_bytes: usize, activate: bool, note: Option<String>) -> Release {
    let mut entries = vec![];
    let mut chunks = vec![];
    for (fi, f) in files.into_iter().enumerate() {
        let stored = if f.gzip { gzip(&f.raw) } else { f.raw.clone() };
        let mut hashes = vec![];
        for (ci, c) in stored.chunks(chunk_bytes).enumerate() {
            hashes.push(sha(c));
            chunks.push((fi as u16, ci as u32, c.to_vec()));
        }
        entries.push(FileEntry {
            path: f.path,
            content_type: f.content_type,
            encoding: if f.gzip { "gzip" } else { "identity" }.into(),
            size: f.raw.len() as u64,
            sha256: sha(&f.raw),
            stored_size: stored.len() as u64,
            chunks: hashes,
        });
    }
    Release { manifest: Manifest { version, entry: entry.into(), files: entries, note, activate }, chunks }
}

/// Re-download every file of `version` and check chunk + raw sha256. Returns raw files.
pub fn fetch_verified<Chain: CwEnv>(c: &FrontendContract<Chain>, version: Option<u32>) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let m: Manifest = c.query(&QueryMsg::Manifest { version })?;
    let mut out = vec![];
    for (fi, f) in m.files.iter().enumerate() {
        let mut stored = vec![];
        for (ci, want) in f.chunks.iter().enumerate() {
            let b: Binary = c.query(&QueryMsg::Chunk { version: m.version, file: fi as u16, index: ci as u32 })?;
            anyhow::ensure!(sha(b.as_slice()) == *want, "chunk {}/{ci} sha mismatch", f.path);
            stored.extend_from_slice(b.as_slice());
        }
        let raw = if f.encoding == "gzip" { gunzip(&stored)? } else { stored };
        anyhow::ensure!(sha(&raw) == f.sha256 && raw.len() as u64 == f.size, "{} raw sha mismatch", f.path);
        out.push((f.path.clone(), raw));
    }
    Ok(out)
}

/// Publish (if `publish`), upload missing chunks (idempotent / resumable), then verify.
pub fn upload_release<Chain: CwEnv>(c: &FrontendContract<Chain>, r: &Release, publish: bool) -> anyhow::Result<StatusResponse> {
    let v = r.manifest.version;
    if publish {
        c.execute(&ExecuteMsg::PublishManifest { manifest: r.manifest.clone() }, &[])?;
    }
    for (file, index, data) in &r.chunks {
        let have: Result<Binary, _> = c.query(&QueryMsg::Chunk { version: v, file: *file, index: *index });
        if have.is_ok() {
            continue;
        }
        c.execute(&ExecuteMsg::UploadChunk { version: v, file: *file, index: *index, data: Binary::from(data.clone()) }, &[])?;
    }
    let st: StatusResponse = c.query(&QueryMsg::Status { version: v })?;
    anyhow::ensure!(st.complete, "upload incomplete: {}/{}", st.uploaded, st.total);
    fetch_verified(c, Some(v))?;
    Ok(st)
}
