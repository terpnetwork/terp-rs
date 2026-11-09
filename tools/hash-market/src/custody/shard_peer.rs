//! Local vs remote FROST shards. Coordinator talks this trait only.

use anyhow::Result;
use async_trait::async_trait;
use frost_ed25519 as frost;
use kms_shard::{ConsensusScheme, FrostEd25519};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Serialize, Deserialize)]
pub struct ShardReq {
    pub op: String,
    #[serde(default)]
    pub cmap_b64: BTreeMap<String, String>,
    #[serde(default)]
    pub msg_b64: String,
}

#[derive(Serialize, Deserialize)]
pub struct ShardResp {
    pub ok: bool,
    #[serde(default)]
    pub id: u16,
    #[serde(default)]
    pub commit_b64: String,
    #[serde(default)]
    pub partial_b64: String,
    #[serde(default)]
    pub error: String,
}

#[async_trait]
pub trait ShardPeer: Send + Sync {
    fn id(&self) -> u16;
    async fn commit(&self) -> Result<frost::round1::SigningCommitments>;
    async fn sign_share(
        &self,
        cmap: &BTreeMap<frost::Identifier, frost::round1::SigningCommitments>,
        msg: &[u8],
    ) -> Result<frost::round2::SignatureShare>;
}

/// In-process share (lab / single-host t-of-n).
pub struct LocalPeer {
    id: u16,
    share: frost::keys::KeyPackage,
    nonces: Mutex<Option<frost::round1::SigningNonces>>,
}

impl LocalPeer {
    pub fn new(id: u16, share: frost::keys::KeyPackage) -> Self {
        Self {
            id,
            share,
            nonces: Mutex::new(None),
        }
    }
}

#[async_trait]
impl ShardPeer for LocalPeer {
    fn id(&self) -> u16 {
        self.id
    }
    async fn commit(&self) -> Result<frost::round1::SigningCommitments> {
        let mut rng = OsRng;
        let (n, c) =
            FrostEd25519::commit(&self.share, &mut rng).map_err(|e| anyhow::anyhow!("{e}"))?;
        *self.nonces.lock().map_err(|e| anyhow::anyhow!("{e}"))? = Some(n);
        Ok(c)
    }
    async fn sign_share(
        &self,
        cmap: &BTreeMap<frost::Identifier, frost::round1::SigningCommitments>,
        msg: &[u8],
    ) -> Result<frost::round2::SignatureShare> {
        let n = self
            .nonces
            .lock()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .take()
            .ok_or_else(|| anyhow::anyhow!("no nonces"))?;
        FrostEd25519::sign_share(&self.share, n, cmap, msg).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// TCP JSON shard (another hash-market operator).
pub struct RemotePeer {
    id: u16,
    addr: String,
}

impl RemotePeer {
    pub fn new(id: u16, addr: String) -> Self {
        Self { id, addr }
    }
}

async fn rpc(addr: &str, req: &ShardReq) -> Result<ShardResp> {
    let mut s = TcpStream::connect(addr).await?;
    let body = serde_json::to_vec(req)?;
    s.write_all(&(body.len() as u32).to_be_bytes()).await?;
    s.write_all(&body).await?;
    let mut lenb = [0u8; 4];
    s.read_exact(&mut lenb).await?;
    let n = u32::from_be_bytes(lenb) as usize;
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

#[async_trait]
impl ShardPeer for RemotePeer {
    fn id(&self) -> u16 {
        self.id
    }
    async fn commit(&self) -> Result<frost::round1::SigningCommitments> {
        let resp = rpc(
            &self.addr,
            &ShardReq {
                op: "commit".into(),
                cmap_b64: BTreeMap::new(),
                msg_b64: String::new(),
            },
        )
        .await?;
        anyhow::ensure!(resp.ok, "{}", resp.error);
        let raw =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &resp.commit_b64)?;
        frost::round1::SigningCommitments::deserialize(&raw).map_err(|e| anyhow::anyhow!("{e}"))
    }
    async fn sign_share(
        &self,
        cmap: &BTreeMap<frost::Identifier, frost::round1::SigningCommitments>,
        msg: &[u8],
    ) -> Result<frost::round2::SignatureShare> {
        let mut cmap_b64 = BTreeMap::new();
        for id in 1u16..=64 {
            let ident = match kms_shard::identifier(id) {
                Ok(i) => i,
                Err(_) => continue,
            };
            if let Some(c) = cmap.get(&ident) {
                let bytes = c.serialize().map_err(|e| anyhow::anyhow!("{e}"))?;
                cmap_b64.insert(
                    id.to_string(),
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes),
                );
            }
        }
        let resp = rpc(
            &self.addr,
            &ShardReq {
                op: "sign".into(),
                cmap_b64,
                msg_b64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, msg),
            },
        )
        .await?;
        anyhow::ensure!(resp.ok, "{}", resp.error);
        let raw = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &resp.partial_b64,
        )?;
        frost::round2::SignatureShare::deserialize(&raw).map_err(|e| anyhow::anyhow!("{e}"))
    }
}
