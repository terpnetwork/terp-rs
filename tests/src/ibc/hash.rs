//! IBC denom hash computation and path geometry helpers.

use sha2::{Digest, Sha256};

/// Compute the IBC denom hash from a trace path.
/// The trace path is the full route from the perspective of the destination chain,
/// e.g. `"transfer/channel-0/transfer/channel-1/uatom"`.
pub fn compute_ibc_denom_hash(trace_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(trace_path.as_bytes());
    let result = hasher.finalize();
    format!("ibc/{}", hex::encode(result).to_uppercase())
}

/// Count `transfer/` segments in a denom trace path (hop honesty invariant).
pub fn hop_count_from_trace_path(trace_path: &str) -> usize {
    trace_path.matches("transfer/").count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ibc_denom_hash_format() {
        let h = compute_ibc_denom_hash("transfer/channel-0/uterp");
        assert!(h.starts_with("ibc/"));
        assert_eq!(h.len(), 68);
    }

    #[test]
    fn test_known_akt_on_osmosis() {
        let h = compute_ibc_denom_hash("transfer/channel-1/uakt");
        assert_eq!(
            h,
            "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4"
        );
    }

    #[test]
    fn hop_count_matches_transfer_segments() {
        assert_eq!(hop_count_from_trace_path("transfer/channel-0/uterp"), 1);
        assert_eq!(
            hop_count_from_trace_path("transfer/channel-0/transfer/channel-1/uakt"),
            2
        );
        assert_eq!(hop_count_from_trace_path("uterp"), 0);
    }
}
