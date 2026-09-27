//! Trivial XOR-based Private Information Retrieval (PIR).
//!
//! The client sends a binary selector vector (one byte per item in the
//! database). The server XORs all items whose selector bit is 1 and
//! returns the result. For a trivial PIR the selector has exactly one
//! bit set, but the server cannot distinguish this from a random vector.

/// Perform XOR PIR over a set of equal-length blobs.
///
/// `blobs` must all be the same length. `selector` must have exactly
/// `blobs.len()` elements (each 0 or 1).
///
/// Returns the XOR accumulation of selected blobs.
pub fn xor_pir(blobs: &[&[u8]], selector: &[u8]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        blobs.len() == selector.len(),
        "selector length ({}) != blobs count ({})",
        selector.len(),
        blobs.len()
    );

    if blobs.is_empty() {
        return Ok(Vec::new());
    }

    let blob_len = blobs[0].len();
    let mut result = vec![0u8; blob_len];

    for (i, blob) in blobs.iter().enumerate() {
        anyhow::ensure!(
            blob.len() == blob_len,
            "blob {i} has length {} but expected {blob_len}",
            blob.len()
        );

        if selector[i] != 0 {
            for (r, &b) in result.iter_mut().zip(blob.iter()) {
                *r ^= b;
            }
        }
    }

    Ok(result)
}

/// Poseidon-v1 auth paths + genesis `notes.json` ingest. Not compiled into the default blossom host.
#[cfg(feature = "pir")]
mod poseidon;
#[cfg(feature = "pir")]
mod ingest;
#[cfg(feature = "pir")]
pub use ingest::{
    ingest_community_notes, load_pir_blobs, load_pir_index, PirIndex, PirIngestReport,
    PublicLeafBlob,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_selection() {
        let a = b"hello world!";
        let b = b"goodbye!!!!!";
        let blobs: Vec<&[u8]> = vec![a, b];

        // Select only blob 0
        let result = xor_pir(&blobs, &[1, 0]).unwrap();
        assert_eq!(result, a.to_vec());

        // Select only blob 1
        let result = xor_pir(&blobs, &[0, 1]).unwrap();
        assert_eq!(result, b.to_vec());
    }

    #[test]
    fn xor_both() {
        let a = [0xAA; 4];
        let b = [0x55; 4];
        let blobs: Vec<&[u8]> = vec![&a, &b];
        let result = xor_pir(&blobs, &[1, 1]).unwrap();
        assert_eq!(result, vec![0xFF; 4]);
    }

    #[test]
    fn empty() {
        let result = xor_pir(&[], &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn length_mismatch() {
        let a = b"aaa";
        let blobs: Vec<&[u8]> = vec![a];
        assert!(xor_pir(&blobs, &[1, 0]).is_err());
    }
}
