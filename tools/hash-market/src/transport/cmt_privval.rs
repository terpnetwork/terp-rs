//! CometBFT 0.40 privval protobuf + VoteSignBytes (protoio delimited CanonicalVote).

use anyhow::{Context, Result};
use prost::Message;

pub const PREVOTE: i32 = 1;
pub const PRECOMMIT: i32 = 2;
pub const PROPOSAL: i32 = 32;

#[derive(Clone, PartialEq, Message)]
pub struct CanonicalPartSetHeader {
    #[prost(uint32, tag = "1")]
    pub total: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct CanonicalBlockId {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    #[prost(message, tag = "2")]
    pub part_set_header: Option<CanonicalPartSetHeader>,
}

#[derive(Clone, PartialEq, Message)]
pub struct CanonicalVote {
    #[prost(int32, tag = "1")]
    pub r#type: i32,
    #[prost(sfixed64, tag = "2")]
    pub height: i64,
    #[prost(sfixed64, tag = "3")]
    pub round: i64,
    #[prost(message, tag = "4")]
    pub block_id: Option<CanonicalBlockId>,
    #[prost(message, tag = "5")]
    pub timestamp: Option<prost_types::Timestamp>,
    #[prost(string, tag = "6")]
    pub chain_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct CanonicalProposal {
    #[prost(int32, tag = "1")]
    pub r#type: i32,
    #[prost(sfixed64, tag = "2")]
    pub height: i64,
    #[prost(sfixed64, tag = "3")]
    pub round: i64,
    #[prost(int64, tag = "4")]
    pub pol_round: i64,
    #[prost(message, tag = "5")]
    pub block_id: Option<CanonicalBlockId>,
    #[prost(message, tag = "6")]
    pub timestamp: Option<prost_types::Timestamp>,
    #[prost(string, tag = "7")]
    pub chain_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct PartSetHeader {
    #[prost(uint32, tag = "1")]
    pub total: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct BlockId {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    #[prost(message, tag = "2")]
    pub part_set_header: Option<PartSetHeader>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Vote {
    #[prost(int32, tag = "1")]
    pub r#type: i32,
    #[prost(int64, tag = "2")]
    pub height: i64,
    #[prost(int32, tag = "3")]
    pub round: i32,
    #[prost(message, tag = "4")]
    pub block_id: Option<BlockId>,
    #[prost(message, tag = "5")]
    pub timestamp: Option<prost_types::Timestamp>,
    #[prost(bytes = "vec", tag = "6")]
    pub validator_address: Vec<u8>,
    #[prost(int32, tag = "7")]
    pub validator_index: i32,
    #[prost(bytes = "vec", tag = "8")]
    pub signature: Vec<u8>,
    #[prost(bytes = "vec", tag = "9")]
    pub extension: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    pub extension_signature: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Proposal {
    #[prost(int32, tag = "1")]
    pub r#type: i32,
    #[prost(int64, tag = "2")]
    pub height: i64,
    #[prost(int32, tag = "3")]
    pub round: i32,
    #[prost(int32, tag = "4")]
    pub pol_round: i32,
    #[prost(message, tag = "5")]
    pub block_id: Option<BlockId>,
    #[prost(message, tag = "6")]
    pub timestamp: Option<prost_types::Timestamp>,
    #[prost(bytes = "vec", tag = "7")]
    pub signature: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PublicKey {
    #[prost(bytes = "bytes", tag = "1")]
    pub ed25519: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct RemoteSignerError {
    #[prost(int32, tag = "1")]
    pub code: i32,
    #[prost(string, tag = "2")]
    pub description: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct PubKeyRequest {
    #[prost(string, tag = "1")]
    pub chain_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct PubKeyResponse {
    #[prost(message, tag = "1")]
    pub pub_key: Option<PublicKey>,
    #[prost(message, tag = "2")]
    pub error: Option<RemoteSignerError>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SignVoteRequest {
    #[prost(message, tag = "1")]
    pub vote: Option<Vote>,
    #[prost(string, tag = "2")]
    pub chain_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct SignedVoteResponse {
    #[prost(message, tag = "1")]
    pub vote: Option<Vote>,
    #[prost(message, tag = "2")]
    pub error: Option<RemoteSignerError>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SignProposalRequest {
    #[prost(message, tag = "1")]
    pub proposal: Option<Proposal>,
    #[prost(string, tag = "2")]
    pub chain_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct SignedProposalResponse {
    #[prost(message, tag = "1")]
    pub proposal: Option<Proposal>,
    #[prost(message, tag = "2")]
    pub error: Option<RemoteSignerError>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PingRequest {}

#[derive(Clone, PartialEq, Message)]
pub struct PingResponse {}

#[derive(Clone, PartialEq, Message)]
pub struct PrivvalMessage {
    #[prost(oneof = "PrivvalSum", tags = "1, 2, 3, 4, 5, 6, 7, 8")]
    pub sum: Option<PrivvalSum>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum PrivvalSum {
    #[prost(message, tag = "1")]
    PubKeyRequest(PubKeyRequest),
    #[prost(message, tag = "2")]
    PubKeyResponse(PubKeyResponse),
    #[prost(message, tag = "3")]
    SignVoteRequest(SignVoteRequest),
    #[prost(message, tag = "4")]
    SignedVoteResponse(SignedVoteResponse),
    #[prost(message, tag = "5")]
    SignProposalRequest(SignProposalRequest),
    #[prost(message, tag = "6")]
    SignedProposalResponse(SignedProposalResponse),
    #[prost(message, tag = "7")]
    PingRequest(PingRequest),
    #[prost(message, tag = "8")]
    PingResponse(PingResponse),
}

pub fn marshal_delimited(msg: &impl Message) -> Result<Vec<u8>> {
    let body = msg.encode_to_vec();
    let mut out = Vec::new();
    prost::encoding::encode_varint(body.len() as u64, &mut out);
    out.extend_from_slice(&body);
    Ok(out)
}

fn block_id_canonical(b: &Option<BlockId>) -> Option<CanonicalBlockId> {
    let Some(b) = b else { return None };
    if b.hash.is_empty() {
        return None;
    }
    Some(CanonicalBlockId {
        hash: b.hash.clone(),
        part_set_header: b.part_set_header.as_ref().map(|p| CanonicalPartSetHeader {
            total: p.total,
            hash: p.hash.clone(),
        }),
    })
}

pub fn vote_sign_bytes(chain_id: &str, vote: &Vote) -> Result<Vec<u8>> {
    let cv = CanonicalVote {
        r#type: vote.r#type,
        height: vote.height,
        round: vote.round as i64,
        block_id: block_id_canonical(&vote.block_id),
        timestamp: vote.timestamp.clone(),
        chain_id: chain_id.to_string(),
    };
    marshal_delimited(&cv)
}

pub fn proposal_sign_bytes(chain_id: &str, p: &Proposal) -> Result<Vec<u8>> {
    let cp = CanonicalProposal {
        r#type: PROPOSAL,
        height: p.height,
        round: p.round as i64,
        pol_round: p.pol_round as i64,
        block_id: block_id_canonical(&p.block_id),
        timestamp: p.timestamp.clone(),
        chain_id: chain_id.to_string(),
    };
    marshal_delimited(&cp)
}

pub fn vote_step(vote: &Vote) -> i8 {
    match vote.r#type {
        PREVOTE => 1,
        PRECOMMIT => 2,
        _ => 0,
    }
}

pub fn encode_privval(msg: &PrivvalMessage) -> Result<Vec<u8>> {
    marshal_delimited(msg)
}

pub fn decode_privval(bytes: &[u8]) -> Result<PrivvalMessage> {
    let mut buf = bytes;
    let n = prost::encoding::decode_varint(&mut buf).map_err(|e| anyhow::anyhow!("{e}"))?;
    anyhow::ensure!(buf.len() as u64 >= n, "short privval");
    PrivvalMessage::decode(&buf[..n as usize]).context("privval decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_sign_bytes_matches_cometbft_0_40() {
        let vote = Vote {
            r#type: PREVOTE,
            height: 42,
            round: 1,
            block_id: Some(BlockId {
                hash: (1u8..=32).collect(),
                part_set_header: Some(PartSetHeader {
                    total: 1,
                    hash: vec![0u8; 32],
                }),
            }),
            timestamp: Some(prost_types::Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
            }),
            validator_address: vec![0u8; 20],
            validator_index: 0,
            signature: vec![],
            extension: vec![],
            extension_signature: vec![],
        };
        let bz = vote_sign_bytes("test-1", &vote).unwrap();
        let want = hex::decode("730801112a0000000000000019010000000000000022480a200102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2012240801122000000000000000000000000000000000000000000000000000000000000000002a0b0880e2cfaa0610959aef3a3206746573742d31").unwrap();
        assert_eq!(
            hex::encode(&bz),
            hex::encode(&want),
            "must match types.VoteSignBytes"
        );
    }
}
