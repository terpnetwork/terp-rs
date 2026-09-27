use cosmwasm_std::Storage;
use prost::Message;

use crate::msg::Height;
use crate::ContractError;

pub const HOST_CLIENT_STATE_KEY: &str = "clientState";
pub const HOST_CONSENSUS_STATES_KEY: &str = "consensusStates";

const WASM_CLIENT_TYPE_URL: &str = "/ibc.lightclients.wasm.v1.ClientState";
const WASM_CONSENSUS_TYPE_URL: &str = "/ibc.lightclients.wasm.v1.ConsensusState";

#[derive(Clone, PartialEq, Message)]
struct Any {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct WasmClientState {
    #[prost(bytes = "vec", tag = "1")]
    data: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    checksum: Vec<u8>,
    #[prost(message, optional, tag = "3")]
    latest_height: Option<ProtoHeight>,
}

#[derive(Clone, PartialEq, Message)]
struct WasmConsensusState {
    #[prost(bytes = "vec", tag = "1")]
    data: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoHeight {
    #[prost(uint64, tag = "1")]
    revision_number: u64,
    #[prost(uint64, tag = "2")]
    revision_height: u64,
}

#[derive(Clone, PartialEq, Message)]
struct Fraction {
    #[prost(uint64, tag = "1")]
    numerator: u64,
    #[prost(uint64, tag = "2")]
    denominator: u64,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoDuration {
    #[prost(int64, tag = "1")]
    seconds: i64,
    #[prost(int32, tag = "2")]
    nanos: i32,
}

#[derive(Clone, PartialEq, Message)]
struct Timestamp {
    #[prost(int64, tag = "1")]
    seconds: i64,
    #[prost(int32, tag = "2")]
    nanos: i32,
}

#[derive(Clone, PartialEq, Message)]
struct MerkleRoot {
    #[prost(bytes = "vec", tag = "1")]
    hash: Vec<u8>,
}

/// ICS-07 Tendermint ClientState (inner bytes of 08-wasm ClientState.data).
/// Hermes `--wasm-checksum` wraps this protobuf, not our lab JSON.
#[derive(Clone, PartialEq, Message)]
struct TmClientState {
    #[prost(string, tag = "1")]
    chain_id: String,
    #[prost(message, optional, tag = "2")]
    trust_level: Option<Fraction>,
    #[prost(message, optional, tag = "3")]
    trusting_period: Option<ProtoDuration>,
    #[prost(message, optional, tag = "4")]
    unbonding_period: Option<ProtoDuration>,
    #[prost(message, optional, tag = "5")]
    max_clock_drift: Option<ProtoDuration>,
    #[prost(message, optional, tag = "6")]
    frozen_height: Option<ProtoHeight>,
    #[prost(message, optional, tag = "7")]
    latest_height: Option<ProtoHeight>,
    #[prost(message, repeated, tag = "8")]
    proof_specs: Vec<ics23::ProofSpec>,
    #[prost(string, repeated, tag = "9")]
    upgrade_path: Vec<String>,
    #[prost(bool, tag = "10")]
    allow_update_after_expiry: bool,
    #[prost(bool, tag = "11")]
    allow_update_after_misbehaviour: bool,
}

#[derive(Clone, PartialEq, Message)]
struct TmConsensusState {
    #[prost(message, optional, tag = "1")]
    timestamp: Option<Timestamp>,
    #[prost(message, optional, tag = "2")]
    root: Option<MerkleRoot>,
    #[prost(bytes = "vec", tag = "3")]
    next_validators_hash: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct InnerClient {
    pub chain_id: String,
    pub latest_height: Height,
    #[serde(default)]
    pub hasher_mode: String,
    #[serde(default)]
    pub frozen: bool,
    /// Full ICS-07 ClientState protobuf when Hermes (or ibc-go) wraps TM.
    #[serde(default, skip)]
    pub tm_client_state: Option<Vec<u8>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct InnerConsensus {
    pub timestamp_ns: u64,
    pub root_hex: String,
    #[serde(default, skip)]
    pub tm: bool,
    #[serde(default, skip)]
    pub next_validators_hash: Vec<u8>,
}

pub fn consensus_db_key(height: &Height) -> String {
    format!(
        "{}/{}-{}",
        HOST_CONSENSUS_STATES_KEY, height.revision_number, height.revision_height
    )
}

fn put_any(storage: &mut dyn Storage, key: &[u8], type_url: &str, value: Vec<u8>) {
    let any = Any {
        type_url: type_url.into(),
        value,
    };
    storage.set(key, &any.encode_to_vec());
}

fn get_any(storage: &dyn Storage, key: &[u8]) -> Result<Any, ContractError> {
    let bz = storage
        .get(key)
        .ok_or(ContractError::ClientStateNotFound)?;
    Any::decode(bz.as_slice()).map_err(|e| ContractError::Prost(e.to_string()))
}

pub fn store_wasm_client(
    storage: &mut dyn Storage,
    data: Vec<u8>,
    checksum: Vec<u8>,
    latest_height: Height,
) -> Result<(), ContractError> {
    let cs = WasmClientState {
        data,
        checksum,
        latest_height: Some(ProtoHeight {
            revision_number: latest_height.revision_number,
            revision_height: latest_height.revision_height,
        }),
    };
    put_any(
        storage,
        HOST_CLIENT_STATE_KEY.as_bytes(),
        WASM_CLIENT_TYPE_URL,
        cs.encode_to_vec(),
    );
    Ok(())
}

pub fn store_wasm_consensus(
    storage: &mut dyn Storage,
    height: &Height,
    data: Vec<u8>,
) -> Result<(), ContractError> {
    let cs = WasmConsensusState { data };
    put_any(
        storage,
        consensus_db_key(height).as_bytes(),
        WASM_CONSENSUS_TYPE_URL,
        cs.encode_to_vec(),
    );
    Ok(())
}

pub fn load_inner_client(storage: &dyn Storage) -> Result<InnerClient, ContractError> {
    let any = get_any(storage, HOST_CLIENT_STATE_KEY.as_bytes())?;
    let wasm = WasmClientState::decode(any.value.as_slice())
        .map_err(|e| ContractError::Prost(e.to_string()))?;
    parse_inner_client(&wasm.data)
}

pub fn load_checksum(storage: &dyn Storage) -> Result<Vec<u8>, ContractError> {
    let any = get_any(storage, HOST_CLIENT_STATE_KEY.as_bytes())?;
    let wasm = WasmClientState::decode(any.value.as_slice())
        .map_err(|e| ContractError::Prost(e.to_string()))?;
    Ok(wasm.checksum)
}

pub fn load_inner_consensus(
    storage: &dyn Storage,
    height: &Height,
) -> Result<InnerConsensus, ContractError> {
    let bz = storage
        .get(consensus_db_key(height).as_bytes())
        .ok_or(ContractError::ConsensusStateNotFound)?;
    let any = Any::decode(bz.as_slice()).map_err(|e| ContractError::Prost(e.to_string()))?;
    let wasm = WasmConsensusState::decode(any.value.as_slice())
        .map_err(|e| ContractError::Prost(e.to_string()))?;
    parse_inner_consensus(&wasm.data)
}

fn frozen_from_height(h: Option<&ProtoHeight>) -> bool {
    h.map(|h| h.revision_height > 0 || h.revision_number > 0)
        .unwrap_or(false)
}

fn inner_bytes(data: &[u8]) -> Vec<u8> {
    if let Ok(any) = Any::decode(data) {
        if any.type_url.contains("tendermint") && !any.value.is_empty() {
            return any.value;
        }
    }
    data.to_vec()
}

fn wrap_tm_any(type_url: &str, value: Vec<u8>) -> Vec<u8> {
    Any {
        type_url: type_url.into(),
        value,
    }
    .encode_to_vec()
}

const TM_CLIENT_TYPE_URL: &str = "/ibc.lightclients.tendermint.v1.ClientState";
const TM_CONSENSUS_TYPE_URL: &str = "/ibc.lightclients.tendermint.v1.ConsensusState";

fn inner_from_tm(data: &[u8]) -> Result<InnerClient, ContractError> {
    let body = inner_bytes(data);
    let tm = TmClientState::decode(body.as_slice()).map_err(|e| ContractError::Prost(e.to_string()))?;
    if tm.chain_id.is_empty() && tm.latest_height.is_none() {
        return Err(ContractError::Json("not tendermint client state".into()));
    }
    let latest = tm.latest_height.clone().unwrap_or(ProtoHeight {
        revision_number: 0,
        revision_height: 0,
    });
    Ok(InnerClient {
        chain_id: tm.chain_id,
        latest_height: Height {
            revision_number: latest.revision_number,
            revision_height: latest.revision_height,
        },
        hasher_mode: "auto".into(),
        frozen: frozen_from_height(tm.frozen_height.as_ref()),
        tm_client_state: Some(data.to_vec()),
    })
}

fn consensus_from_tm(data: &[u8]) -> Result<InnerConsensus, ContractError> {
    let body = inner_bytes(data);
    let tm = TmConsensusState::decode(body.as_slice()).map_err(|e| ContractError::Prost(e.to_string()))?;
    let hash = tm.root.as_ref().map(|r| r.hash.as_slice()).unwrap_or(&[]);
    if hash.is_empty() {
        return Err(ContractError::Json("not tendermint consensus state".into()));
    }
    let timestamp_ns = tm
        .timestamp
        .as_ref()
        .map(|t| {
            (t.seconds.max(0) as u64)
                .saturating_mul(1_000_000_000)
                .saturating_add(t.nanos.max(0) as u64)
        })
        .unwrap_or(0);
    Ok(InnerConsensus {
        timestamp_ns,
        root_hex: hex::encode(hash),
        tm: true,
        next_validators_hash: tm.next_validators_hash,
    })
}

pub fn parse_inner_client(data: &[u8]) -> Result<InnerClient, ContractError> {
    if data.first() == Some(&b'{') {
        return serde_json::from_slice(data).map_err(|e| ContractError::Json(e.to_string()));
    }
    inner_from_tm(data)
}

pub fn parse_inner_consensus(data: &[u8]) -> Result<InnerConsensus, ContractError> {
    if data.first() == Some(&b'{') {
        return serde_json::from_slice(data).map_err(|e| ContractError::Json(e.to_string()));
    }
    consensus_from_tm(data)
}

pub fn encode_inner_client(c: &InnerClient) -> Result<Vec<u8>, ContractError> {
    if let Some(raw) = &c.tm_client_state {
        let body = inner_bytes(raw);
        let mut tm = TmClientState::decode(body.as_slice())
            .map_err(|e| ContractError::Prost(e.to_string()))?;
        tm.chain_id = c.chain_id.clone();
        tm.latest_height = Some(ProtoHeight {
            revision_number: c.latest_height.revision_number,
            revision_height: c.latest_height.revision_height,
        });
        if c.frozen {
            tm.frozen_height = Some(ProtoHeight {
                revision_number: c.latest_height.revision_number,
                revision_height: c.latest_height.revision_height,
            });
        }
        let encoded = tm.encode_to_vec();
        if let Ok(any) = Any::decode(raw.as_slice()) {
            if any.type_url.contains("tendermint") {
                return Ok(wrap_tm_any(&any.type_url, encoded));
            }
        }
        return Ok(wrap_tm_any(TM_CLIENT_TYPE_URL, encoded));
    }
    serde_json::to_vec(c).map_err(|e| ContractError::Json(e.to_string()))
}

pub fn encode_inner_consensus(c: &InnerConsensus) -> Result<Vec<u8>, ContractError> {
    if c.tm {
        let hash = c.root_bytes()?;
        let seconds = (c.timestamp_ns / 1_000_000_000) as i64;
        let nanos = (c.timestamp_ns % 1_000_000_000) as i32;
        let tm = TmConsensusState {
            timestamp: Some(Timestamp { seconds, nanos }),
            root: Some(MerkleRoot { hash }),
            next_validators_hash: c.next_validators_hash.clone(),
        };
        return Ok(wrap_tm_any(TM_CONSENSUS_TYPE_URL, tm.encode_to_vec()));
    }
    serde_json::to_vec(c).map_err(|e| ContractError::Json(e.to_string()))
}

impl InnerConsensus {
    pub fn root_bytes(&self) -> Result<Vec<u8>, ContractError> {
        hex::decode(self.root_hex.trim().trim_start_matches("0x"))
            .map_err(|e| ContractError::Hex(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_inner_round_trip() {
        let c = InnerClient {
            chain_id: "terp-a-1".into(),
            latest_height: Height {
                revision_number: 1,
                revision_height: 9,
            },
            hasher_mode: "blake3".into(),
            frozen: false,
            tm_client_state: None,
        };
        let bz = encode_inner_client(&c).unwrap();
        assert_eq!(bz.first(), Some(&b'{'));
        let back = parse_inner_client(&bz).unwrap();
        assert_eq!(back.chain_id, "terp-a-1");
        assert_eq!(back.hasher_mode, "blake3");
        assert!(back.tm_client_state.is_none());
    }

    #[test]
    fn tm_inner_round_trip_preserves_proof_specs() {
        let tm = TmClientState {
            chain_id: "terp-a-1".into(),
            trust_level: Some(Fraction {
                numerator: 1,
                denominator: 3,
            }),
            trusting_period: Some(ProtoDuration {
                seconds: 1209600,
                nanos: 0,
            }),
            unbonding_period: Some(ProtoDuration {
                seconds: 1814400,
                nanos: 0,
            }),
            max_clock_drift: Some(ProtoDuration {
                seconds: 5,
                nanos: 0,
            }),
            frozen_height: Some(ProtoHeight {
                revision_number: 0,
                revision_height: 0,
            }),
            latest_height: Some(ProtoHeight {
                revision_number: 1,
                revision_height: 42,
            }),
            proof_specs: vec![ics23::blake3_iavl_spec(), ics23::tendermint_spec()],
            upgrade_path: vec!["upgrade".into(), "upgradedIBCState".into()],
            allow_update_after_expiry: false,
            allow_update_after_misbehaviour: false,
        };
        let raw = tm.encode_to_vec();
        let inner = parse_inner_client(&raw).unwrap();
        assert_eq!(inner.chain_id, "terp-a-1");
        assert_eq!(inner.hasher_mode, "auto");
        assert_eq!(inner.latest_height.revision_height, 42);
        assert!(inner.tm_client_state.is_some());
        let mut inner = inner;
        inner.latest_height.revision_height = 43;
        let again = encode_inner_client(&inner).unwrap();
        let body = inner_bytes(&again);
        let tm2 = TmClientState::decode(body.as_slice()).unwrap();
        assert_eq!(tm2.latest_height.unwrap().revision_height, 43);
        assert_eq!(tm2.trust_level.unwrap().denominator, 3);
        assert_eq!(tm2.proof_specs.len(), 2);
        assert_eq!(tm2.upgrade_path.len(), 2);
    }

    #[test]
    fn hermes_any_wrapped_tm_inner() {
        let tm = TmClientState {
            chain_id: "terp-a-1".into(),
            latest_height: Some(ProtoHeight {
                revision_number: 1,
                revision_height: 7,
            }),
            ..Default::default()
        };
        let wrapped = wrap_tm_any(TM_CLIENT_TYPE_URL, tm.encode_to_vec());
        let inner = parse_inner_client(&wrapped).unwrap();
        assert_eq!(inner.chain_id, "terp-a-1");
        assert_eq!(inner.hasher_mode, "auto");
        assert_eq!(inner.latest_height.revision_height, 7);
        let again = encode_inner_client(&inner).unwrap();
        let any = Any::decode(again.as_slice()).unwrap();
        assert_eq!(any.type_url, TM_CLIENT_TYPE_URL);
        let tm2 = TmClientState::decode(any.value.as_slice()).unwrap();
        assert_eq!(tm2.chain_id, "terp-a-1");
    }
}
