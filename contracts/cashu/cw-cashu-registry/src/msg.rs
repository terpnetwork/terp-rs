use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::Addr;
use std::collections::BTreeMap;

use crate::state::{Config, MintDescriptor, MintStatus};

// ---------------------------------------------------------------------------
// Instantiate
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct InstantiateMsg {
    /// Contract admin. Defaults to instantiate sender when omitted.
    pub admin: Option<String>,
    /// Cap on free-form metadata size (sum of key+value lengths). Default 512.
    pub max_metadata_bytes: Option<u32>,
    /// When true, non-admin may RegisterMint. Default false.
    #[serde(default)]
    pub allow_public_register: bool,
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

#[cw_serde]
pub enum ExecuteMsg {
    /// Insert a mint descriptor. Rejects duplicate mint_id or normalized url.
    /// Admin always; anyone when `allow_public_register`.
    RegisterMint {
        /// Optional override. Empty/None → `hex(sha256(normalize_url(url)))`.
        mint_id: Option<String>,
        url: String,
        name: Option<String>,
        #[serde(default)]
        units: Vec<String>,
        #[serde(default)]
        keyset_ids: Vec<String>,
        #[serde(default)]
        nuts: BTreeMap<String, String>,
        pubkey: Option<String>,
        /// Defaults to `active` when omitted.
        status: Option<MintStatus>,
        content_sha256: Option<String>,
        #[serde(default)]
        metadata: BTreeMap<String, String>,
    },

    /// Patch mutable fields. Admin or original registrar.
    UpdateMint {
        mint_id: String,
        url: Option<String>,
        name: Option<Option<String>>,
        units: Option<Vec<String>>,
        keyset_ids: Option<Vec<String>>,
        nuts: Option<BTreeMap<String, String>>,
        pubkey: Option<Option<String>>,
        content_sha256: Option<Option<String>>,
        metadata: Option<BTreeMap<String, String>>,
    },

    /// Admin only: set discovery status.
    SetStatus { mint_id: String, status: MintStatus },

    /// Admin only: soft-revoke (status = revoked). Keeps row for audit.
    RemoveMint { mint_id: String },

    /// Admin only: update config / transfer admin.
    UpdateConfig {
        admin: Option<String>,
        max_metadata_bytes: Option<u32>,
        allow_public_register: Option<bool>,
    },
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},

    #[returns(Option<MintDescriptor>)]
    Mint { mint_id: String },

    #[returns(Option<MintDescriptor>)]
    MintByUrl { url: String },

    /// Page of descriptors. Default status filter: **active only**.
    #[returns(ListMintsResponse)]
    ListMints {
        start_after: Option<String>,
        limit: Option<u32>,
        /// When set, only that status. When omitted, only `active`.
        status_filter: Option<MintStatus>,
    },

    /// True if mint_id is present, or normalized url maps to a mint.
    /// Provide exactly one of `mint_id` or `url`.
    #[returns(IsRegisteredResponse)]
    IsRegistered {
        mint_id: Option<String>,
        url: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct ConfigResponse {
    pub admin: Addr,
    pub max_metadata_bytes: u32,
    pub allow_public_register: bool,
}

impl From<Config> for ConfigResponse {
    fn from(c: Config) -> Self {
        Self {
            admin: c.admin,
            max_metadata_bytes: c.max_metadata_bytes,
            allow_public_register: c.allow_public_register,
        }
    }
}

#[cw_serde]
pub struct ListMintsResponse {
    pub mints: Vec<MintDescriptor>,
}

#[cw_serde]
pub struct IsRegisteredResponse {
    pub registered: bool,
    pub mint_id: Option<String>,
}
