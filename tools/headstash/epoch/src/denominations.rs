//! Rev 5 fixed-denomination eligibility: one leaf per note, depth 17.
//!
//! - note leaf: `sha256("terp-hs-note-elig-v2" || id_tag || identity || denom_class u8 || note_index u16_le)`
//!   with `id_tag` 0x01 (EVM, 20-byte address) or 0x02 (Solana, 32-byte pubkey).
//!   The amount is NOT in the leaf: it is `DENOMS_UTERP[denom_class]` from the campaign table.
//! - node: `sha256("terp-hs-note-node-v2" || left || right)`; padding leaf = 32 zero bytes.
//! - leaf order: identities sorted by `Identity::key()`, then each identity's notes by
//!   `note_index`; notes are numbered largest class first (greedy order).
//! - decomposition: greedy over the 1-2-5 series 1..5,000 TERP. The sub-1-TERP
//!   remainder of each identity is NOT allocated (reported as `remainder_uterp`).

use sha2::{Digest, Sha256};

use crate::eligibility::Identity;

pub const NOTE_TREE_DEPTH: u32 = 17;
pub const MAX_NOTES_PER_IDENTITY: usize = 256;
pub const EVM_ID_TAG: u8 = 0x01;
pub const SOL_ID_TAG: u8 = 0x02;

/// Must equal `zk_headstash::note_membership::DENOMS_UTERP`.
pub const DENOMS_UTERP: [u64; 12] = [
    1_000_000, 2_000_000, 5_000_000, 10_000_000, 20_000_000, 50_000_000, 100_000_000,
    200_000_000, 500_000_000, 1_000_000_000, 2_000_000_000, 5_000_000_000,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub class: u8,
    pub note_index: u16,
}

impl Note {
    pub fn amount(&self) -> u64 {
        DENOMS_UTERP[self.class as usize]
    }
}

/// Greedy 1-2-5 decomposition. Returns the notes (largest first) and the remainder.
pub fn decompose(amount: u64) -> (Vec<Note>, u64) {
    let mut rest = amount;
    let mut notes = Vec::new();
    for class in (0..DENOMS_UTERP.len()).rev() {
        let d = DENOMS_UTERP[class];
        while rest >= d {
            rest -= d;
            notes.push(Note { class: class as u8, note_index: notes.len() as u16 });
        }
    }
    (notes, rest)
}

pub fn note_leaf(id: &Identity, note: &Note) -> [u8; 32] {
    let mut pre = b"terp-hs-note-elig-v2".to_vec();
    match id {
        Identity::Evm(a) => {
            pre.push(EVM_ID_TAG);
            pre.extend_from_slice(a);
        }
        Identity::Sol(p) => {
            pre.push(SOL_ID_TAG);
            pre.extend_from_slice(p);
        }
    }
    pre.push(note.class);
    pre.extend_from_slice(&note.note_index.to_le_bytes());
    Sha256::digest(&pre).into()
}

pub fn note_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-note-node-v2".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    Sha256::digest(&pre).into()
}

pub fn open_note(leaf: [u8; 32], index: usize, path: &[[u8; 32]]) -> [u8; 32] {
    let mut n = leaf;
    for (l, s) in path.iter().enumerate() {
        n = if (index >> l) & 1 == 0 { note_node(&n, s) } else { note_node(s, &n) };
    }
    n
}

pub struct IdentityNotes {
    pub identity: Identity,
    pub allocation: u64,
    pub notes: Vec<Note>,
    /// Global leaf index of `notes[0]`; notes are contiguous.
    pub first_leaf: usize,
    pub remainder: u64,
}

pub struct NoteTree {
    pub ids: Vec<IdentityNotes>,
    levels: Vec<Vec<[u8; 32]>>,
}

impl NoteTree {
    /// `entries` must already be aggregated per identity (e.g. `EligTree::entries`).
    pub fn build(entries: &[(Identity, u64)]) -> Result<Self, String> {
        let mut sorted: Vec<(Identity, u64)> = entries.to_vec();
        sorted.sort_by_key(|(id, _)| id.key());
        for w in sorted.windows(2) {
            if w[0].0.key() == w[1].0.key() {
                return Err(format!("duplicate identity {}", w[0].0.key()));
            }
        }
        let mut ids = Vec::with_capacity(sorted.len());
        let mut leaves = Vec::new();
        for (id, amount) in sorted {
            let (notes, remainder) = decompose(amount);
            if notes.len() > MAX_NOTES_PER_IDENTITY {
                return Err(format!("{} has {} notes > {MAX_NOTES_PER_IDENTITY}", id.key(), notes.len()));
            }
            let first_leaf = leaves.len();
            leaves.extend(notes.iter().map(|n| note_leaf(&id, n)));
            ids.push(IdentityNotes { identity: id, allocation: amount, notes, first_leaf, remainder });
        }
        let cap = 1usize << NOTE_TREE_DEPTH;
        if leaves.len() > cap {
            return Err(format!("{} notes exceed depth {NOTE_TREE_DEPTH} ({cap})", leaves.len()));
        }
        leaves.resize(cap, [0u8; 32]);
        let mut levels = vec![leaves];
        for _ in 0..NOTE_TREE_DEPTH {
            let next = levels.last().unwrap().chunks(2).map(|p| note_node(&p[0], &p[1])).collect();
            levels.push(next);
        }
        Ok(Self { ids, levels })
    }

    pub fn root(&self) -> [u8; 32] {
        self.levels[NOTE_TREE_DEPTH as usize][0]
    }

    pub fn note_count(&self) -> usize {
        self.ids.iter().map(|i| i.notes.len()).sum()
    }

    pub fn path(&self, index: usize) -> Vec<[u8; 32]> {
        (0..NOTE_TREE_DEPTH as usize).map(|l| self.levels[l][(index >> l) ^ 1]).collect()
    }

    pub fn leaf(&self, index: usize) -> [u8; 32] {
        self.levels[0][index]
    }

    /// (notes, holders) per class.
    pub fn class_table(&self) -> Vec<(u64, u64)> {
        let mut t = vec![(0u64, 0u64); DENOMS_UTERP.len()];
        for i in &self.ids {
            let mut seen = [false; 12];
            for n in &i.notes {
                t[n.class as usize].0 += 1;
                seen[n.class as usize] = true;
            }
            for (c, s) in seen.iter().enumerate() {
                if *s {
                    t[c].1 += 1;
                }
            }
        }
        t
    }

    pub fn notes_uterp(&self) -> u128 {
        self.ids.iter().flat_map(|i| i.notes.iter()).map(|n| n.amount() as u128).sum()
    }

    pub fn remainder_uterp(&self) -> u128 {
        self.ids.iter().map(|i| i.remainder as u128).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greedy_decomposition() {
        let (n, r) = decompose(18_765_432_100);
        let classes: Vec<u8> = n.iter().map(|x| x.class).collect();
        // 18,765.4321 TERP = 3x5000 + 1x2000 + 1x1000 + 1x500 + 1x200 + 1x50 + 1x10 + 1x5
        assert_eq!(classes, vec![11, 11, 11, 10, 9, 8, 7, 5, 3, 2]);
        assert_eq!(r, 432_100);
        assert_eq!(n.iter().map(|x| x.amount()).sum::<u64>() + r, 18_765_432_100);
        assert!(n.iter().enumerate().all(|(i, x)| x.note_index as usize == i));
        assert_eq!(decompose(999_999), (vec![], 999_999));
        assert_eq!(decompose(4_000_000).0.iter().map(|x| x.class).collect::<Vec<_>>(), vec![1, 1]);
    }

    #[test]
    fn leaves_are_domain_separated() {
        let e = Identity::Evm([1; 20]);
        let s = Identity::Sol([1; 32]);
        let a = Note { class: 0, note_index: 0 };
        let b = Note { class: 1, note_index: 0 };
        let c = Note { class: 0, note_index: 1 };
        assert_ne!(note_leaf(&e, &a), note_leaf(&s, &a));
        assert_ne!(note_leaf(&e, &a), note_leaf(&e, &b));
        assert_ne!(note_leaf(&e, &a), note_leaf(&e, &c));
        assert_ne!(note_leaf(&e, &a), crate::eligibility::evm_leaf(&[1; 20], 1_000_000));
        // Fixed vector for the contract port.
        assert_eq!(&hex::encode(note_leaf(&e, &a))[..8], NOTE_LEAF_VECTOR_PREFIX);
    }

    #[test]
    fn tree_paths_open_and_invariant_holds() {
        let rows = vec![
            (Identity::Evm([2; 20]), 7_500_000u64),
            (Identity::Sol([3; 32]), 12_345_678_901),
            (Identity::Evm([1; 20]), 900_000),
        ];
        let t = NoteTree::build(&rows).unwrap();
        assert_eq!(t.ids[0].identity, Identity::Evm([1; 20]));
        assert_eq!(t.ids[0].notes.len(), 0);
        let total: u128 = rows.iter().map(|r| r.1 as u128).sum();
        assert_eq!(t.notes_uterp() + t.remainder_uterp(), total);
        for i in &t.ids {
            for (k, n) in i.notes.iter().enumerate() {
                let idx = i.first_leaf + k;
                assert_eq!(t.leaf(idx), note_leaf(&i.identity, n));
                assert_eq!(open_note(t.leaf(idx), idx, &t.path(idx)), t.root());
            }
        }
        let tab = t.class_table();
        assert_eq!(tab.iter().map(|x| x.0).sum::<u64>() as usize, t.note_count());
        assert!(NoteTree::build(&[rows[0], rows[0]]).is_err());
    }
}

#[cfg(test)]
const NOTE_LEAF_VECTOR_PREFIX: &str = "76925bb0";
