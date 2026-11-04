use crate::source::BeaconSource;
use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Coin, HexBinary, Timestamp};

/// Drand signature scheme configured for this beacon instance.
///
/// JSON uses the scheme id published by drand (`/info` field `schemeID`).
/// The ids contain `-`, which the `cw_serde` schema derive cannot name, so
/// this enum derives serde and schemars directly and reports a string schema.
#[derive(
    serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema,
)]
pub enum DrandScheme {
    /// League of Entropy mainnet "default". Public key is 48-byte G1.
    /// Signature is 96-byte G2 and is chained to the previous round's signature.
    #[serde(rename = "pedersen-bls-chained")]
    PedersenBlsChained,
    /// Quicknet. Public key is 96-byte G2. Signature is 48-byte G1.
    /// `previous_signature` must be omitted.
    #[serde(rename = "bls-unchained-g1-rfc9380")]
    BlsUnchainedG1Rfc9380,
}

impl cosmwasm_schema::cw_schema::Schemaifier for DrandScheme {
    fn visit_schema(
        visitor: &mut cosmwasm_schema::cw_schema::SchemaVisitor,
    ) -> cosmwasm_schema::cw_schema::DefinitionReference {
        <String as cosmwasm_schema::cw_schema::Schemaifier>::visit_schema(visitor)
    }
}

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    /// 32-byte drand chain hash (network id).
    pub chain_hash: HexBinary,
    /// Compressed drand group public key. 48 bytes for chained, 96 bytes for quicknet.
    pub pubkey: HexBinary,
    pub genesis_time: Timestamp,
    pub period_seconds: u64,
    pub scheme: DrandScheme,
    /// Attestations required before a round can be delivered. Default 0 (drand only).
    #[serde(default)]
    pub required_attestations: Option<u32>,
    /// Accepted fees. Empty or omitted means requests are free (funds are still accepted).
    #[serde(default)]
    pub prices: Option<Vec<Coin>>,
    /// Gas limit on each nois callback submessage. Default 500_000.
    #[serde(default)]
    pub callback_gas_limit: Option<u64>,
}

/// Only secp256k1 compressed keys are accepted. The enum is the storage tag
/// a later ed25519 variant would extend.
#[cw_serde]
pub enum AttestorKeyType {
    Secp256k1,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    /// Verify a drand beacon and store `sha256(signature)` as the round randomness.
    ///
    /// `previous_signature` is required for [`DrandScheme::PedersenBlsChained`]
    /// unless round-1 is already stored. It must be omitted for quicknet.
    AddRound {
        round: u64,
        signature: HexBinary,
        #[serde(default)]
        previous_signature: Option<HexBinary>,
    },
    /// Attest a stored drand round. The signer must be a registered attestor.
    ///
    /// Message is documented in the crate README and [`crate::attestor`].
    Attest {
        round: u64,
        pubkey: HexBinary,
        signature: HexBinary,
    },
    AddAttestor {
        pubkey: HexBinary,
    },
    RemoveAttestor {
        pubkey: HexBinary,
    },
    TransferAdmin {
        new_admin: String,
    },
    UpdateConfig {
        required_attestations: Option<u32>,
        /// `Some` replaces the price list. An empty list makes requests free.
        prices: Option<Vec<Coin>>,
        callback_gas_limit: Option<u64>,
    },
    Withdraw {
        recipient: Option<String>,
        denom: String,
        /// Omit to withdraw the full balance of `denom`.
        amount: Option<cosmwasm_std::Uint256>,
    },
    /// Deliver up to the per-tx callback batch. Permissionless.
    ProcessQueue {},
    /// Pull entrypoint for `nftfi-raffle` (`RandomnessSourceMsg::Callback`).
    ///
    /// The sender is the raffle contract. The beacon reads raffle `raffle_id`
    /// from the sender, takes the first drand round published strictly after
    /// the raffle end (`start + duration`), and answers with the attributes
    /// the raffle reply reads: `raffle_id`, `round`, `randomness` (base64,
    /// 32 bytes) and `owner`. Fails when that round is not stored or not yet
    /// deliverable under `required_attestations`.
    Provide {
        raffle_id: u64,
        owner: String,
    },
    /// Same JSON as `nois::ProxyExecuteMsg::GetNextRandomness`.
    #[cfg_attr(feature = "interface", cw_orch(payable))]
    GetNextRandomness {
        job_id: String,
    },
    /// Same JSON as `nois::ProxyExecuteMsg::GetRandomnessAfter`.
    #[cfg_attr(feature = "interface", cw_orch(payable))]
    GetRandomnessAfter {
        after: Timestamp,
        job_id: String,
    },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub chain_hash: HexBinary,
    pub pubkey: HexBinary,
    pub genesis_time: Timestamp,
    pub period_seconds: u64,
    pub scheme: DrandScheme,
    pub required_attestations: u32,
    pub prices: Vec<Coin>,
    pub callback_gas_limit: u64,
    /// Source ids currently written, plus the reserved IBC id.
    pub source_ids: SourceIds,
}

#[cw_serde]
pub struct SourceIds {
    pub drand: u8,
    pub attestor: u8,
    pub reserved_ibc: u8,
}

#[cw_serde]
pub struct BeaconResponse {
    pub round: u64,
    pub randomness: HexBinary,
    pub signature: HexBinary,
    pub published: Timestamp,
    pub verified_at: Timestamp,
    pub source: BeaconSource,
    pub attestation_count: u32,
    pub deliverable: bool,
}

#[cw_serde]
pub struct AttestorInfo {
    pub pubkey: HexBinary,
    pub key_type: AttestorKeyType,
}

#[cw_serde]
pub struct AttestorsResponse {
    pub attestors: Vec<AttestorInfo>,
}

#[cw_serde]
pub struct AttestationInfo {
    pub pubkey: HexBinary,
    pub signature: HexBinary,
    pub source: BeaconSource,
}

#[cw_serde]
pub struct AttestationsResponse {
    pub round: u64,
    pub attestations: Vec<AttestationInfo>,
}

#[cw_serde]
pub struct JobInfo {
    pub requester: String,
    pub job_id: String,
    pub after: Timestamp,
    /// First drand round whose publish time is strictly after `after`.
    pub round: u64,
}

#[cw_serde]
pub struct JobResponse {
    pub job: Option<JobInfo>,
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
#[derive(QueryResponses)]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(BeaconResponse)]
    Beacon { round: u64 },
    #[returns(AttestorsResponse)]
    Attestors {},
    #[returns(AttestationsResponse)]
    Attestations { round: u64 },
    #[returns(JobResponse)]
    Job { requester: String, job_id: String },
}
