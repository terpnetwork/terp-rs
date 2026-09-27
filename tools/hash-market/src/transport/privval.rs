//! Privval sidecar: SessionCrypto + ConsensusSigner.
//!
//! Two codecs on the same loop:
//! - `json` — lab / e2e (`op: pubkey|sign|ping`)
//! - `proto` — length-prefixed protobuf `SignBytes` / `PubKey` / `Ping`
//!   (canonical sign bytes supplied by the caller; full Vote proto is the
//!   cometbft codec team's follow-up behind this same trait).

use std::sync::Arc;

use tokio::sync::Mutex;

use anyhow::{Context, Result};
use prost::Message;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use crate::custody::consensus::ConsensusSigner;
use crate::custody::hrs::HrsLock;

use super::session::SessionCrypto;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivvalConfig {
    #[serde(default)]
    pub enabled: bool,
    pub addr: String,
    #[serde(default = "default_crypto")]
    pub crypto: String,
    /// `json` (default, e2e) or `proto`
    #[serde(default = "default_codec")]
    pub codec: String,
}

fn default_crypto() -> String {
    "noise".into()
}
fn default_codec() -> String {
    "json".into()
}

#[derive(Serialize, Deserialize)]
struct JsonReq {
    op: String,
    #[serde(default)]
    msg_hex: String,
    #[serde(default)]
    height: i64,
    #[serde(default)]
    round: i32,
    #[serde(default)]
    step: i8,
}

#[derive(Serialize, Deserialize)]
struct JsonResp {
    sig_hex: Option<String>,
    pubkey_hex: Option<String>,
    error: Option<String>,
}

// Minimal privval subset (canonical bytes in, 64-byte sig out).
#[derive(Clone, PartialEq, Message)]
struct ProtoMessage {
    #[prost(oneof = "ProtoSum", tags = "1, 2, 3, 4, 5, 6")]
    pub sum: Option<ProtoSum>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
enum ProtoSum {
    #[prost(message, tag = "1")]
    PubKeyRequest(ProtoEmpty),
    #[prost(message, tag = "2")]
    PubKeyResponse(ProtoBytes),
    #[prost(message, tag = "3")]
    SignBytesRequest(ProtoBytes),
    #[prost(message, tag = "4")]
    SignBytesResponse(ProtoBytes),
    #[prost(message, tag = "5")]
    PingRequest(ProtoEmpty),
    #[prost(message, tag = "6")]
    PingResponse(ProtoEmpty),
}

#[derive(Clone, PartialEq, Message)]
struct ProtoEmpty {}

#[derive(Clone, PartialEq, Message)]
struct ProtoBytes {
    #[prost(bytes = "vec", tag = "1")]
    pub body: Vec<u8>,
    #[prost(string, tag = "2")]
    pub error: String,
}

pub async fn serve_once(
    crypto: &dyn SessionCrypto,
    addr: &str,
    signer: Arc<dyn ConsensusSigner>,
    codec: &str,
    hrs: Option<Arc<Mutex<HrsLock>>>,
) -> Result<()> {
    let host = addr
        .trim_start_matches("tcp://")
        .trim_start_matches("noise://");
    let raw = TcpStream::connect(host).await?;
    let mut sess = crypto.upgrade_outbound(raw).await?;
    loop {
        let frame = sess.read_frame().await?;
        if codec == "cmt" {
            // CometBFT protoio: uvarint + Message on the stream (already in `frame` if we
            // read a 4-byte lab frame containing a delimited message, or raw encode).
            let out = handle_cmt(&frame, signer.as_ref(), hrs.as_ref()).await?;
            sess.write_frame(&out).await?;
            continue;
        }
        let out = if codec == "proto" {
            handle_proto(&frame, signer.as_ref(), hrs.as_ref()).await?
        } else {
            handle_json(&frame, signer.as_ref(), hrs.as_ref()).await?
        };
        sess.write_frame(&out).await?;
    }
}

async fn handle_json(
    frame: &[u8],
    signer: &dyn ConsensusSigner,
    hrs: Option<&Arc<Mutex<HrsLock>>>,
) -> Result<Vec<u8>> {
    let req: JsonReq = serde_json::from_slice(frame)?;
    let resp = match req.op.as_str() {
        "ping" => JsonResp {
            pubkey_hex: None,
            sig_hex: None,
            error: None,
        },
        "pubkey" => JsonResp {
            pubkey_hex: Some(hex::encode(signer.group_pubkey())),
            sig_hex: None,
            error: None,
        },
        "sign" => match sign_gated(signer, hrs, &req).await {
            Ok(sig) => JsonResp {
                sig_hex: Some(hex::encode(sig)),
                pubkey_hex: None,
                error: None,
            },
            Err(e) => JsonResp {
                sig_hex: None,
                pubkey_hex: None,
                error: Some(e.to_string()),
            },
        },
        _ => JsonResp {
            sig_hex: None,
            pubkey_hex: None,
            error: Some("unknown op".into()),
        },
    };
    Ok(serde_json::to_vec(&resp)?)
}

async fn sign_gated(
    signer: &dyn ConsensusSigner,
    hrs: Option<&Arc<Mutex<HrsLock>>>,
    req: &JsonReq,
) -> Result<[u8; 64]> {
    let msg = hex::decode(req.msg_hex.trim())?;
    if let Some(lock) = hrs {
        let mut g = lock.lock().await;
        if let Some(cached) = g.check(req.height, req.round, req.step)? {
            let mut out = [0u8; 64];
            anyhow::ensure!(cached.len() == 64, "cached sig");
            out.copy_from_slice(&cached);
            return Ok(out);
        }
        drop(g);
        let sig = signer.sign_consensus_for(&msg, req.step).await?;
        let mut g = lock.lock().await;
        g.commit(req.height, req.round, req.step, &sig)?;
        return Ok(sig);
    }
    signer.sign_consensus_for(&msg, req.step).await
}

async fn handle_proto(
    frame: &[u8],
    signer: &dyn ConsensusSigner,
    hrs: Option<&Arc<Mutex<HrsLock>>>,
) -> Result<Vec<u8>> {
    let msg = ProtoMessage::decode(frame)?;
    let reply = match msg.sum {
        Some(ProtoSum::PingRequest(_)) => ProtoMessage {
            sum: Some(ProtoSum::PingResponse(ProtoEmpty {})),
        },
        Some(ProtoSum::PubKeyRequest(_)) => ProtoMessage {
            sum: Some(ProtoSum::PubKeyResponse(ProtoBytes {
                body: signer.group_pubkey().to_vec(),
                error: String::new(),
            })),
        },
        Some(ProtoSum::SignBytesRequest(b)) => {
            let fake = JsonReq {
                op: "sign".into(),
                msg_hex: hex::encode(&b.body),
                height: 0,
                round: 0,
                step: 0,
            };
            match sign_gated(signer, hrs, &fake).await {
                Ok(sig) => ProtoMessage {
                    sum: Some(ProtoSum::SignBytesResponse(ProtoBytes {
                        body: sig.to_vec(),
                        error: String::new(),
                    })),
                },
                Err(e) => ProtoMessage {
                    sum: Some(ProtoSum::SignBytesResponse(ProtoBytes {
                        body: vec![],
                        error: e.to_string(),
                    })),
                },
            }
        }
        _ => ProtoMessage {
            sum: Some(ProtoSum::PubKeyResponse(ProtoBytes {
                body: vec![],
                error: "unknown".into(),
            })),
        },
    };
    let mut out = Vec::new();
    reply.encode(&mut out)?;
    Ok(out)
}

async fn handle_cmt(
    frame: &[u8],
    signer: &dyn ConsensusSigner,
    hrs: Option<&Arc<Mutex<HrsLock>>>,
) -> Result<Vec<u8>> {
    use super::cmt_privval::*;
    let msg = if let Ok(m) = decode_privval(frame) {
        m
    } else {
        PrivvalMessage::decode(frame).context("cmt privval")?
    };
    let reply = match msg.sum {
        Some(PrivvalSum::PingRequest(_)) => PrivvalMessage {
            sum: Some(PrivvalSum::PingResponse(PingResponse {})),
        },
        Some(PrivvalSum::PubKeyRequest(_)) => PrivvalMessage {
            sum: Some(PrivvalSum::PubKeyResponse(PubKeyResponse {
                pub_key: Some(PublicKey {
                    ed25519: signer.group_pubkey().to_vec(),
                }),
                error: None,
            })),
        },
        Some(PrivvalSum::SignVoteRequest(req)) => {
            let chain = req.chain_id;
            let mut vote = req.vote.context("sign vote missing")?;
            let bz = vote_sign_bytes(&chain, &vote)?;
            let fake = JsonReq {
                op: "sign".into(),
                msg_hex: hex::encode(&bz),
                height: vote.height,
                round: vote.round,
                step: vote_step(&vote),
            };
            match sign_gated(signer, hrs, &fake).await {
                Ok(sig) => {
                    vote.signature = sig.to_vec();
                    PrivvalMessage {
                        sum: Some(PrivvalSum::SignedVoteResponse(SignedVoteResponse {
                            vote: Some(vote),
                            error: None,
                        })),
                    }
                }
                Err(e) => PrivvalMessage {
                    sum: Some(PrivvalSum::SignedVoteResponse(SignedVoteResponse {
                        vote: None,
                        error: Some(RemoteSignerError {
                            code: 1,
                            description: e.to_string(),
                        }),
                    })),
                },
            }
        }
        Some(PrivvalSum::SignProposalRequest(req)) => {
            let chain = req.chain_id;
            let mut p = req.proposal.context("sign proposal missing")?;
            let bz = proposal_sign_bytes(&chain, &p)?;
            let fake = JsonReq {
                op: "sign".into(),
                msg_hex: hex::encode(&bz),
                height: p.height,
                round: p.round,
                step: 3,
            };
            match sign_gated(signer, hrs, &fake).await {
                Ok(sig) => {
                    p.signature = sig.to_vec();
                    PrivvalMessage {
                        sum: Some(PrivvalSum::SignedProposalResponse(SignedProposalResponse {
                            proposal: Some(p),
                            error: None,
                        })),
                    }
                }
                Err(e) => PrivvalMessage {
                    sum: Some(PrivvalSum::SignedProposalResponse(SignedProposalResponse {
                        proposal: None,
                        error: Some(RemoteSignerError {
                            code: 1,
                            description: e.to_string(),
                        }),
                    })),
                },
            }
        }
        _ => PrivvalMessage {
            sum: Some(PrivvalSum::PubKeyResponse(PubKeyResponse {
                pub_key: None,
                error: Some(RemoteSignerError {
                    code: 1,
                    description: "unknown".into(),
                }),
            })),
        },
    };
    encode_privval(&reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custody::frost::FrostCoordinator;
    use crate::transport::session::Cleartext;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn json_loopback_sign_and_hrs() {
        let coord = FrostCoordinator::from_dkg(3, 2).unwrap();
        let pk = coord.pubkey_bytes().to_vec();
        let signer: Arc<dyn ConsensusSigner> = Arc::new(coord);
        let dir = std::env::temp_dir().join(format!("pv-{}-{}", std::process::id(), "hrs"));
        let _ = std::fs::create_dir_all(&dir);
        let hrs = Arc::new(Mutex::new(HrsLock::load(&dir.join("hrs.json")).unwrap()));
        let lst = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = lst.local_addr().unwrap();
        let s2 = signer.clone();
        let h2 = hrs.clone();
        let sidecar = tokio::spawn(async move {
            let crypto = Cleartext;
            let _ = serve_once(&crypto, &addr.to_string(), s2, "json", Some(h2)).await;
        });
        let (mut tcp, _) = lst.accept().await.unwrap();
        let req = serde_json::json!({
            "op": "sign",
            "msg_hex": hex::encode(b"vote-bytes"),
            "height": 10,
            "round": 0,
            "step": 2
        });
        let body = serde_json::to_vec(&req).unwrap();
        tcp.write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        tcp.write_all(&body).await.unwrap();
        let mut lenb = [0u8; 4];
        tcp.read_exact(&mut lenb).await.unwrap();
        let n = u32::from_be_bytes(lenb) as usize;
        let mut buf = vec![0u8; n];
        tcp.read_exact(&mut buf).await.unwrap();
        let resp: JsonResp = serde_json::from_slice(&buf).unwrap();
        assert!(resp.error.is_none(), "{:?}", resp.error);
        let sig = hex::decode(resp.sig_hex.unwrap()).unwrap();
        assert_eq!(sig.len(), 64);
        let vk =
            ed25519_dalek::VerifyingKey::from_bytes(pk.as_slice().try_into().unwrap()).unwrap();
        vk.verify_strict(
            b"vote-bytes",
            &ed25519_dalek::Signature::from_bytes(sig.as_slice().try_into().unwrap()),
        )
        .unwrap();
        drop(tcp);
        let _ = sidecar.await;
    }

    #[tokio::test]
    async fn cmt_signvote_protobuf_loopback() {
        use crate::transport::cmt_privval::*;
        let coord = FrostCoordinator::from_dkg(3, 2).unwrap();
        let pk = coord.pubkey_bytes().to_vec();
        let signer: Arc<dyn ConsensusSigner> = Arc::new(coord);
        let lst = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = lst.local_addr().unwrap();
        let s2 = signer.clone();
        let sidecar = tokio::spawn(async move {
            let crypto = Cleartext;
            let _ = serve_once(&crypto, &addr.to_string(), s2, "cmt", None).await;
        });
        let (mut tcp, _) = lst.accept().await.unwrap();
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
        let want_bytes = vote_sign_bytes("test-1", &vote).unwrap();
        let req = PrivvalMessage {
            sum: Some(PrivvalSum::SignVoteRequest(SignVoteRequest {
                vote: Some(vote),
                chain_id: "test-1".into(),
            })),
        };
        let body = req.encode_to_vec();
        tcp.write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        tcp.write_all(&body).await.unwrap();
        let mut lenb = [0u8; 4];
        tcp.read_exact(&mut lenb).await.unwrap();
        let n = u32::from_be_bytes(lenb) as usize;
        let mut buf = vec![0u8; n];
        tcp.read_exact(&mut buf).await.unwrap();
        let reply = decode_privval(&buf)
            .or_else(|_| Ok::<_, anyhow::Error>(PrivvalMessage::decode(&buf[..])?))
            .unwrap();
        let PrivvalSum::SignedVoteResponse(r) = reply.sum.unwrap() else {
            panic!("expected signed vote");
        };
        let signed = r.vote.unwrap();
        assert_eq!(signed.signature.len(), 64);
        let vk =
            ed25519_dalek::VerifyingKey::from_bytes(pk.as_slice().try_into().unwrap()).unwrap();
        vk.verify_strict(
            &want_bytes,
            &ed25519_dalek::Signature::from_bytes(signed.signature.as_slice().try_into().unwrap()),
        )
        .unwrap();
        drop(tcp);
        let _ = sidecar.await;
    }
}
