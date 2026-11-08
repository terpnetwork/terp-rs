//! Validator session crypto. Trait-shaped so Noise XX can replace SecretConnection
//! without rewriting privval framing.

use anyhow::Result;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// Encrypted (or clear) byte stream after handshake.
#[async_trait]
pub trait SecureConn: Send {
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()>;
    async fn write_all(&mut self, buf: &[u8]) -> Result<()>;
    async fn read_frame(&mut self) -> Result<Vec<u8>> {
        let mut lenb = [0u8; 4];
        self.read_exact(&mut lenb).await?;
        let n = u32::from_be_bytes(lenb) as usize;
        anyhow::ensure!(n <= 1_048_576, "frame too large");
        let mut b = vec![0u8; n];
        self.read_exact(&mut b).await?;
        Ok(b)
    }
    async fn write_frame(&mut self, payload: &[u8]) -> Result<()> {
        self.write_all(&(payload.len() as u32).to_be_bytes())
            .await?;
        self.write_all(payload).await
    }
}

/// How the sidecar authenticates to `priv_validator_laddr`.
#[async_trait]
pub trait SessionCrypto: Send + Sync {
    async fn upgrade_outbound(&self, raw: TcpStream) -> Result<Box<dyn SecureConn>>;
}

/// Test / lab: no handshake. Not for production consensus.
pub struct Cleartext;

#[async_trait]
impl SessionCrypto for Cleartext {
    async fn upgrade_outbound(&self, raw: TcpStream) -> Result<Box<dyn SecureConn>> {
        Ok(Box::new(Plain(raw)))
    }
}

struct Plain(TcpStream);

#[async_trait]
impl SecureConn for Plain {
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.0.read_exact(buf).await?;
        Ok(())
    }
    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.0.write_all(buf).await?;
        self.0.flush().await?;
        Ok(())
    }
}

pub async fn read_lp<R: AsyncRead + Unpin + Send>(r: &mut R) -> Result<Vec<u8>> {
    let mut lenb = [0u8; 4];
    r.read_exact(&mut lenb).await?;
    let n = u32::from_be_bytes(lenb) as usize;
    anyhow::ensure!(n <= 1_048_576, "frame too large");
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).await?;
    Ok(b)
}

pub async fn write_lp<W: AsyncWrite + Unpin + Send>(w: &mut W, payload: &[u8]) -> Result<()> {
    w.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}
