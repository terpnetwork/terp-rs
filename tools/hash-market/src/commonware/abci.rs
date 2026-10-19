//! ABCI transport lanes using commonware-abci.
//!
//! Provides a unified mapping of commonwares ABCI server that integrates with the hash-market
//! sidecar transport layer.

use commonware_abci::v038::codec::{Decode, Encode};
use commonware_abci::v038::server::{
    ConsensusService, InfoService, MempoolService, Server2, SnapshotService,
};
use std::convert::TryFrom;
use tendermint::v0_38::abci::{
    ConsensusRequest, ConsensusResponse, InfoRequest, InfoResponse, MempoolRequest,
    MempoolResponse, Request, Response,
};
use tokio::net::TcpListener;
use tokio_util::codec::{FramedRead, FramedWrite};

/// Build a commonware ABCI server from handler functions.
pub struct AbciServer {
    consensus: Option<ConsensusService>,
    mempool: Option<MempoolService>,
    info: Option<InfoService>,
    snapshot: Option<SnapshotService>,
}

impl AbciServer {
    pub fn new() -> Self {
        Self {
            consensus: None,
            mempool: None,
            info: None,
            snapshot: None,
        }
    }

    pub fn with_consensus<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(ConsensusRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<
                Output = Result<ConsensusResponse, Box<dyn std::error::Error + Send + Sync>>,
            > + Send
            + 'static,
    {
        self.consensus = Some(ConsensusService::new(|req| {
            Box::pin(async move { f(req).await })
        }));
        self
    }

    pub fn with_mempool<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(MempoolRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<
                Output = Result<MempoolResponse, Box<dyn std::error::Error + Send + Sync>>,
            > + Send
            + 'static,
    {
        self.mempool = Some(MempoolService::new(|req| {
            Box::pin(async move { f(req).await })
        }));
        self
    }

    pub fn with_info<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(InfoRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<
                Output = Result<InfoResponse, Box<dyn std::error::Error + Send + Sync>>,
            > + Send
            + 'static,
    {
        self.info = Some(InfoService::new(|req| {
            Box::pin(async move { f(req).await })
        }));
        self
    }

    pub async fn serve(self, addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let consensus = self.consensus.ok_or("consensus handler required")?;
        let mempool = self.mempool.ok_or("mempool handler required")?;
        let info = self.info.ok_or("info handler required")?;
        let snapshot = self.snapshot.unwrap_or_else(|| {
            SnapshotService::new(|_| {
                Box::pin(async { Ok(tendermint::v0_38::abci::SnapshotResponse::default()) })
            })
        });

        let server = Server2::builder()
            .consensus(consensus)
            .mempool(mempool)
            .info(info)
            .snapshot(snapshot)
            .finish()
            .ok_or("failed to build server")?;

        server.listen_tcp(addr).await
    }
}

impl Default for AbciServer {
    fn default() -> Self {
        Self::new()
    }
}
