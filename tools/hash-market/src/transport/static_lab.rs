//! Lab / e2e transport: emit a fixed `VoteExtensionHashData` on an interval.
//!
//! Used by ICT L3 full-runtime B5 and local demos — **not** for production
//! foreign-chain ingestion. Prefer `http_poll` / `grpc` / Anvil client for real feeds.

use crate::msg::VoteExtensionHashData;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

pub async fn run(
    data: VoteExtensionHashData,
    interval_secs: u64,
    tx: mpsc::Sender<VoteExtensionHashData>,
) {
    let interval = Duration::from_secs(interval_secs.max(1));
    // Immediate first emit so /vote-extension is not 503 on startup.
    loop {
        if tx.send(data.clone()).await.is_err() {
            break;
        }
        sleep(interval).await;
    }
}
