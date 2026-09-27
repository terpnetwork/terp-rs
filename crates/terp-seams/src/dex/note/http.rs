//! Optional blocking HTTP client for HeadstashStore notes.
//!
//! Feature: `http` (reqwest + rustls). Does **not** implement a second blob
//! store — only auth-gated `/notes/{hs_id}/...` against hash-market.

use super::{validate_id, NoteEnvelope, NotePersistError};

/// PUT envelope JSON to hash-market HeadstashStore notes route.
///
/// - `base_url` — e.g. `http://127.0.0.1:8080` (no trailing slash required)
/// - `hs_id` / `addr` — path segments; validated with [`validate_id`]
/// - `envelope` — already encrypted (see [`super::encrypt_note_out`])
/// - `headers` — blossom (or other) auth headers as `(name, value)` pairs
///
/// Success: HTTP 2xx. Body is the envelope JSON.
pub fn put_note_envelope(
    base_url: &str,
    hs_id: &str,
    addr: &str,
    envelope: &NoteEnvelope,
    headers: &[(&str, &str)],
) -> Result<(), NotePersistError> {
    validate_id(hs_id)?;
    validate_id(addr)?;

    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/notes/{hs_id}/{addr}");

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    let mut req = client
        .put(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }

    let body = envelope
        .to_json_bytes()
        .map_err(|_| NotePersistError::Json)?;

    let resp = req
        .body(body)
        .send()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        return Err(NotePersistError::HttpStatus {
            status,
            body: text.chars().take(256).collect(),
        });
    }
    Ok(())
}

/// GET encrypted envelope from hash-market HeadstashStore notes route.
///
/// Primary recovery path for owner/snap (auth-gated). Does not decrypt.
pub fn get_note_envelope(
    base_url: &str,
    hs_id: &str,
    addr: &str,
    headers: &[(&str, &str)],
) -> Result<NoteEnvelope, NotePersistError> {
    validate_id(hs_id)?;
    validate_id(addr)?;

    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/notes/{hs_id}/{addr}");

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    let mut req = client.get(&url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }

    let resp = req
        .send()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        return Err(NotePersistError::HttpStatus {
            status,
            body: text.chars().take(256).collect(),
        });
    }

    let bytes = resp
        .bytes()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;
    NoteEnvelope::from_json_bytes(&bytes)
}

/// GET `/notes/{hs_id}` → list of note address keys (for snap / PIR setup).
pub fn list_note_keys(
    base_url: &str,
    hs_id: &str,
    headers: &[(&str, &str)],
) -> Result<Vec<String>, NotePersistError> {
    validate_id(hs_id)?;
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/notes/{hs_id}");

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    let mut req = client.get(&url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }

    let resp = req
        .send()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        return Err(NotePersistError::HttpStatus {
            status,
            body: text.chars().take(256).collect(),
        });
    }

    let v: serde_json::Value = resp
        .json()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;
    let keys = v
        .get("keys")
        .and_then(|k| k.as_array())
        .ok_or(NotePersistError::Json)?
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    Ok(keys)
}

/// POST `/notes/{hs_id}/pir` with one-hot selector → XOR result bytes (padded envelope JSON).
///
/// Snap recover path when hiding which `addr` is requested. Caller finds `addr`
/// index via [`list_note_keys`].
pub fn pir_fetch_note_blob(
    base_url: &str,
    hs_id: &str,
    selector: &[u8],
    headers: &[(&str, &str)],
) -> Result<Vec<u8>, NotePersistError> {
    validate_id(hs_id)?;
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/notes/{hs_id}/pir");

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    let body = serde_json::json!({ "selector": selector });
    let mut req = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }

    let resp = req
        .send()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        return Err(NotePersistError::HttpStatus {
            status,
            body: text.chars().take(256).collect(),
        });
    }

    let v: serde_json::Value = resp
        .json()
        .map_err(|e| NotePersistError::Http(e.to_string()))?;
    // Server returns hex result or raw — accept both.
    if let Some(hex_s) = v.get("result").and_then(|r| r.as_str()) {
        return hex::decode(hex_s).map_err(|_| NotePersistError::BadHex);
    }
    if let Some(arr) = v.get("result").and_then(|r| r.as_array()) {
        let mut out = Vec::with_capacity(arr.len());
        for x in arr {
            let b = x
                .as_u64()
                .ok_or(NotePersistError::Json)?
                .try_into()
                .map_err(|_| NotePersistError::Json)?;
            out.push(b);
        }
        return Ok(out);
    }
    Err(NotePersistError::Json)
}

/// List keys, build one-hot selector for `addr`, PIR-fetch, parse envelope.
///
/// Returns the padded JSON blob trimmed via envelope parse (padding zeros ok for JSON).
pub fn pir_get_note_envelope(
    base_url: &str,
    hs_id: &str,
    addr: &str,
    headers: &[(&str, &str)],
) -> Result<NoteEnvelope, NotePersistError> {
    validate_id(addr)?;
    let keys = list_note_keys(base_url, hs_id, headers)?;
    let idx = keys
        .iter()
        .position(|k| k == addr)
        .ok_or_else(|| NotePersistError::HttpStatus {
            status: 404,
            body: format!("addr {addr} not in note list"),
        })?;
    let mut selector = vec![0u8; keys.len()];
    selector[idx] = 1;
    let blob = pir_fetch_note_blob(base_url, hs_id, &selector, headers)?;
    // Strip trailing NUL pad from equal-length PIR blobs.
    let end = blob.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
    NoteEnvelope::from_json_bytes(&blob[..end])
}
