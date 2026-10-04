use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{CosmosMsg, Empty, Timestamp};

#[cw_serde]
pub struct Member {
    pub address: String,
    pub mint_count: u32,
}

#[cw_serde]
pub struct InstantiateMsg {
    pub merkle_root: String,
    pub merkle_tree_uri: Option<String>,
    pub admins: Vec<String>,
    pub admins_mutable: bool,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    /// Change the admin set while it is still mutable.
    UpdateAdmins { admins: Vec<String> },
    /// Make the admin list immutable.
    Freeze {},
}

/// Current admins.
#[cw_serde]
pub struct AdminListResponse {
    pub admins: Vec<String>,
    pub mutable: bool,
}

#[cw_serde]
#[derive(QueryResponses)]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
pub enum QueryMsg {
    /// Whether an address is in the Merkle tree.
    #[returns(HasMemberResponse)]
    HasMember {
        member: String,
        proof_hashes: Vec<String>,
    },

    #[returns(AdminListResponse)]
    AdminList {},
    /// Whether an address may run admin executes.
    #[returns(CanExecuteResponse)]
    CanExecute {
        sender: String,
        msg: CosmosMsg<Empty>,
    },
    /// Current Merkle root.
    #[returns(MerkleRootResponse)]
    MerkleRoot {},
    /// Optional published tree URI.
    #[returns(MerkleTreeURIResponse)]
    MerkleTreeURI {},
}

#[cw_serde]
pub struct HasMemberResponse {
    pub has_member: bool,
}

#[cw_serde]
pub struct ConfigResponse {
    pub num_members: u32,
    pub member_limit: u32,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
    pub is_active: bool,
}

#[cw_serde]
pub struct MerkleRootResponse {
    pub merkle_root: String,
}

#[cw_serde]
pub struct MerkleTreeURIResponse {
    pub merkle_tree_uri: Option<String>,
}

#[cw_serde]
pub enum SudoMsg {
    /// Add a new operator
    AddOperator { operator: String },
    /// Remove operator
    RemoveOperator { operator: String },
}

#[cw_serde]
pub struct CanExecuteResponse {
    pub can_execute: bool,
}

// #[cw_serde]
// pub struct UploadTreeMsg {
//     pub tree_id: String,
//     pub tree_file: PathBuf,
//     pub key: String,
//     pub server: String,
// }
// #[cw_serde]
// pub struct DeleteTreeMsg {
//     pub tree_id: String,
//     pub key: String,
//     pub server: String,
// }
