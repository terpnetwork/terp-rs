//! Tiny HTTP/1.1 JSON client (no curl, no ureq).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

fn split_host_path(url: &str) -> Option<(bool, String, u16, String)> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else {
        (false, url.strip_prefix("http://")?)
    };
    let (hp, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = if path.is_empty() {
        "/".into()
    } else {
        format!("/{path}")
    };
    let (host, port) = if let Some((h, p)) = hp.rsplit_once(':') {
        (h.to_string(), p.parse().ok()?)
    } else {
        (hp.to_string(), if tls { 443 } else { 80 })
    };
    Some((tls, host, port, path))
}

fn read_http_body(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if buf.len() > 2_000_000 {
            break;
        }
    }
    let s = String::from_utf8_lossy(&buf);
    if let Some(i) = s.find("\r\n\r\n") {
        Ok(s[i + 4..].as_bytes().to_vec())
    } else {
        Ok(buf)
    }
}

pub fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let (tls, host, port, path) = split_host_path(url).ok_or("url")?;
    if tls {
        return Err("https not used for local LCD/RPC".into());
    }
    let mut stream = TcpStream::connect((host.as_str(), port)).map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .ok();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .ok();
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let body = read_http_body(&mut stream)?;
    serde_json::from_slice(&body).map_err(|e| format!("json: {e}"))
}

pub fn post_json(url: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    post_json_timeout(url, body, Duration::from_secs(8), Duration::from_secs(8))
}

/// JSON POST with connect + I/O deadlines (ZEC tip probe must fail closed fast).
pub fn post_json_timeout(
    url: &str,
    body: &serde_json::Value,
    write: Duration,
    read: Duration,
) -> Result<serde_json::Value, String> {
    post_json_timeout_auth(url, body, write, read, None)
}

/// Same as [`post_json_timeout`] with an optional `Authorization` header value
/// (already formatted, e.g. `Basic …`).
pub fn post_json_timeout_auth(
    url: &str,
    body: &serde_json::Value,
    write: Duration,
    read: Duration,
    authorization: Option<&str>,
) -> Result<serde_json::Value, String> {
    let (tls, host, port, path) = split_host_path(url).ok_or("url")?;
    if tls {
        return Err("https not used for local LCD/RPC".into());
    }
    let payload = body.to_string();
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| format!("dns: {host}"))?;
    let mut stream =
        TcpStream::connect_timeout(&addr, write.min(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(write)).ok();
    stream.set_read_timeout(Some(read)).ok();
    let auth = authorization
        .filter(|s| !s.is_empty())
        .map(|s| format!("Authorization: {s}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "POST {path} HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let resp = read_http_body(&mut stream)?;
    serde_json::from_slice(&resp).map_err(|e| format!("rpc json: {e}"))
}
