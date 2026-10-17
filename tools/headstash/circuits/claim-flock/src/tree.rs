//! Blake3 allocation tree.
//!
//! Depth 2 holds four leaves. The community set is 51,807 notes, so the
//! production tree is depth 16 (`2^16 = 65536`). `rseed` is an input to the
//! builder and is not a field on the published leaf. Callers that write the
//! tree to disk serialize [`PublicLeaf`] only.

use crate::{compress, merkle_leaf, node_message, rseed_message, FLAG_NODE, FLAG_RSEED};

/// Zero digest used to pad the tree up to `2^depth`. It is not a leaf preimage.
const EMPTY: [u32; 8] = [0u32; 8];

/// One note the registrar sampled. `rseed` never leaves this type into [`BuiltTree`].
pub struct SecretNote {
    pub rseed: [u32; 8],
    pub epk_x: [u8; 32],
    pub epk_y: [u8; 32],
    /// First 16 bytes of the eligible key, packed the way the claim circuit wires `epk_x`.
    pub epk_words: [u32; 4],
    pub nd: [u32; 2],
    pub v: u32,
    pub fdi: u32,
}

/// A leaf an observer of the tree is allowed to see.
/// The key, the denomination, the amount, and the piece index stay in the
/// registrar's secret notes. They are not on this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicLeaf {
    pub index: u32,
    pub rseed_com: [u32; 8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltTree {
    pub depth: u32,
    pub root: [u32; 8],
    pub leaves: Vec<PublicLeaf>,
}

/// Smallest depth that can hold `note_count` leaves, and at least 2.
///
/// Depth 2 is four leaves. 51,807 notes need 16.
pub fn depth_for(note_count: usize) -> u32 {
    assert!(note_count > 0, "a tree needs at least one note");
    let mut depth = 2u32;
    while (1usize << depth) < note_count {
        depth += 1;
        assert!(depth <= 32, "note count does not fit a Blake3 index");
    }
    depth
}

fn rseed_com(rseed: &[u32; 8]) -> [u32; 8] {
    compress(&rseed_message(rseed), FLAG_RSEED)
}

fn leaf_digest(note: &SecretNote, com: &[u32; 8]) -> [u32; 8] {
    merkle_leaf(com, &note.epk_words, &note.nd, note.v, note.fdi)
}

/// Build the padded tree. The returned leaves do not carry `rseed`.
pub fn build_tree(notes: &[SecretNote]) -> BuiltTree {
    let depth = depth_for(notes.len());
    let width = 1usize << depth;
    let mut level = vec![EMPTY; width];
    let mut leaves = Vec::with_capacity(notes.len());
    for (index, note) in notes.iter().enumerate() {
        let com = rseed_com(&note.rseed);
        level[index] = leaf_digest(note, &com);
        leaves.push(PublicLeaf {
            index: index as u32,
            rseed_com: com,
        });
    }
    for _ in 0..depth {
        let mut next = vec![EMPTY; level.len() / 2];
        for (i, slot) in next.iter_mut().enumerate() {
            *slot = compress(&node_message(&level[2 * i], &level[2 * i + 1]), FLAG_NODE);
        }
        level = next;
    }
    BuiltTree {
        depth,
        root: level[0],
        leaves,
    }
}

/// Sibling digests from the leaf up to the root, matching [`crate::Opening::siblings`].
pub fn siblings(notes: &[SecretNote], index: usize) -> (BuiltTree, Vec<[u32; 8]>) {
    let depth = depth_for(notes.len()) as usize;
    let width = 1usize << depth;
    assert!(index < notes.len() && index < width);
    let mut level = vec![EMPTY; width];
    for (i, note) in notes.iter().enumerate() {
        let com = rseed_com(&note.rseed);
        level[i] = leaf_digest(note, &com);
    }
    let mut path = Vec::with_capacity(depth);
    let mut cursor = index;
    let mut current = level;
    for _ in 0..depth {
        let sibling = cursor ^ 1;
        path.push(current[sibling]);
        let mut next = vec![EMPTY; current.len() / 2];
        for (i, slot) in next.iter_mut().enumerate() {
            *slot = compress(
                &node_message(&current[2 * i], &current[2 * i + 1]),
                FLAG_NODE,
            );
        }
        current = next;
        cursor /= 2;
    }
    let tree = build_tree(notes);
    (tree, path)
}

/// Pack the first 16 bytes of a 32-byte key as four big-endian words.
pub fn epk_words(epk_x: &[u8; 32]) -> [u32; 4] {
    let mut out = [0u32; 4];
    for i in 0..4 {
        out[i] = u32::from_be_bytes(epk_x[4 * i..4 * i + 4].try_into().unwrap());
    }
    out
}

pub fn words_from_le_bytes(bytes: &[u8; 32]) -> [u32; 8] {
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
    }
    out
}

#[cfg(test)]
fn public_json(leaf: &PublicLeaf) -> String {
    format!(
        "{{\"index\":{},\"rseed_com\":{:?}}}",
        leaf.index, leaf.rseed_com
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(rseed0: u32, v: u32) -> SecretNote {
        let mut rseed = [0u32; 8];
        rseed[0] = rseed0;
        let mut epk_x = [0u8; 32];
        epk_x[31] = rseed0 as u8;
        SecretNote {
            rseed,
            epk_x,
            epk_y: [2u8; 32],
            epk_words: epk_words(&epk_x),
            nd: [3, 4],
            v,
            fdi: 0,
        }
    }

    #[test]
    fn depth_matches_the_eligible_set() {
        assert_eq!(depth_for(1), 2);
        assert_eq!(depth_for(4), 2);
        assert_eq!(depth_for(5), 3);
        assert_eq!(depth_for(32_768), 15);
        assert_eq!(depth_for(32_769), 16);
        assert_eq!(depth_for(51_807), 16);
    }

    #[test]
    fn published_leaf_omits_rseed_and_a_second_string_moves_the_root() {
        let secret = 0xA1B2_C3D4;
        let notes = [note(secret, 15), note(2, 15), note(3, 30), note(4, 15)];
        let tree = build_tree(&notes);
        assert_eq!(tree.depth, 2);
        assert_eq!(tree.leaves.len(), 4);
        let json = public_json(&tree.leaves[0]);
        assert!(!json.contains("\"rseed\""));
        assert!(!json.contains("epk"));
        assert!(!json.contains("\"v\""));
        assert!(!json.contains(&format!("{secret:08x}")));
        assert_ne!(tree.leaves[0].rseed_com, notes[0].rseed);

        let mut rerolled = notes;
        rerolled[0].rseed[0] ^= 1;
        let other = build_tree(&rerolled);
        assert_ne!(other.root, tree.root);
        assert_ne!(other.leaves[0].rseed_com, tree.leaves[0].rseed_com);
    }

    #[test]
    fn fifty_one_thousand_synthetic_notes_fit_at_depth_16() {
        let notes: Vec<_> = (0..51_807).map(|i| note(i as u32, 15)).collect();
        let tree = build_tree(&notes);
        assert_eq!(tree.depth, 16);
        assert_eq!(tree.leaves.len(), 51_807);
        assert_ne!(tree.root, [0u32; 8]);
        let json = public_json(&tree.leaves[0]);
        assert!(!json.contains("\"rseed\""));
        assert!(!json.contains("epk"));
        assert!(!json.contains("\"v\""));
    }

    #[test]
    fn real_eligible_set_publishes_no_rseed() {
        let Some(path) = notes_file() else {
            // The community file is gitignored because it holds rseed.
            return;
        };
        let file = std::fs::File::open(&path).expect("open community notes");
        let value: serde_json::Value = serde_json::from_reader(file).expect("parse community notes");
        let raw = value["notes"].as_array().expect("notes array");
        assert_eq!(raw.len(), 51_807);
        let mut notes = Vec::with_capacity(raw.len());
        let mut first_secret = String::new();
        for (i, note) in raw.iter().enumerate() {
            let rseed_hex = note["rseed"].as_str().expect("rseed").to_string();
            if i == 0 {
                first_secret = rseed_hex.trim_start_matches("0x").to_ascii_lowercase();
            }
            let rseed_vec = decode_hex(&rseed_hex);
            let rseed_bytes: [u8; 32] = rseed_vec.try_into().expect("rseed width");
            let epk_x = decode_fixed::<32>(note["epk_x"].as_str().expect("epk_x"));
            let epk_y = decode_fixed::<32>(note["epk_y"].as_str().expect("epk_y"));
            let nd_bytes = decode_hex(note["nd"].as_str().expect("nd"));
            notes.push(SecretNote {
                rseed: words_from_le_bytes(&rseed_bytes),
                epk_words: epk_words(&epk_x),
                epk_x,
                epk_y,
                nd: [
                    u32::from_be_bytes(nd_bytes[0..4].try_into().unwrap()),
                    u32::from_be_bytes(nd_bytes[4..8].try_into().unwrap()),
                ],
                v: note["v"].as_u64().expect("v") as u32,
                fdi: note["fdi"].as_u64().expect("fdi") as u32,
            });
        }
        drop(value);
        let tree = build_tree(&notes);
        assert_eq!(tree.depth, 16);
        assert_eq!(tree.leaves.len(), 51_807);
        let json = public_json(&tree.leaves[0]);
        assert!(!json.contains("\"rseed\""));
        assert!(!json.contains("epk"));
        assert!(!json.contains("\"v\""));
        assert!(!json.contains(&first_secret));
        assert!(!first_secret.is_empty());
    }

    fn notes_file() -> Option<std::path::PathBuf> {
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for _ in 0..6 {
            let candidate = dir.join("artifacts/community-notes/notes.json");
            if candidate.is_file() {
                return Some(candidate);
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    fn decode_hex(s: &str) -> Vec<u8> {
        let s = s.trim().trim_start_matches("0x");
        assert!(s.len() % 2 == 0, "hex length");
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex"))
            .collect()
    }

    fn decode_fixed<const N: usize>(s: &str) -> [u8; N] {
        let bytes = decode_hex(s);
        bytes.try_into().expect("fixed hex width")
    }
}
