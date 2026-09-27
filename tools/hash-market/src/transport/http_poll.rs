//! HTTP polling transport.
//!
//! Periodically fetches `VoteExtensionHashData` from a remote HTTP endpoint
//! and pushes it through the channel.

use crate::msg::VoteExtensionHashData;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

pub async fn run(url: String, interval_secs: u64, tx: mpsc::Sender<VoteExtensionHashData>) {
    let interval = Duration::from_secs(interval_secs);

    loop {
        match fetch_data(&url).await {
            Ok(Some(data)) => {
                if tx.send(data).await.is_err() {
                    break;
                }
            }
            Ok(None) => {} // no new data
            Err(e) => {
                eprintln!("http_poll transport: {e}");
            }
        }
        sleep(interval).await;
    }
}

async fn fetch_data(url: &str) -> anyhow::Result<Option<VoteExtensionHashData>> {
    // Uses a simple TCP GET since reqwest may not be available under
    // the transport feature alone. When the server feature is active,
    // reqwest is available and this could be upgraded.
    //
    // For now, use a basic HTTP/1.1 request via tokio TCP.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let url_parsed: Vec<&str> = url
        .strip_prefix("http://")
        .unwrap_or(url)
        .splitn(2, '/')
        .collect();

    let host = url_parsed[0];
    let path = if url_parsed.len() > 1 {
        format!("/{}", url_parsed[1])
    } else {
        "/".to_string()
    };

    let mut stream = TcpStream::connect(host).await?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;

    // Find the body after \r\n\r\n
    let body_start = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);

    match body_start {
        Some(start) if start < buf.len() => {
            let data = VoteExtensionHashData::decode(&buf[start..])?;
            Ok(Some(data))
        }
        _ => Ok(None),
    }
}
