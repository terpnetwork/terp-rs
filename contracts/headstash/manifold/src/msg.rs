use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Addr, Binary, Uint256};
use cw_headstash::distro::DistroHashDomain;
use cw_headstash::msg::InstantiateMsg as HeadstashInstantiateMsg;

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    pub headstash_code_id: u64,
}

#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
#[cw_serde]
pub enum ExecuteMsg {
    UpdateOwnership(cw_ownable::Action),
    CreateHeadstash {
        instantiate_msg: HeadstashInstantiateMsg,
        label: Option<String>,
        funding: Option<FundingInfo>,
    },
    /// Mirror an additive eligibility root into the manifold registry after
    /// (or while) registering it on the Headstash contract.
    ///
    /// If `forward_to_headstash` is true, emits a Wasm execute to
    /// `RegisterEligibilityRoot` on the Headstash. Owner-only.
    RegisterEligibilityRoot {
        headstash: String,
        root: Binary,
        /// Defaults to poseidon-v1 when omitted.
        #[serde(default)]
        domain: Option<DistroHashDomain>,
        #[serde(default)]
        label: Option<String>,
        /// When true (default), also call the Headstash contract.
        #[serde(default = "default_true")]
        forward_to_headstash: bool,
        /// If known (e.g. after Headstash reply), pin the root_id; otherwise
        /// manifold assigns from its own counter per headstash only when not
        /// forwarding. Prefer forwarding so Headstash is source of truth.
        #[serde(default)]
        root_id: Option<u64>,
    },
}

fn default_true() -> bool {
    true
}

#[cw_serde]
pub struct FundingInfo {
    pub amount: Uint256,
    pub token: FundingToken,
}

#[cw_serde]
pub enum FundingToken {
    Native { denom: String },
    Cw20 { contract_addr: String },
}

#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(cw_ownable::Ownership<Addr>)]
    Ownership {},

    #[returns(Vec<HeadstashContract>)]
    ContractsByInstantiator {
        instantiator: String,
        start_after: Option<String>,
        limit: Option<u32>,
    },

    #[returns(HeadstashContract)]
    Contract { address: String },

    /// All eligibility roots registered under a Headstash in the manifold index.
    #[returns(Vec<EligibilityRootRecord>)]
    EligibilityRoots {
        headstash: String,
        start_after: Option<u64>,
        limit: Option<u32>,
    },

    #[returns(EligibilityRootRecord)]
    EligibilityRoot {
        headstash: String,
        root_id: u64,
    },
}

#[cw_serde]
pub struct HeadstashContract {
    pub address: Addr,
    pub instantiator: Addr,
    pub genesis_root: Binary,
    /// Public inclusion hash domain for this Headstash (default poseidon-v1).
    #[serde(default)]
    pub distro_hash_domain: DistroHashDomain,
    pub funding: Option<FundingInfo>,
}

/// Manifold-side view of one additive eligibility root.
#[cw_serde]
pub struct EligibilityRootRecord {
    pub headstash: Addr,
    pub root_id: u64,
    pub root: Binary,
    pub domain: DistroHashDomain,
    pub label: Option<String>,
}

#[cw_serde]
pub struct MigrateMsg {}
