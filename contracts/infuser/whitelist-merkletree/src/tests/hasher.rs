use rs_merkle::Hasher;

#[derive(Clone)]
pub struct SortingBlake3Hasher {}

// Default Sha256 doesn't sort left and right which make it impossible to
// compare on the contract side without extra payload
impl Hasher for SortingBlake3Hasher {
    type Hash = [u8; 32];

    fn concat_and_hash(left: &Self::Hash, right: Option<&Self::Hash>) -> Self::Hash {
        match right {
            Some(right_node) => {
                let mut both = [left, right_node];
                both.sort_unstable();

                let mut concatenated: Vec<u8> = both[0].to_vec();
                concatenated.extend_from_slice(both[1]);

                Self::hash(&concatenated)
            }
            None => *left,
        }
    }

    fn hash(data: &[u8]) -> Self::Hash {
        *blake3::hash(data).as_bytes()
    }

    fn hash_size() -> usize {
        blake3::OUT_LEN
    }
}
