//! Headstash claim statement over the BLAKE3 circuit.
//!
//! One row is one BLAKE3 compression. The wires, not a second hash, are what
//! make it a claim:
//!
//! - `rseed` is a private input. Its compression output is `rseed_com`.
//! - The nullifier row reads that same `rseed` and its output is public.
//! - The leaf row reads `rseed_com` plus the private key and allocation.
//!   A second string is a different leaf. The amount is bound in the next
//!   row and is the only allocation word that is public.
//! - Each Merkle level hashes `left ‖ right` with the running digest on the
//!   side the public index bit selects. The last output is the public root.
//!
//! A proof is a Ligerito proof of this circuit. The nullifier is not a free
//! witness: it is the compression output, and the same string is what the
//! leaf commits to.

use flock_core::circuit::builder::{CircuitBuilder, GateType, SlotId, SlotWitness};
use flock_core::field::F128;
use flock_core::schedule::TableType;
use flock_hash::blake3_compress;

use flock_prover::r1cs_hashes::blake3::{Compression, build_block_r1cs, io_schema};

const IV: [u32; 8] = [
    0x6A09_E667, 0xBB67_AE85, 0x3C6E_F372, 0xA54F_F53A, 0x510E_527F, 0x9B05_688C, 0x1F83_D9AB,
    0x5BE0_CD19,
];

/// Production inclusion depth. `2^16 = 65536` leaves, which covers the
/// 51,807-note eligible set. A depth-2 path holds four leaves and is only
/// the fixture used by the small tests.
pub const DEPTH: usize = 16;
pub const FIXTURE_DEPTH: usize = 2;

pub(crate) const FLAG_RSEED: u32 = 1;
pub(crate) const FLAG_NULLIFIER: u32 = 2;
pub(crate) const FLAG_LEAF: u32 = 3;
pub(crate) const FLAG_NODE: u32 = 4;
pub(crate) const FLAG_KEY: u32 = 7;
pub(crate) const FLAG_BIND: u32 = 8;
/// Hiding commitment of the payout. Destination, amount, and the blind are
/// witnesses. The digest is a public word.
pub(crate) const FLAG_PAYOUT_COM: u32 = 9;
/// Second compression of the payout. Its amount input is the leaf amount wire.
pub(crate) const FLAG_PAYOUT_BIND: u32 = 10;

/// Private opening of one leaf, plus the siblings for `D` levels.
/// `destination` is the payout address (20 bytes, five words). `v` is both
/// the leaf amount and the public payout amount.
///
/// `D` is [`DEPTH`] for the real set. The index bits are wired when the
/// circuit is built, so two leaf positions are two circuit shapes.
pub struct Opening<const D: usize> {
    pub rseed: [u32; 8],
    pub epk_x: [u32; 4],
    pub nd: [u32; 2],
    pub v: u32,
    pub fdi: u32,
    /// Leaf position. The circuit shape is this index's path.
    pub index: u32,
    pub siblings: [[u32; 8]; D],
    pub destination: [u32; 5],
}

/// Public claim: tree root, nullifier, and the payout the leaf amount pays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub root: [u32; 8],
    pub nullifier: [u32; 8],
    pub destination: [u32; 5],
    pub amount: u32,
}

struct Blake3Gate {
    nu: usize,
}

impl GateType for Blake3Gate {
    type Row = Compression;
    type Hint = ();

    fn table(&self) -> TableType {
        TableType::from_block_r1cs(&build_block_r1cs(self.nu)).with_io_schema(io_schema())
    }

    fn eval(&self, inputs: &[F128], _hint: &(), outputs: &mut Vec<F128>) -> Self::Row {
        let cv = unpack8(inputs[0], inputs[1]);
        let mut m = [0u32; 16];
        for i in 0..4 {
            let w = unpack4(inputs[2 + i]);
            m[4 * i..4 * i + 4].copy_from_slice(&w);
        }
        let (counter, block_len, flags) = unpack_params(inputs[6]);
        let out = blake3_compress(&cv, &m, counter, block_len, flags);
        let out_lo: [u32; 8] = out[0..8].try_into().unwrap();
        let out_hi: [u32; 8] = out[8..16].try_into().unwrap();
        let (lo, hi) = (pack8(&out_lo), pack8(&out_hi));
        outputs.extend_from_slice(&[lo[0], lo[1], hi[0], hi[1]]);
        (cv, m, counter, block_len, flags)
    }

    fn witness(&self, _rows: &[Self::Row], _nu: usize) -> SlotWitness {
        SlotWitness::DeferredToRows
    }
}

pub(crate) fn compress(message: &[u32; 16], flags: u32) -> [u32; 8] {
    let out = blake3_compress(&IV, message, 0, 64, flags);
    out[0..8].try_into().unwrap()
}

pub(crate) fn rseed_message(rseed: &[u32; 8]) -> [u32; 16] {
    let mut m = [0u32; 16];
    m[..8].copy_from_slice(rseed);
    m
}

/// Blake3 commitment of the leaf key. The sidecar checks this against the
/// recovered MetaMask key. It is not the key, and the chain batch does not
/// carry it.
pub(crate) fn key_com(epk_x: &[u32; 4]) -> [u32; 8] {
    let mut m = [0u32; 16];
    m[..4].copy_from_slice(epk_x);
    compress(&m, FLAG_KEY)
}

/// Merkle leaf. The first compression absorbs `rseed_com`, the key, and the
/// allocation except the amount. The second absorbs that digest and the
/// amount, so the amount is bound and the key is not a public input.
pub(crate) fn merkle_leaf(
    rseed_com: &[u32; 8],
    epk_x: &[u32; 4],
    nd: &[u32; 2],
    v: u32,
    fdi: u32,
) -> [u32; 8] {
    let mut alloc = [0u32; 16];
    alloc[..8].copy_from_slice(rseed_com);
    alloc[8..12].copy_from_slice(epk_x);
    alloc[12] = nd[0];
    alloc[13] = nd[1];
    alloc[14] = fdi;
    let inner = compress(&alloc, FLAG_LEAF);
    let mut bound = [0u32; 16];
    bound[..8].copy_from_slice(&inner);
    bound[8] = v;
    compress(&bound, FLAG_BIND)
}

pub(crate) fn node_message(left: &[u32; 8], right: &[u32; 8]) -> [u32; 16] {
    let mut m = [0u32; 16];
    m[..8].copy_from_slice(left);
    m[8..].copy_from_slice(right);
    m
}

/// Root and nullifier for this opening. The nullifier depends only on `rseed`.
/// The root depends on `rseed` through the leaf, so a second string does not
/// sit under the same root.
pub fn derive<const D: usize>(opening: &Opening<D>) -> Claim {
    let rseed_com = compress(&rseed_message(&opening.rseed), FLAG_RSEED);
    let nullifier = compress(&rseed_message(&opening.rseed), FLAG_NULLIFIER);
    let mut running = merkle_leaf(
        &rseed_com,
        &opening.epk_x,
        &opening.nd,
        opening.v,
        opening.fdi,
    );
    for level in 0..D {
        let bit = (opening.index >> level) & 1 == 1;
        let (left, right) = if bit {
            (&opening.siblings[level], &running)
        } else {
            (&running, &opening.siblings[level])
        };
        // `running` is moved into the message by copy.
        let left = *left;
        let right = *right;
        running = compress(&node_message(&left, &right), FLAG_NODE);
    }
    Claim {
        root: running,
        nullifier,
        destination: opening.destination,
        amount: opening.v,
    }
}

fn pack4(w: [u32; 4]) -> F128 {
    F128::new(w[0] as u64 | ((w[1] as u64) << 32), w[2] as u64 | ((w[3] as u64) << 32))
}

fn unpack4(v: F128) -> [u32; 4] {
    [v.lo as u32, (v.lo >> 32) as u32, v.hi as u32, (v.hi >> 32) as u32]
}

fn pack8(w: &[u32; 8]) -> [F128; 2] {
    [pack4([w[0], w[1], w[2], w[3]]), pack4([w[4], w[5], w[6], w[7]])]
}

fn unpack8(a: F128, b: F128) -> [u32; 8] {
    let (x, y) = (unpack4(a), unpack4(b));
    [x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3]]
}

fn pack_params(flags: u32) -> F128 {
    F128::new(0, 64u64 | ((flags as u64) << 32))
}

fn unpack_params(v: F128) -> (u64, u32, u32) {
    (v.lo, v.hi as u32, (v.hi >> 32) as u32)
}

/// Build the claim circuit. `rseed`, the siblings, the leaf key, and the
/// allocation except the amount enter as private values. The public words
/// are the nullifier, the key commitment, the amount, the destination, and
/// the root.
pub fn build<const D: usize>(
    opening: &Opening<D>,
) -> (flock_core::circuit::builder::BuiltCircuit, SlotId) {
    let claim = derive(opening);
    let nu = 8;
    let mut b = CircuitBuilder::new(nu);
    let g: SlotId = b.slot(Blake3Gate { nu });
    let iv = pack8(&IV);
    let cv = [b.public_value(iv[0]), b.public_value(iv[1])];

    let rseed = pack8(&opening.rseed);
    let rseed_w = [b.value(rseed[0]), b.value(rseed[1])];
    let zero = b.public_value(F128::ZERO);
    let rseed_params = b.public_value(pack_params(FLAG_RSEED));
    let rseed_out = b.gate(g, &[cv[0], cv[1], rseed_w[0], rseed_w[1], zero, zero, rseed_params]);

    let nf_params = b.public_value(pack_params(FLAG_NULLIFIER));
    let nf_out = b.gate(g, &[cv[0], cv[1], rseed_w[0], rseed_w[1], zero, zero, nf_params]);
    let nf_at = b.public_len();
    b.publish(nf_out[0]);
    b.publish(nf_out[1]);

    let epk = b.value(pack4(opening.epk_x));
    let key_params = b.public_value(pack_params(FLAG_KEY));
    let key_out = b.gate(g, &[cv[0], cv[1], epk, zero, zero, zero, key_params]);
    let key_at = b.public_len();
    b.publish(key_out[0]);
    b.publish(key_out[1]);

    let alloc = b.value(pack4([opening.nd[0], opening.nd[1], opening.fdi, 0]));
    let leaf_params = b.public_value(pack_params(FLAG_LEAF));
    let inner = b.gate(
        g,
        &[cv[0], cv[1], rseed_out[0], rseed_out[1], epk, alloc, leaf_params],
    );
    let amount = b.public_value(pack4([opening.v, 0, 0, 0]));
    let bind_params = b.public_value(pack_params(FLAG_BIND));
    let leaf_out = b.gate(g, &[cv[0], cv[1], inner[0], inner[1], amount, zero, bind_params]);
    let dest_lo = b.public_value(pack4([
        opening.destination[0],
        opening.destination[1],
        opening.destination[2],
        opening.destination[3],
    ]));
    let dest_hi = b.public_value(F128::new(opening.destination[4] as u64, 0));

    let mut running = [leaf_out[0], leaf_out[1]];
    let node_params = b.public_value(pack_params(FLAG_NODE));
    for level in 0..D {
        let sib = pack8(&opening.siblings[level]);
        let sib_w = [b.value(sib[0]), b.value(sib[1])];
        let bit = (opening.index >> level) & 1 == 1;
        let (left, right) = if bit {
            (sib_w, running)
        } else {
            (running, sib_w)
        };
        let out = b.gate(g, &[cv[0], cv[1], left[0], left[1], right[0], right[1], node_params]);
        running = [out[0], out[1]];
    }
    let root_at = b.public_len();
    b.publish(running[0]);
    b.publish(running[1]);
    let dest_at = b.public_len();
    b.publish(dest_lo);
    b.publish(dest_hi);
    let amount_at = b.public_len();
    b.publish(amount);

    let built = b.finish().expect("headstash circuit");
    let published = &built.witness.public;
    let got_nf = unpack8(published[nf_at], published[nf_at + 1]);
    let got_key = unpack8(published[key_at], published[key_at + 1]);
    let got_root = unpack8(published[root_at], published[root_at + 1]);
    let got_dest_lo = unpack4(published[dest_at]);
    let got_dest_hi = unpack4(published[dest_at + 1]);
    let got_amount = unpack4(published[amount_at]);
    assert_eq!(got_nf, claim.nullifier, "published nullifier");
    assert_eq!(got_key, key_com(&opening.epk_x), "published key commitment");
    assert_eq!(got_root, claim.root, "published root");
    assert_eq!(got_dest_lo, [
        claim.destination[0],
        claim.destination[1],
        claim.destination[2],
        claim.destination[3],
    ]);
    assert_eq!(got_dest_hi[0], claim.destination[4]);
    assert_eq!(got_amount[0], claim.amount, "payout amount is the leaf amount");
    (built, g)
}

pub mod batch;
pub mod sidecar;
pub mod statement;
pub mod tree;

/// Same inclusion opening as [`Opening`], plus the blind that hides the payout.
///
/// The public statement is [`PrivateClaim`]. The leaf key, denomination,
/// amount, destination, and blind are witnesses.
pub struct PrivateOpening<const D: usize> {
    pub open: Opening<D>,
    pub blind: [u32; 8],
}

/// What a verifier of the private claim is allowed to see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateClaim {
    pub root: [u32; 8],
    pub nullifier: [u32; 8],
    pub payout_com: [u32; 8],
}

/// First payout block: five destination words and the eight-word blind.
/// The amount is not in this block. It is bound by [`payout_commitment`]'s
/// second compression, on the same wire the leaf uses.
pub(crate) fn payout_message(destination: &[u32; 5], blind: &[u32; 8]) -> [u32; 16] {
    let mut m = [0u32; 16];
    m[0] = destination[0];
    m[1] = destination[1];
    m[2] = destination[2];
    m[3] = destination[3];
    m[4] = destination[4];
    m[5] = blind[0];
    m[6] = blind[1];
    m[7] = blind[2];
    m[8] = blind[3];
    m[9] = blind[4];
    m[10] = blind[5];
    m[11] = blind[6];
    m[12] = blind[7];
    m
}

pub(crate) fn payout_commitment(destination: &[u32; 5], amount: u32, blind: &[u32; 8]) -> [u32; 8] {
    let inner = compress(&payout_message(destination, blind), FLAG_PAYOUT_COM);
    let mut bound = [0u32; 16];
    bound[..8].copy_from_slice(&inner);
    bound[8] = amount;
    compress(&bound, FLAG_PAYOUT_BIND)
}

/// Root, nullifier, and payout commitment. The blind does not enter the leaf,
/// so it does not move the allocation root.
pub fn derive_private<const D: usize>(opening: &PrivateOpening<D>) -> PrivateClaim {
    let clear = derive(&opening.open);
    PrivateClaim {
        root: clear.root,
        nullifier: clear.nullifier,
        payout_com: payout_commitment(&opening.open.destination, opening.open.v, &opening.blind),
    }
}

/// Private statement over the same Blake3 rows as [`build`].
///
/// Public words are the nullifier, the allocation root, and `payout_com`.
/// The amount is a witness and is the same wire the leaf binds. The
/// destination and the blind are witnesses inside `payout_com`. The leaf key
/// is a witness inside the leaf. This circuit does not yet check the
/// secp256k1 signature. That check is `prove_ecdsa_verify` on the Halo2 side
/// and the missing Ligerito gate on this side.
pub fn build_private<const D: usize>(
    opening: &PrivateOpening<D>,
) -> (flock_core::circuit::builder::BuiltCircuit, SlotId) {
    let claim = derive_private(opening);
    let nu = 8;
    let mut b = CircuitBuilder::new(nu);
    let g: SlotId = b.slot(Blake3Gate { nu });
    let iv = pack8(&IV);
    let cv = [b.public_value(iv[0]), b.public_value(iv[1])];
    let zero = b.public_value(F128::ZERO);

    let rseed = pack8(&opening.open.rseed);
    let rseed_w = [b.value(rseed[0]), b.value(rseed[1])];
    let rseed_params = b.public_value(pack_params(FLAG_RSEED));
    let rseed_out = b.gate(g, &[cv[0], cv[1], rseed_w[0], rseed_w[1], zero, zero, rseed_params]);

    let nf_params = b.public_value(pack_params(FLAG_NULLIFIER));
    let nf_out = b.gate(g, &[cv[0], cv[1], rseed_w[0], rseed_w[1], zero, zero, nf_params]);
    let nf_at = b.public_len();
    b.publish(nf_out[0]);
    b.publish(nf_out[1]);

    let epk = b.value(pack4(opening.open.epk_x));
    let alloc = b.value(pack4([
        opening.open.nd[0],
        opening.open.nd[1],
        opening.open.fdi,
        0,
    ]));
    let leaf_params = b.public_value(pack_params(FLAG_LEAF));
    let inner = b.gate(
        g,
        &[cv[0], cv[1], rseed_out[0], rseed_out[1], epk, alloc, leaf_params],
    );
    let amount = b.value(pack4([opening.open.v, 0, 0, 0]));
    let bind_params = b.public_value(pack_params(FLAG_BIND));
    let leaf_out = b.gate(g, &[cv[0], cv[1], inner[0], inner[1], amount, zero, bind_params]);

    let mut running = [leaf_out[0], leaf_out[1]];
    let node_params = b.public_value(pack_params(FLAG_NODE));
    for level in 0..D {
        let sib = pack8(&opening.open.siblings[level]);
        let sib_w = [b.value(sib[0]), b.value(sib[1])];
        let bit = (opening.open.index >> level) & 1 == 1;
        let (left, right) = if bit {
            (sib_w, running)
        } else {
            (running, sib_w)
        };
        let out = b.gate(g, &[cv[0], cv[1], left[0], left[1], right[0], right[1], node_params]);
        running = [out[0], out[1]];
    }
    let root_at = b.public_len();
    b.publish(running[0]);
    b.publish(running[1]);

    let pay = payout_message(&opening.open.destination, &opening.blind);
    let pay_w = [
        b.value(pack4([pay[0], pay[1], pay[2], pay[3]])),
        b.value(pack4([pay[4], pay[5], pay[6], pay[7]])),
        b.value(pack4([pay[8], pay[9], pay[10], pay[11]])),
        b.value(pack4([pay[12], pay[13], pay[14], pay[15]])),
    ];
    let pay_params = b.public_value(pack_params(FLAG_PAYOUT_COM));
    let pay_inner = b.gate(
        g,
        &[cv[0], cv[1], pay_w[0], pay_w[1], pay_w[2], pay_w[3], pay_params],
    );
    let bind_pay = b.public_value(pack_params(FLAG_PAYOUT_BIND));
    let pay_out = b.gate(
        g,
        &[cv[0], cv[1], pay_inner[0], pay_inner[1], amount, zero, bind_pay],
    );
    let pay_at = b.public_len();
    b.publish(pay_out[0]);
    b.publish(pay_out[1]);

    let built = b.finish().expect("private headstash circuit");
    let published = &built.witness.public;
    assert_eq!(
        unpack8(published[nf_at], published[nf_at + 1]),
        claim.nullifier
    );
    assert_eq!(
        unpack8(published[root_at], published[root_at + 1]),
        claim.root
    );
    assert_eq!(
        unpack8(published[pay_at], published[pay_at + 1]),
        claim.payout_com
    );
    let hidden_key = pack4(opening.open.epk_x);
    let hidden_amount = pack4([opening.open.v, 0, 0, 0]);
    let hidden_dest = pack4([
        opening.open.destination[0],
        opening.open.destination[1],
        opening.open.destination[2],
        opening.open.destination[3],
    ]);
    assert!(published.iter().all(|word| *word != hidden_key));
    assert!(published.iter().all(|word| *word != hidden_amount));
    assert!(published.iter().all(|word| *word != hidden_dest));
    (built, g)
}

const NU: usize = 8;
const DOMAIN: &[u8] = b"terp-headstash-flock-v1";

/// Public-word positions written by [`build`]. The shape does not depend on
/// the private `rseed` or the siblings, so a verifier rebuilds it from the
/// leaf index and checks these words.
const PUB_NF: usize = 5;
const PUB_KEY: usize = 8;
const PUB_AMOUNT: usize = 11;
const PUB_DEST_LO: usize = 13;
const PUB_DEST_HI: usize = 14;
const PUB_ROOT: usize = 16;
const PUB_DEST_LO_AGAIN: usize = 18;
const PUB_DEST_HI_AGAIN: usize = 19;
const PUB_AMOUNT_AGAIN: usize = 20;
const PUB_LEN: usize = 21;

/// Read the claim out of the public segment. The destination and the amount
/// are published twice; both copies have to match.
pub fn claim_from_public(public: &[F128]) -> Option<Claim> {
    if public.len() < PUB_LEN {
        return None;
    }
    if public[PUB_DEST_LO] != public[PUB_DEST_LO_AGAIN]
        || public[PUB_DEST_HI] != public[PUB_DEST_HI_AGAIN]
        || public[PUB_AMOUNT] != public[PUB_AMOUNT_AGAIN]
    {
        return None;
    }
    let dest_lo = unpack4(public[PUB_DEST_LO]);
    let dest_hi = unpack4(public[PUB_DEST_HI]);
    let amount = unpack4(public[PUB_AMOUNT]);
    Some(Claim {
        nullifier: unpack8(public[PUB_NF], public[PUB_NF + 1]),
        root: unpack8(public[PUB_ROOT], public[PUB_ROOT + 1]),
        destination: [dest_lo[0], dest_lo[1], dest_lo[2], dest_lo[3], dest_hi[0]],
        amount: amount[0],
    })
}

/// Blake3 commitment of the leaf key published beside the nullifier.
/// The chain batch does not store this.
pub fn public_key_com(public: &[F128]) -> Option<[u32; 8]> {
    if public.len() < PUB_LEN {
        return None;
    }
    Some(unpack8(public[PUB_KEY], public[PUB_KEY + 1]))
}

fn skeleton<const D: usize>(index: u32) -> Opening<D> {
    Opening {
        rseed: [0; 8],
        epk_x: [0; 4],
        nd: [0; 2],
        v: 0,
        fdi: 0,
        index,
        siblings: [[0; 8]; D],
        destination: [0; 5],
    }
}

/// One Ligerito proof of [`build`], plus the time to build the circuit,
/// prove, and verify.
pub struct ProofBench {
    pub gates: usize,
    pub build_secs: f64,
    pub prove_secs: f64,
    pub verify_secs: f64,
    pub proof_bytes: usize,
}

/// Proof bytes the sidecar verifies. The public words are the statement.
/// `rseed` is not in here.
pub struct ClaimProof {
    pub proof: flock_core::proof::R1csProofCircuitMerged,
    pub commitment: flock_core::pcs::Commitment,
    pub public: Vec<F128>,
}

#[derive(Debug)]
pub enum VerifyError {
    Index,
    Public,
    Proof,
}

/// Prove one opening. The sidecar never sees this witness.
pub fn prove_claim<const D: usize>(opening: &Opening<D>) -> ClaimProof {
    let (built, slot) = build(opening);
    let rows = built.rows::<Blake3Gate>(slot).to_vec();
    let (proof, commitment) = prove_built(&built, &rows);
    ClaimProof {
        proof,
        commitment,
        public: built.witness.public.clone(),
    }
}

/// Verify a proof against a circuit rebuilt from `index` alone.
///
/// The index selects the Merkle wiring, so it is part of the statement the
/// sidecar has to rebuild. Private `rseed` and siblings are not.
pub fn verify_claim<const D: usize>(
    index: u32,
    proof: &ClaimProof,
) -> Result<Claim, VerifyError> {
    if D >= 32 || (index >> D) != 0 {
        return Err(VerifyError::Index);
    }
    let claim = claim_from_public(&proof.public).ok_or(VerifyError::Public)?;
    let (built, _) = build(&skeleton::<D>(index));
    verify_built(&built, &proof.public, &proof.commitment, &proof.proof)
        .map_err(|_| VerifyError::Proof)?;
    Ok(claim)
}

fn prove_built(
    built: &flock_core::circuit::builder::BuiltCircuit,
    rows: &[Compression],
) -> (
    flock_core::proof::R1csProofCircuitMerged,
    flock_core::pcs::Commitment,
) {
    use flock_core::pcs::ligerito::LigeritoProfile;
    use flock_core::pcs::PcsParams;
    use flock_core::union::UnionInstance;
    use flock_prover::challenger::FsChallenger;
    use flock_prover::prover::{self, UnionSlotProverInput};
    use flock_prover::r1cs_hashes::blake3::generate_witness_batch_major_partial;

    let union = UnionInstance::new(&built.shape.registry, built.shape.counts.clone());
    let pcs_params = PcsParams {
        m: union.dense_m(),
        log_inv_rate: 1,
        log_batch_size: 6,
        profile: LigeritoProfile::Fast,
        num_lanes: union.commit_lanes(6),
        merkle_hash: Default::default(),
    };
    let r1cs = build_block_r1cs(NU);
    let lc = r1cs.csc_lincheck_circuit();
    let witness = generate_witness_batch_major_partial(rows, NU);
    let mut ch = FsChallenger::new(DOMAIN);
    let (proof, commitment, _) = prover::prove_fast_ligerito_union_circuit(
        &union,
        &built.shape.circuit,
        &built.witness.public,
        &pcs_params,
        vec![UnionSlotProverInput::new(witness, lc)],
        Vec::new(),
        &mut ch,
    );
    (proof, commitment)
}

fn verify_built(
    built: &flock_core::circuit::builder::BuiltCircuit,
    public: &[F128],
    commitment: &flock_core::pcs::Commitment,
    proof: &flock_core::proof::R1csProofCircuitMerged,
) -> Result<(), ()> {
    use flock_core::pcs::ligerito::LigeritoProfile;
    use flock_core::pcs::PcsParams;
    use flock_core::union::UnionInstance;
    use flock_prover::challenger::FsChallenger;
    use flock_prover::verifier;

    let union = UnionInstance::new(&built.shape.registry, built.shape.counts.clone());
    let pcs_params = PcsParams {
        m: union.dense_m(),
        log_inv_rate: 1,
        log_batch_size: 6,
        profile: LigeritoProfile::Fast,
        num_lanes: union.commit_lanes(6),
        merkle_hash: Default::default(),
    };
    let r1cs = build_block_r1cs(NU);
    let lc = r1cs.csc_lincheck_circuit();
    let mut ch = FsChallenger::new(DOMAIN);
    verifier::verify_ligerito_union_circuit(
        &union,
        &built.shape.circuit,
        public,
        &[lc],
        commitment,
        proof,
        &pcs_params,
        &mut ch,
    )
    .map(|_| ())
    .map_err(|_| ())
}

/// Prove the claim and verify it against the same public words. A flipped
/// public root must fail. The timed verify is the native Ligerito check.
pub fn prove_and_verify<const D: usize>(opening: &Opening<D>) -> ProofBench {
    use std::time::Instant;

    let t0 = Instant::now();
    let (built, slot) = build(opening);
    let build_secs = t0.elapsed().as_secs_f64();
    let rows = built.rows::<Blake3Gate>(slot);
    let gates = rows.len();
    let rows = rows.to_vec();

    let t1 = Instant::now();
    let (proof, commitment) = prove_built(&built, &rows);
    let prove_secs = t1.elapsed().as_secs_f64();

    let t2 = Instant::now();
    verify_built(&built, &built.witness.public, &commitment, &proof)
        .expect("honest headstash claim verifies");
    let verify_secs = t2.elapsed().as_secs_f64();

    let mut bad = built.witness.public.clone();
    let last = bad.len() - 1;
    bad[last] += F128::ONE;
    assert!(
        verify_built(&built, &bad, &commitment, &proof).is_err(),
        "a flipped public root must fail"
    );

    ProofBench {
        gates,
        build_secs,
        prove_secs,
        verify_secs,
        proof_bytes: bincode::serialize(&proof).expect("proof bytes").len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<const D: usize>(rseed0: u32, index: u32) -> Opening<D> {
        let mut siblings = [[0u32; 8]; D];
        siblings[0] = [21, 22, 23, 24, 25, 26, 27, 28];
        if D > 1 {
            siblings[1] = [31, 32, 33, 34, 35, 36, 37, 38];
        }
        Opening {
            rseed: [rseed0, 2, 3, 4, 5, 6, 7, 8],
            epk_x: [9, 10, 11, 12],
            nd: [13, 14],
            v: 15,
            fdi: 1,
            index,
            siblings,
            destination: [0x1111, 0x2222, 0x3333, 0x4444, 0x5555],
        }
    }

    #[test]
    fn nullifier_is_the_string_and_the_root_binds_it() {
        let a = sample::<FIXTURE_DEPTH>(1, 1);
        let claim = derive(&a);
        let again = derive(&a);
        assert_eq!(claim.nullifier, again.nullifier);
        assert_eq!(claim.root, again.root);
        assert_eq!(claim.amount, a.v);
        assert_eq!(claim.destination, a.destination);

        let mut other = sample::<FIXTURE_DEPTH>(99, 1);
        other.siblings = a.siblings;
        let moved = derive(&other);
        assert_ne!(moved.nullifier, claim.nullifier, "a second string is a second nullifier");
        assert_ne!(moved.root, claim.root, "a second string is not in the same leaf");

        let mut relocated = sample::<FIXTURE_DEPTH>(1, 0);
        relocated.siblings = a.siblings;
        assert_ne!(derive(&relocated).root, claim.root, "the index bit selects the side");
    }

    #[test]
    fn circuit_publishes_the_derived_claim() {
        let opening = sample::<FIXTURE_DEPTH>(1, 1);
        let (built, slot) = build(&opening);
        // commitment, nullifier, key, leaf, amount bind, then one node per level.
        assert_eq!(built.rows::<Blake3Gate>(slot).len(), 5 + FIXTURE_DEPTH);
        let claim = derive(&opening);
        let parsed = claim_from_public(&built.witness.public).expect("public claim");
        assert_eq!(parsed.nullifier, claim.nullifier);
        assert_eq!(parsed.root, claim.root);
        assert_eq!(parsed.amount, opening.v);
        assert_eq!(public_key_com(&built.witness.public), Some(key_com(&opening.epk_x)));
        let hidden_key = pack4(opening.epk_x);
        let hidden_alloc = pack4([opening.nd[0], opening.nd[1], opening.fdi, 0]);
        assert!(built.witness.public.iter().all(|word| *word != hidden_key));
        assert!(built.witness.public.iter().all(|word| *word != hidden_alloc));

        let (same_index, _) = build(&skeleton::<FIXTURE_DEPTH>(opening.index));
        assert_eq!(
            built.shape.circuit.digest(),
            same_index.shape.circuit.digest(),
            "private values are not part of the circuit shape"
        );
        let (other_index, _) = build(&skeleton::<FIXTURE_DEPTH>(opening.index ^ 1));
        assert_ne!(
            built.shape.circuit.digest(),
            other_index.shape.circuit.digest(),
            "the leaf index is part of the circuit shape"
        );
    }

    #[test]
    fn private_statement_hides_the_key_the_amount_and_the_destination() {
        let open = sample::<FIXTURE_DEPTH>(1, 1);
        let opening = PrivateOpening {
            open,
            blind: [8, 7, 6, 5, 4, 3, 2, 1],
        };
        let claim = derive_private(&opening);
        let (built, _) = build_private(&opening);
        let parsed_nf = unpack8(built.witness.public[5], built.witness.public[6]);
        assert_eq!(parsed_nf, claim.nullifier);
        assert!(built
            .witness
            .public
            .iter()
            .all(|word| *word != pack4(opening.open.epk_x)));
        assert!(built
            .witness
            .public
            .iter()
            .all(|word| *word != pack4([opening.open.v, 0, 0, 0])));

        let mut other = opening;
        other.blind[0] ^= 1;
        let moved = derive_private(&other);
        assert_ne!(moved.payout_com, claim.payout_com);
        assert_eq!(moved.root, claim.root);
        assert_eq!(moved.nullifier, claim.nullifier);
    }
}
