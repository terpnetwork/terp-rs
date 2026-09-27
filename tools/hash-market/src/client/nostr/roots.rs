//! kind:30070 root event — represents a confirmed foreign-chain Merkle root.
//!
//! The root event is an addressable Nostr event (3xxxx range). The `d` tag
//! uniquely identifies the root by `{chain_uid}:{algo}:{height}`.

use crate::client::nostr::HashMerchantPayload;

/// Build the `d` tag identifier for a root event.
pub fn root_d_tag(chain_uid: &str, algo: &str, height: u64) -> String {
    format!("{}:{}:{}", chain_uid, algo, height)
}

/// Build tags for a kind:30070 root event from a payload.
pub fn build_root_tags(
    payload: &HashMerchantPayload,
    prev_event_id: Option<&str>,
    webhook_url: Option<&str>,
    bud_cid: Option<&str>,
) -> Vec<Vec<String>> {
    let mut tags: Vec<Vec<String>> = Vec::new();

    // d-tag — addressable identifier
    let d = root_d_tag(&payload.chain_uid, &payload.algo, payload.height);
    tags.push(vec!["d".to_string(), d]);

    // chain tag
    tags.push(vec!["chain".to_string(), payload.chain_uid.clone()]);

    // algo tag
    tags.push(vec!["algo".to_string(), payload.algo.clone()]);

    // height tag
    tags.push(vec!["height".to_string(), payload.height.to_string()]);

    // root (hex-encoded)
    tags.push(vec!["root".to_string(), hex::encode(&payload.root)]);

    // attestation count
    tags.push(vec![
        "attestations".to_string(),
        payload.attestation_count.to_string(),
    ]);

    // prev root event ID (if known)
    if let Some(prev) = prev_event_id {
        tags.push(vec!["e".to_string(), prev.to_string()]);
    }

    // webhook URL (if configured)
    if let Some(wh) = webhook_url {
        tags.push(vec!["webhook".to_string(), wh.to_string()]);
    }

    // BUD CID (if available)
    if let Some(cid) = bud_cid {
        tags.push(vec!["bud".to_string(), cid.to_string()]);
    }

    tags
}

/// Build the content JSON string for a root event.
pub fn build_root_content(payload: &HashMerchantPayload, cid: Option<&str>) -> String {
    let mut content = serde_json::json!({
        "block_time": payload.block_time,
    });
    if let Some(c) = cid {
        content["cid"] = serde_json::json!(c);
    }
    content.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_d_tag() {
        assert_eq!(
            root_d_tag("ethereum", "keccak256", 12345),
            "ethereum:keccak256:12345"
        );
    }

    #[test]
    fn test_build_root_tags() {
        let payload = HashMerchantPayload {
            chain_uid: "ethereum".to_string(),
            algo: "keccak256".to_string(),
            height: 100,
            root: hex::decode("abcdef123456").unwrap(),
            attestation_count: 3,
            block_time: 1234567890,
        };

        let tags = build_root_tags(&payload, None, None, None);
        assert!(tags
            .iter()
            .any(|t| t.len() >= 2 && t[0] == "d" && t[1].contains("ethereum")));
    }
}
