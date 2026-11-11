use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary};
use cw_storage_plus::{Item, Map};

use crate::committee::{CommitteePolicy, KeyScheme, Member};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub label: String,
    pub target: String,
    pub registry: Addr,
    pub client_id: String,
    pub aggregator: Option<Addr>,
    pub policy: CommitteePolicy,
}

#[cw_serde]
pub enum ProposalReason {
    Bootstrap,
    RotateShard { by: Addr, note: String },
    LivenessFault,
    Equivocation { note: String },
}

#[cw_serde]
pub enum EpochStatus {
    Pending,
    Active,
    Retired,
    Faulted { reason: String },
    Expired,
}

#[cw_serde]
pub struct Epoch {
    pub epoch: u64,
    pub status: EpochStatus,
    pub reason: ProposalReason,
    pub members: Vec<Member>,
    pub total_shares: u16,
    pub threshold: u16,
    /// BLAKE3 over the member list; bound into the activation digest.
    pub members_hash: Binary,
    pub group_key: Option<Binary>,
    pub scheme: Option<KeyScheme>,
    pub proposed_at: u64,
    pub activated_at: Option<u64>,
    pub ended_at: Option<u64>,
}

impl Epoch {
    pub fn member(&self, a: &Addr) -> Option<&crate::committee::Member> {
        self.members.iter().find(|m| m.operator == *a)
    }
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const EPOCHS: Map<u64, Epoch> = Map::new("epochs");
pub const ACTIVE: Item<Option<u64>> = Item::new("active");
pub const PENDING: Item<Option<u64>> = Item::new("pending");
pub const LAST_EPOCH: Item<u64> = Item::new("last_epoch");
/// (epoch, member) → acknowledged group key.
pub const ACKS: Map<(u64, &Addr), Binary> = Map::new("acks");
