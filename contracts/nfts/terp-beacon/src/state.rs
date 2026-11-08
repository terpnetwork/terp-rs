use crate::msg::{AttestorKeyType, DrandScheme};
use crate::source::BeaconSource;
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Coin, HexBinary, Timestamp};
use cw_storage_plus::{Item, Map};

pub const DEFAULT_CALLBACK_GAS_LIMIT: u64 = 500_000;

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub chain_hash: HexBinary,
    pub pubkey: HexBinary,
    pub genesis_time: Timestamp,
    pub period_seconds: u64,
    pub scheme: DrandScheme,
    pub required_attestations: u32,
    pub prices: Vec<Coin>,
    pub callback_gas_limit: u64,
}

/// Verified drand output. `randomness` is sha256(signature).
/// `published` is the drand round time, not the chain block time.
#[cw_serde]
pub struct StoredBeacon {
    pub round: u64,
    pub randomness: HexBinary,
    pub signature: HexBinary,
    pub published: Timestamp,
    pub verified_at: Timestamp,
    pub source: BeaconSource,
}

#[cw_serde]
pub struct StoredAttestor {
    pub key_type: AttestorKeyType,
}

#[cw_serde]
pub struct StoredAttestation {
    pub signature: HexBinary,
    pub source: BeaconSource,
}

#[cw_serde]
pub struct StoredJob {
    pub after: Timestamp,
    /// Drand round committed at request time. Delivery uses this round only.
    pub round: u64,
}

pub const CONFIG: Item<Config> = Item::new("config");

/// Keyed by `(source id, round)`. Drand beacons use [`BeaconSource::Drand`].
pub const BEACONS: Map<(u8, u64), StoredBeacon> = Map::new("beacons");

/// Registered attestor keys, keyed by compressed pubkey bytes.
pub const ATTESTORS: Map<&[u8], StoredAttestor> = Map::new("attestors");

/// Attestations keyed by `(round, pubkey)`.
pub const ATTESTATIONS: Map<(u64, &[u8]), StoredAttestation> = Map::new("attestations");

/// Jobs keyed by `(requester, job_id)`.
pub const JOBS: Map<(&Addr, &str), StoredJob> = Map::new("jobs");

/// Queue ordered by the committed drand round, then requester, then job id.
pub const QUEUE: Map<(u64, &Addr, &str), u8> = Map::new("queue");

/// Marker so the queue value is a real byte. The key is the data.
pub const QUEUE_MARKER: u8 = 1;
