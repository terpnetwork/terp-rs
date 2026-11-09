use rs_merkle::MerkleTree;
use std::{fs::File, io::Read};

use super::hasher::SortingBlake3Hasher;

fn text_from_file(path: &str) -> String {
    let mut file = File::open(path).unwrap();
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    data
}

pub fn hash_and_build_tree(serialized: &[String]) -> MerkleTree<SortingBlake3Hasher> {
    let leaves: Vec<[u8; 32]> = serialized
        .iter()
        .map(|x| *blake3::hash(x.as_bytes()).as_bytes())
        .collect();
    println!("{:#?}", leaves);
    MerkleTree::<SortingBlake3Hasher>::from_leaves(&leaves)
}

pub fn tree_from_file(path: &str) -> MerkleTree<SortingBlake3Hasher> {
    let data = text_from_file(path);

    let serialized: Vec<String> = data
        .split('\n')
        .map(|x| x.to_string())
        .filter(|s| !s.is_empty() && s.len() > 1)
        .collect();

    hash_and_build_tree(&serialized)
}

pub fn get_merkle_tree_simple(path_prefix: Option<String>) -> MerkleTree<SortingBlake3Hasher> {
    let path = path_prefix.unwrap_or_default() + "src/tests/data/whitelist_simple.txt";
    tree_from_file(path.as_str())
}

pub fn get_merkle_tree_medium() -> MerkleTree<SortingBlake3Hasher> {
    let path = "src/tests/data/whitelist_medium.txt";
    tree_from_file(path)
}

pub fn get_merkle_tree_large() -> MerkleTree<SortingBlake3Hasher> {
    let path = "src/tests/data/whitelist_medium.txt";
    let data = text_from_file(path);

    let mut serialized: Vec<String> = data
        .split('\n')
        .map(|x| x.to_string())
        .filter(|s| !s.is_empty() && s.len() > 1)
        .collect();

    for _ in 0..5 {
        serialized.extend(serialized.clone());
    }

    hash_and_build_tree(&serialized)
}
