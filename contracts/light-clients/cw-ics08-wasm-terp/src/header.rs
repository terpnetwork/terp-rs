//! Lab header decode: JSON inner header, or TM Header protobuf (sigs skipped).

use prost::Message;

use crate::msg::Height;
use crate::state::InnerConsensus;
use crate::ContractError;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct JsonHeader {
    pub root_hex: String,
    #[serde(default)]
    pub timestamp_ns: u64,
    #[serde(default)]
    pub revision_number: u64,
    pub revision_height: u64,
    #[serde(default)]
    pub chain_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct Any {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct TmIbcHeader {
    #[prost(message, optional, tag = "1")]
    signed_header: Option<SignedHeader>,
    #[prost(message, optional, tag = "3")]
    trusted_height: Option<ProtoHeight>,
}

#[derive(Clone, PartialEq, Message)]
struct SignedHeader {
    #[prost(message, optional, tag = "1")]
    header: Option<RawHeader>,
}

#[derive(Clone, PartialEq, Message)]
struct RawHeader {
    #[prost(string, tag = "2")]
    chain_id: String,
    #[prost(int64, tag = "3")]
    height: i64,
    #[prost(message, optional, tag = "4")]
    time: Option<Timestamp>,
    #[prost(bytes = "vec", tag = "9")]
    next_validators_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "11")]
    app_hash: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct Timestamp {
    #[prost(int64, tag = "1")]
    seconds: i64,
    #[prost(int32, tag = "2")]
    nanos: i32,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoHeight {
    #[prost(uint64, tag = "1")]
    revision_number: u64,
    #[prost(uint64, tag = "2")]
    revision_height: u64,
}

pub struct ParsedHeader {
    pub height: Height,
    pub consensus: InnerConsensus,
    pub chain_id: String,
}

pub fn parse_client_message(bz: &[u8]) -> Result<ParsedHeader, ContractError> {
    if bz.first() == Some(&b'{') {
        let j: JsonHeader =
            serde_json::from_slice(bz).map_err(|e| ContractError::Json(e.to_string()))?;
        return Ok(ParsedHeader {
            height: Height {
                revision_number: j.revision_number,
                revision_height: j.revision_height,
            },
            consensus: InnerConsensus {
                timestamp_ns: j.timestamp_ns,
                root_hex: j.root_hex,
                tm: false,
                next_validators_hash: Vec::new(),
            },
            chain_id: j.chain_id,
        });
    }
    if let Ok(h) = decode_tm_header(bz) {
        return Ok(h);
    }
    if let Ok(any) = Any::decode(bz) {
        if !any.value.is_empty() {
            if let Ok(h) = decode_tm_header(&any.value) {
                return Ok(h);
            }
            if any.value.first() == Some(&b'{') {
                return parse_client_message(&any.value);
            }
        }
    }
    Err(ContractError::InvalidClientMessage)
}

fn decode_tm_header(bz: &[u8]) -> Result<ParsedHeader, ContractError> {
    let h = TmIbcHeader::decode(bz).map_err(|e| ContractError::Prost(e.to_string()))?;
    let raw = h
        .signed_header
        .and_then(|s| s.header)
        .ok_or(ContractError::InvalidClientMessage)?;
    if raw.app_hash.len() != 32 || raw.height <= 0 {
        return Err(ContractError::InvalidClientMessage);
    }
    let ts = raw.time.map(|t| {
        (t.seconds.max(0) as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(t.nanos.max(0) as u64)
    }).unwrap_or(0);
    let revision_number = h
        .trusted_height
        .map(|t| t.revision_number)
        .unwrap_or_else(|| revision_from_chain_id(&raw.chain_id));
    Ok(ParsedHeader {
        height: Height {
            revision_number,
            revision_height: raw.height as u64,
        },
        consensus: InnerConsensus {
            timestamp_ns: ts,
            root_hex: hex::encode(&raw.app_hash),
            tm: true,
            next_validators_hash: raw.next_validators_hash,
        },
        chain_id: raw.chain_id,
    })
}

fn revision_from_chain_id(chain_id: &str) -> u64 {
    chain_id
        .rsplit('-')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
