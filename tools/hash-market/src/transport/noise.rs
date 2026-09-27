//! Noise XX (snow) — latest modular session crypto for the sidecar.
//! K_noise is the static key here, never K_cons.

use anyhow::{Context, Result};
use async_trait::async_trait;
use snow::{Builder, TransportState};

use tokio::net::TcpStream;

use super::session::{read_lp, write_lp, SecureConn, SessionCrypto};

const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

pub struct NoiseXx {
    local: snow::Keypair,
}

impl NoiseXx {
    pub fn generate() -> Result<Self> {
        let builder = Builder::new(PATTERN.parse().context("noise pattern")?);
        Ok(Self {
            local: builder.generate_keypair().context("noise keypair")?,
        })
    }

    pub fn from_private(sk32: &[u8]) -> Result<Self> {
        anyhow::ensure!(sk32.len() == 32, "noise static key is 32 bytes");
        let params = PATTERN.parse().context("noise pattern")?;
        let local = Builder::new(params)
            .local_private_key(sk32)
            .generate_keypair()
            .context("noise from static")?;
        Ok(Self { local })
    }

    pub fn public(&self) -> &[u8] {
        &self.local.public
    }
}

struct NoiseConn {
    tcp: TcpStream,
    ts: TransportState,
    leftover: Vec<u8>,
}

impl NoiseConn {
    fn new(tcp: TcpStream, ts: TransportState) -> Self {
        Self {
            tcp,
            ts,
            leftover: Vec::new(),
        }
    }
}

#[async_trait]
impl SecureConn for NoiseConn {
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        while self.leftover.len() < buf.len() {
            let ct = read_lp(&mut self.tcp).await?;
            let mut pt = vec![0u8; ct.len()];
            let n = self
                .ts
                .read_message(&ct, &mut pt)
                .map_err(|e| anyhow::anyhow!("noise read: {e}"))?;
            self.leftover.extend_from_slice(&pt[..n]);
        }
        buf.copy_from_slice(&self.leftover[..buf.len()]);
        self.leftover.drain(..buf.len());
        Ok(())
    }
    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut ct = vec![0u8; buf.len() + 32];
        let n = self
            .ts
            .write_message(buf, &mut ct)
            .map_err(|e| anyhow::anyhow!("noise write: {e}"))?;
        write_lp(&mut self.tcp, &ct[..n]).await
    }
    async fn read_frame(&mut self) -> Result<Vec<u8>> {
        let ct = read_lp(&mut self.tcp).await?;
        let mut pt = vec![0u8; ct.len()];
        let n = self
            .ts
            .read_message(&ct, &mut pt)
            .map_err(|e| anyhow::anyhow!("noise read: {e}"))?;
        pt.truncate(n);
        Ok(pt)
    }
    async fn write_frame(&mut self, payload: &[u8]) -> Result<()> {
        self.write_all(payload).await
    }
}

#[async_trait]
impl SessionCrypto for NoiseXx {
    async fn upgrade_outbound(&self, mut raw: TcpStream) -> Result<Box<dyn SecureConn>> {
        let mut hs = Builder::new(PATTERN.parse()?)
            .local_private_key(&self.local.private)
            .build_initiator()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut buf = vec![0u8; 1024];
        // -> e
        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        write_lp(&mut raw, &buf[..n]).await?;
        // <- e, ee, s, es
        let m = read_lp(&mut raw).await?;
        hs.read_message(&m, &mut buf)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        // -> s, se
        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        write_lp(&mut raw, &buf[..n]).await?;
        let ts = hs
            .into_transport_mode()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Box::new(NoiseConn::new(raw, ts)))
    }
}

/// Responder side for tests / validator-shaped loopback.
pub async fn noise_accept(
    local: &snow::Keypair,
    mut raw: TcpStream,
) -> Result<Box<dyn SecureConn>> {
    let mut hs = Builder::new(PATTERN.parse()?)
        .local_private_key(&local.private)
        .build_responder()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut buf = vec![0u8; 1024];
    let m = read_lp(&mut raw).await?;
    hs.read_message(&m, &mut buf)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let n = hs
        .write_message(&[], &mut buf)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    write_lp(&mut raw, &buf[..n]).await?;
    let m = read_lp(&mut raw).await?;
    hs.read_message(&m, &mut buf)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let ts = hs
        .into_transport_mode()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Box::new(NoiseConn::new(raw, ts)))
}

impl NoiseXx {
    pub fn keypair(&self) -> &snow::Keypair {
        &self.local
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::session::SessionCrypto;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn noise_xx_roundtrip() {
        let client = NoiseXx::generate().unwrap();
        let server = NoiseXx::generate().unwrap();
        let lst = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = lst.local_addr().unwrap();
        let sk = snow::Keypair {
            public: server.keypair().public.clone(),
            private: server.keypair().private.clone(),
        };
        let h = tokio::spawn(async move {
            let (tcp, _) = lst.accept().await.unwrap();
            let mut s = noise_accept(&sk, tcp).await.unwrap();
            let got = s.read_frame().await.unwrap();
            assert_eq!(got, b"ping");
            s.write_frame(b"pong").await.unwrap();
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut c = client.upgrade_outbound(tcp).await.unwrap();
        c.write_frame(b"ping").await.unwrap();
        let got = c.read_frame().await.unwrap();
        assert_eq!(got, b"pong");
        h.await.unwrap();
    }
}
