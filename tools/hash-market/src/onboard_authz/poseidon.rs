//! Poseidon-128 over Pallas Fp (t=3, α=5). Ingress SoT for guest_root leaves.
//!
//! `leaf = Poseidon(ticketSecret, sku, event_id)`
//! `nullifier = Poseidon(ticketSecret, event_id)` (door; not published at sale)
//! Parent = Poseidon(left, right). Not SHA2, not Sinsemilla/Pedersen.
//!
//! Round constants: SHA256("itl-poseidon-pallas-t3-v1" || i) reduced into Fp.
//! MDS: Cauchy 1/(i+j+3). Consume Halo2 gadgets must use this same hash.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Pallas base-field modulus (little-endian u64 limbs).
const P: [u64; 4] = [
    0x992d30ed00000001,
    0x224698fc094cf91b,
    0x0000000000000000,
    0x4000000000000000,
];

const T: usize = 3;
const RF: usize = 8;
const RP: usize = 56;
const ROUNDS: usize = RF + RP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fp(pub [u64; 4]);

impl Fp {
    pub const ZERO: Self = Self([0; 4]);
    pub const ONE: Self = Self([1, 0, 0, 0]);

    pub fn from_u64(x: u64) -> Self {
        Self([x, 0, 0, 0])
    }

    /// 32-byte LE, top two bits cleared so the value is < 2^254 < p.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut b = [0u8; 32];
        let n = bytes.len().min(32);
        b[..n].copy_from_slice(&bytes[..n]);
        b[31] &= 0x3f;
        Self([
            u64::from_le_bytes(b[0..8].try_into().unwrap()),
            u64::from_le_bytes(b[8..16].try_into().unwrap()),
            u64::from_le_bytes(b[16..24].try_into().unwrap()),
            u64::from_le_bytes(b[24..32].try_into().unwrap()),
        ])
    }

    pub fn from_str_bytes(s: &str) -> Self {
        Self::from_bytes(s.as_bytes())
    }

    pub fn to_bytes(self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..8].copy_from_slice(&self.0[0].to_le_bytes());
        b[8..16].copy_from_slice(&self.0[1].to_le_bytes());
        b[16..24].copy_from_slice(&self.0[2].to_le_bytes());
        b[24..32].copy_from_slice(&self.0[3].to_le_bytes());
        b
    }

    pub fn to_hex(self) -> String {
        hex_encode(&self.to_bytes())
    }

    fn add(self, rhs: Self) -> Self {
        let mut out = [0u64; 4];
        let mut carry = 0u128;
        for i in 0..4 {
            carry += self.0[i] as u128 + rhs.0[i] as u128;
            out[i] = carry as u64;
            carry >>= 64;
        }
        if carry > 0 || gte4(&out, &P) {
            sub4(&mut out, &P);
        }
        Self(out)
    }

    #[allow(dead_code)]
    fn sub(self, rhs: Self) -> Self {
        let mut out = [0u64; 4];
        let mut borrow = 0i128;
        for i in 0..4 {
            let v = self.0[i] as i128 - rhs.0[i] as i128 - borrow;
            if v < 0 {
                out[i] = (v + (1i128 << 64)) as u64;
                borrow = 1;
            } else {
                out[i] = v as u64;
                borrow = 0;
            }
        }
        if borrow != 0 {
            add4(&mut out, &P);
        }
        Self(out)
    }

    fn mul(self, rhs: Self) -> Self {
        let w = mul_wide(&self.0, &rhs.0);
        rem_p(w)
    }

    fn square(self) -> Self {
        self.mul(self)
    }

    fn pow5(self) -> Self {
        let s = self.square();
        s.square().mul(self)
    }

    fn inv(self) -> Self {
        // a^{p-2}
        let e: [u64; 4] = [
            0x992d30ecffffffff,
            0x224698fc094cf91b,
            0x0000000000000000,
            0x4000000000000000,
        ];
        let mut base = self;
        let mut acc = Self::ONE;
        for limb in e {
            let mut x = limb;
            for _ in 0..64 {
                if x & 1 == 1 {
                    acc = acc.mul(base);
                }
                base = base.square();
                x >>= 1;
            }
        }
        acc
    }
}

fn gte4(a: &[u64; 4], b: &[u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] > b[i] {
            return true;
        }
        if a[i] < b[i] {
            return false;
        }
    }
    true
}

fn sub4(a: &mut [u64; 4], b: &[u64; 4]) {
    let mut borrow = 0u64;
    for i in 0..4 {
        let (x1, c1) = a[i].overflowing_sub(b[i]);
        let (x2, c2) = x1.overflowing_sub(borrow);
        a[i] = x2;
        borrow = u64::from(c1 || c2);
    }
}

#[allow(dead_code)]
fn add4(a: &mut [u64; 4], b: &[u64; 4]) {
    let mut carry = 0u128;
    for i in 0..4 {
        carry += a[i] as u128 + b[i] as u128;
        a[i] = carry as u64;
        carry >>= 64;
    }
}

fn mul_wide(a: &[u64; 4], b: &[u64; 4]) -> [u64; 8] {
    let mut r = [0u64; 8];
    for i in 0..4 {
        let mut carry = 0u128;
        for j in 0..4 {
            carry += r[i + j] as u128 + a[i] as u128 * b[j] as u128;
            r[i + j] = carry as u64;
            carry >>= 64;
        }
        r[i + 4] = carry as u64;
    }
    r
}

fn gte8(a: &[u64; 8], b: &[u64; 8]) -> bool {
    for i in (0..8).rev() {
        if a[i] > b[i] {
            return true;
        }
        if a[i] < b[i] {
            return false;
        }
    }
    true
}

fn sub8(a: &mut [u64; 8], b: &[u64; 8]) {
    let mut borrow = 0u64;
    for i in 0..8 {
        let (x1, c1) = a[i].overflowing_sub(b[i]);
        let (x2, c2) = x1.overflowing_sub(borrow);
        a[i] = x2;
        borrow = u64::from(c1 || c2);
    }
}

fn p_shl(shift: u32) -> [u64; 8] {
    let mut out = [0u64; 8];
    let word = (shift / 64) as usize;
    let bit = shift % 64;
    for i in 0..4 {
        let v = P[i];
        if word + i < 8 {
            out[word + i] |= v << bit;
        }
        if bit > 0 && word + i + 1 < 8 {
            out[word + i + 1] |= v >> (64 - bit);
        }
    }
    out
}

fn p_shifted() -> &'static [[u64; 8]; 257] {
    static T: OnceLock<[[u64; 8]; 257]> = OnceLock::new();
    T.get_or_init(|| core::array::from_fn(|s| p_shl(s as u32)))
}

fn rem_p(mut w: [u64; 8]) -> Fp {
    let table = p_shifted();
    for s in (0..=256).rev() {
        if gte8(&w, &table[s]) {
            sub8(&mut w, &table[s]);
        }
    }
    let mut out = [w[0], w[1], w[2], w[3]];
    if gte4(&out, &P) {
        sub4(&mut out, &P);
    }
    Fp(out)
}

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, &'static str> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("odd_hex");
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let nibble = |c: u8| -> Result<u8, &'static str> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err("bad_hex"),
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        out.push((nibble(bytes[i])? << 4) | nibble(bytes[i + 1])?);
        i += 2;
    }
    Ok(out)
}

fn round_constants() -> &'static [Fp] {
    static RC: OnceLock<Vec<Fp>> = OnceLock::new();
    RC.get_or_init(|| {
        let mut out = Vec::with_capacity(ROUNDS * T);
        let mut i = 0u32;
        while out.len() < ROUNDS * T {
            let mut h = Sha256::new();
            h.update(b"itl-poseidon-pallas-t3-v1");
            h.update(i.to_le_bytes());
            out.push(Fp::from_bytes(&h.finalize()));
            i += 1;
        }
        out
    })
}

fn mds() -> &'static [[Fp; T]; T] {
    static MDS: OnceLock<[[Fp; T]; T]> = OnceLock::new();
    MDS.get_or_init(|| {
        let mut m = [[Fp::ZERO; T]; T];
        for i in 0..T {
            for j in 0..T {
                m[i][j] = Fp::from_u64((i + j + 3) as u64).inv();
            }
        }
        m
    })
}

fn mix(state: &mut [Fp; T]) {
    let m = mds();
    let mut out = [Fp::ZERO; T];
    for i in 0..T {
        let mut acc = Fp::ZERO;
        for j in 0..T {
            acc = acc.add(m[i][j].mul(state[j]));
        }
        out[i] = acc;
    }
    *state = out;
}

fn permute(state: &mut [Fp; T]) {
    let rc = round_constants();
    let mut k = 0;
    let half = RF / 2;
    for _ in 0..half {
        for s in state.iter_mut() {
            *s = s.add(rc[k]);
            k += 1;
        }
        for s in state.iter_mut() {
            *s = s.pow5();
        }
        mix(state);
    }
    for _ in 0..RP {
        for s in state.iter_mut() {
            *s = s.add(rc[k]);
            k += 1;
        }
        state[0] = state[0].pow5();
        mix(state);
    }
    for _ in 0..half {
        for s in state.iter_mut() {
            *s = s.add(rc[k]);
            k += 1;
        }
        for s in state.iter_mut() {
            *s = s.pow5();
        }
        mix(state);
    }
}

/// Sponge: capacity = arity, rate = 2. Squeeze state[1].
pub fn poseidon_hash(inputs: &[Fp]) -> Fp {
    let mut state = [Fp::ZERO, Fp::ZERO, Fp::ZERO];
    state[0] = Fp::from_u64(inputs.len() as u64);
    let mut i = 0;
    if inputs.is_empty() {
        permute(&mut state);
        return state[1];
    }
    while i < inputs.len() {
        let mut r = 1;
        while r < T && i < inputs.len() {
            state[r] = state[r].add(inputs[i]);
            i += 1;
            r += 1;
        }
        permute(&mut state);
    }
    state[1]
}

pub fn poseidon2(a: Fp, b: Fp) -> Fp {
    poseidon_hash(&[a, b])
}

pub fn poseidon3(a: Fp, b: Fp, c: Fp) -> Fp {
    poseidon_hash(&[a, b, c])
}

/// `leaf = Poseidon(ticketSecret, sku, event_id)`
pub fn ticket_leaf(ticket_secret: &[u8], sku: &str, event_id: &str) -> [u8; 32] {
    poseidon3(
        Fp::from_bytes(ticket_secret),
        Fp::from_str_bytes(sku),
        Fp::from_str_bytes(event_id),
    )
    .to_bytes()
}

/// Door `nullifier = Poseidon(ticketSecret, event_id)`. Not a payment nullifier.
pub fn door_nullifier(ticket_secret: &[u8], event_id: &str) -> [u8; 32] {
    poseidon2(Fp::from_bytes(ticket_secret), Fp::from_str_bytes(event_id)).to_bytes()
}

/// Incremental Poseidon merkle. Empty leaf = 0. Depth 8 = 256 tickets (lab).
pub const TREE_DEPTH: usize = 8;

#[derive(Debug, Clone)]
pub struct PoseidonTree {
    pub depth: usize,
    pub next: u64,
    nodes: HashMap<(u32, u64), Fp>,
    empty: Vec<Fp>,
}

impl Default for PoseidonTree {
    fn default() -> Self {
        Self::new(TREE_DEPTH)
    }
}

impl PoseidonTree {
    pub fn new(depth: usize) -> Self {
        let mut empty = vec![Fp::ZERO; depth + 1];
        for d in 0..depth {
            empty[d + 1] = poseidon2(empty[d], empty[d]);
        }
        Self {
            depth,
            next: 0,
            nodes: HashMap::new(),
            empty,
        }
    }

    pub fn len(&self) -> usize {
        self.next as usize
    }

    pub fn is_empty(&self) -> bool {
        self.next == 0
    }

    pub fn root(&self) -> Fp {
        self.node(self.depth as u32, 0)
    }

    pub fn root_bytes(&self) -> [u8; 32] {
        self.root().to_bytes()
    }

    pub fn root_hex(&self) -> String {
        self.root().to_hex()
    }

    fn node(&self, level: u32, index: u64) -> Fp {
        self.nodes
            .get(&(level, index))
            .copied()
            .unwrap_or(self.empty[level as usize])
    }

    pub fn append(&mut self, leaf: Fp) -> Result<u64, &'static str> {
        let max = 1u64 << self.depth;
        if self.next >= max {
            return Err("tree_full");
        }
        let idx = self.next;
        self.next += 1;
        self.nodes.insert((0, idx), leaf);
        let mut i = idx;
        let mut cur = leaf;
        for level in 0..self.depth {
            let sibling = i ^ 1;
            let sib = self.node(level as u32, sibling);
            let (l, r) = if i % 2 == 0 { (cur, sib) } else { (sib, cur) };
            cur = poseidon2(l, r);
            i /= 2;
            self.nodes.insert(((level + 1) as u32, i), cur);
        }
        Ok(idx)
    }

    pub fn append_bytes(&mut self, leaf: [u8; 32]) -> Result<u64, &'static str> {
        self.append(Fp::from_bytes(&leaf))
    }

    /// Sibling path for Halo2 consume. `siblings[d]` is the level-d neighbor of the
    /// current node; index bit `d` chooses left/right.
    pub fn prove(&self, index: u64) -> Result<MerklePath, &'static str> {
        if index >= self.next {
            return Err("no_leaf");
        }
        let mut siblings = Vec::with_capacity(self.depth);
        let mut i = index;
        for level in 0..self.depth {
            let sibling = i ^ 1;
            siblings.push(self.node(level as u32, sibling));
            i /= 2;
        }
        Ok(MerklePath { index, siblings })
    }
}

#[derive(Clone, Debug)]
pub struct MerklePath {
    pub index: u64,
    pub siblings: Vec<Fp>,
}

impl MerklePath {
    pub fn verify(&self, leaf: Fp, root: Fp) -> bool {
        let mut cur = leaf;
        let mut i = self.index;
        for sib in &self.siblings {
            let (l, r) = if i % 2 == 0 { (cur, *sib) } else { (*sib, cur) };
            cur = poseidon2(l, r);
            i /= 2;
        }
        cur == root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fp_add_wraps_at_p() {
        let pm1 = Fp(P).sub(Fp::ONE);
        assert_eq!(pm1.add(Fp::ONE), Fp::ZERO);
        assert_eq!(Fp::ONE.add(Fp::ONE), Fp::from_u64(2));
    }

    #[test]
    fn fp_mul_small() {
        assert_eq!(Fp::from_u64(2).mul(Fp::from_u64(3)), Fp::from_u64(6));
        let pm1 = Fp(P).sub(Fp::ONE);
        assert_eq!(pm1.mul(Fp::from_u64(2)), Fp(P).sub(Fp::from_u64(2)));
    }

    #[test]
    fn poseidon_is_not_sha256_and_not_commutative() {
        let a = Fp::from_u64(1);
        let b = Fp::from_u64(2);
        let h = poseidon2(a, b);
        assert_ne!(h, Fp::ZERO);
        assert_ne!(h, poseidon2(b, a));
        let sha = Sha256::digest([1u8, 2]);
        assert_ne!(h.to_bytes(), sha.as_slice());
    }

    #[test]
    fn tree_append_bumps_root() {
        let mut t = PoseidonTree::default();
        let empty = t.root_hex();
        t.append(Fp::from_u64(7)).unwrap();
        let r1 = t.root_hex();
        assert_ne!(empty, r1);
        t.append(Fp::from_u64(8)).unwrap();
        assert_ne!(r1, t.root_hex());
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn merkle_path_verifies_and_rejects_unpaid() {
        let mut t = PoseidonTree::default();
        let a = Fp::from_u64(7);
        let b = Fp::from_u64(8);
        t.append(a).unwrap();
        t.append(b).unwrap();
        let p0 = t.prove(0).unwrap();
        assert!(p0.verify(a, t.root()));
        assert!(!p0.verify(b, t.root()));
        let unpaid = Fp::from_u64(99);
        assert!(!p0.verify(unpaid, t.root()));
        assert_eq!(t.prove(2).unwrap_err(), "no_leaf");
    }
}
