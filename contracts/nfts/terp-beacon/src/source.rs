use cosmwasm_schema::cw_serde;

/// Where a beacon value came from.
///
/// Verified drand outputs are stored under [`BeaconSource::Drand`].
/// Attestor signatures over a drand output are stored under [`BeaconSource::Attestor`].
/// Both are keyed by source id plus round (attestations also include the attestor key).
///
/// [`RESERVED_IBC_SOURCE_ID`] is the id a future IBC v2 client should use when a
/// hash-market runtime starts aggregating beacons. That client should verify the
/// packet, then write the same [`crate::state::StoredBeacon`] record the drand
/// path writes. There is no IBC entry point in this contract.
#[cw_serde]
#[derive(Copy, Eq)]
pub enum BeaconSource {
    Drand,
    Attestor,
}

/// Storage prefix reserved for a future IBC v2 beacon client. Not written today.
pub const RESERVED_IBC_SOURCE_ID: u8 = 3;

impl BeaconSource {
    pub const fn source_id(self) -> u8 {
        match self {
            Self::Drand => 1,
            Self::Attestor => 2,
        }
    }
}
