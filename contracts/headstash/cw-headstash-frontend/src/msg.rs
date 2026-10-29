use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, HexBinary};
use cw_ownable::{cw_ownable_execute, cw_ownable_query};

pub const MAX_FILES: usize = 16;
pub const MAX_CHUNKS_PER_FILE: usize = 512;
/// wasmvm refuses storage values over 128 KiB ("Value too big ... limit is 131072"), measured on the local chain.
pub const HARD_MAX_CHUNK: u32 = 128 * 1024;
pub const MAX_PATH: usize = 64;

#[cw_serde]
pub struct InstantiateMsg {
    /// Owner (x/gov on mainnet). Defaults to the sender.
    pub owner: Option<String>,
    /// Per-chunk byte cap (<= 128 KiB, the wasmvm storage value limit). Default 128 KiB.
    pub max_chunk_bytes: Option<u32>,
}

#[cw_serde]
pub struct FileEntry {
    pub path: String,
    pub content_type: String,
    /// "gzip" or "identity": how stored bytes decode to the raw file.
    pub encoding: String,
    /// Raw (decoded) size and sha256.
    pub size: u64,
    pub sha256: HexBinary,
    /// Stored (encoded) size = sum of chunk lengths.
    pub stored_size: u64,
    /// sha256 of each stored chunk, in order.
    pub chunks: Vec<HexBinary>,
}

#[cw_serde]
pub struct Manifest {
    pub version: u32,
    /// Path of the entry document inside `files`.
    pub entry: String,
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub note: Option<String>,
    /// Become `current` automatically once all chunks land.
    #[serde(default)]
    pub activate: bool,
}

#[cw_ownable_execute]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
#[cw_serde]
pub enum ExecuteMsg {
    /// Owner: publish an immutable manifest; `version` must equal `next_version`.
    PublishManifest { manifest: Manifest },
    /// Permissionless: store one chunk whose sha256 matches the manifest.
    UploadChunk { version: u32, file: u16, index: u32, data: Binary },
    /// Owner: point `current` at a complete version.
    SetCurrent { version: u32 },
}

#[cw_serde]
pub struct ConfigResponse {
    pub max_chunk_bytes: u32,
    pub current: Option<u32>,
    pub next_version: u32,
}

#[cw_serde]
pub struct StatusResponse {
    pub version: u32,
    pub uploaded: u32,
    pub total: u32,
    pub complete: bool,
}

#[cw_serde]
pub struct VersionsResponse {
    pub versions: Vec<StatusResponse>,
}

#[cw_ownable_query]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    /// Defaults to `current`.
    #[returns(Manifest)]
    Manifest { version: Option<u32> },
    #[returns(Binary)]
    Chunk { version: u32, file: u16, index: u32 },
    #[returns(StatusResponse)]
    Status { version: u32 },
    #[returns(VersionsResponse)]
    Versions { start_after: Option<u32>, limit: Option<u32> },
}
