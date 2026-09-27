//! Transport listeners for receiving hash data from various sources.
//!
//! Each transport mode runs in its own task and pushes `VoteExtensionHashData`
//! messages through an mpsc channel.

#[cfg(feature = "privval")]
pub mod cmt_privval;
pub mod grpc;
pub mod http_poll;
#[cfg(feature = "noise")]
pub mod noise;
#[cfg(feature = "privval")]
pub mod privval;
pub mod session;
pub mod static_lab;
pub mod websocket;

use crate::msg::VoteExtensionHashData;
use tokio::sync::mpsc;

/// Supported transport modes for receiving hash data.
#[derive(Debug, Clone)]
pub enum TransportMode {
    /// gRPC listener (CometBFT ABCI sidecar pattern)
    Grpc { listen_addr: String },
    /// HTTP polling against a remote endpoint
    HttpPoll { url: String, interval_secs: u64 },
    /// WebSocket subscription
    WebSocket { url: String },
    /// Lab/e2e only: re-emit a fixed VoteExtensionHashData (no external feeder).
    Static {
        data: VoteExtensionHashData,
        interval_secs: u64,
    },
}

/// Start a transport listener, returning a receiver for incoming hash data.
///
/// The transport runs in a background tokio task.
pub fn start(mode: TransportMode) -> mpsc::Receiver<VoteExtensionHashData> {
    let (tx, rx) = mpsc::channel(256);

    match mode {
        TransportMode::Grpc { listen_addr } => {
            tokio::spawn(grpc::run(listen_addr, tx));
        }
        TransportMode::HttpPoll { url, interval_secs } => {
            tokio::spawn(http_poll::run(url, interval_secs, tx));
        }
        TransportMode::WebSocket { url } => {
            tokio::spawn(websocket::run(url, tx));
        }
        TransportMode::Static {
            data,
            interval_secs,
        } => {
            tokio::spawn(static_lab::run(data, interval_secs, tx));
        }
    }

    rx
}
