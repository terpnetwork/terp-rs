//! Batch egress for claim proofs.
//!
//! Each accepted claim contributes its nullifier, destination, and leaf
//! amount. Those three are hashed, then folded into one Blake3 root. The
//! public output of the circuit is that root, the rejected count, and every
//! accepted `(destination, amount)`. A full in-circuit replay of each
//! Ligerito claim proof is a later row in this same shape. This circuit is
//! the egress those rows have to match.

use flock_core::circuit::builder::{CircuitBuilder, GateType, SlotId, SlotWitness};
use flock_core::field::F128;
use flock_core::schedule::TableType;
use flock_hash::blake3_compress;
use flock_prover::r1cs_hashes::blake3::{Compression, build_block_r1cs, io_schema};

const IV: [u32; 8] = [
    0x6A09_E667, 0xBB67_AE85, 0x3C6E_F372, 0xA54F_F53A, 0x510E_527F, 0x9B05_688C, 0x1F83_D9AB,
    0x5BE0_CD19,
];

const FLAG_PAYOUT: u32 = 5;
const FLAG_BATCH: u32 = 6;

/// One accepted claim, already tied to a leaf amount.
pub struct Payout {
    pub nullifier: [u32; 8],
    pub destination: [u32; 5],
    pub amount: u32,
}

/// What the chain stores for one epoch of claims.
pub struct Egress {
    pub nullifier_root: [u32; 8],
    pub rejected: u32,
    pub payouts: Vec<([u32; 5], u32)>,
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

fn compress(message: &[u32; 16], flags: u32) -> [u32; 8] {
    let out = blake3_compress(&IV, message, 0, 64, flags);
    out[0..8].try_into().unwrap()
}

fn payout_message(payout: &Payout) -> [u32; 16] {
    let mut m = [0u32; 16];
    m[..8].copy_from_slice(&payout.nullifier);
    m[8..13].copy_from_slice(&payout.destination);
    m[15] = payout.amount;
    m
}

fn node_message(left: &[u32; 8], right: &[u32; 8]) -> [u32; 16] {
    let mut m = [0u32; 16];
    m[..8].copy_from_slice(left);
    m[8..].copy_from_slice(right);
    m
}

/// Fold accepted payouts. Rejected claims are counted and not hashed.
pub fn egress(accepted: &[Payout], rejected: u32) -> Egress {
    let mut running = [0u32; 8];
    let mut payouts = Vec::with_capacity(accepted.len());
    for payout in accepted {
        let dig = compress(&payout_message(payout), FLAG_PAYOUT);
        running = compress(&node_message(&running, &dig), FLAG_BATCH);
        payouts.push((payout.destination, payout.amount));
    }
    Egress {
        nullifier_root: running,
        rejected,
        payouts,
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

fn message_wires(
    b: &mut CircuitBuilder,
    message: &[u32; 16],
) -> [flock_core::circuit::builder::Wire; 4] {
    std::array::from_fn(|j| {
        let w: [u32; 4] = message[4 * j..4 * j + 4].try_into().unwrap();
        b.public_value(pack4(w))
    })
}

/// Circuit whose public output is the egress list and the nullifier root.
pub fn build(accepted: &[Payout], rejected: u32) -> flock_core::circuit::builder::BuiltCircuit {
    let want = egress(accepted, rejected);
    let nu = 8;
    let mut b = CircuitBuilder::new(nu);
    let g: SlotId = b.slot(Blake3Gate { nu });
    let iv = pack8(&IV);
    let cv = [b.public_value(iv[0]), b.public_value(iv[1])];
    let zero = b.public_value(F128::ZERO);
    let payout_params = b.public_value(pack_params(FLAG_PAYOUT));
    let batch_params = b.public_value(pack_params(FLAG_BATCH));
    b.public_value(F128::new(rejected as u64, 0));
    let mut running = [zero, zero];
    for payout in accepted {
        let msg = message_wires(&mut b, &payout_message(payout));
        let dig = b.gate(g, &[cv[0], cv[1], msg[0], msg[1], msg[2], msg[3], payout_params]);
        let out = b.gate(
            g,
            &[cv[0], cv[1], running[0], running[1], dig[0], dig[1], batch_params],
        );
        running = [out[0], out[1]];
    }
    let root_at = b.public_len();
    if accepted.is_empty() {
        b.publish(zero);
        b.publish(zero);
    } else {
        b.publish(running[0]);
        b.publish(running[1]);
    }
    let built = b.finish().expect("batch circuit");
    if !accepted.is_empty() {
        let got = unpack8(built.witness.public[root_at], built.witness.public[root_at + 1]);
        assert_eq!(got, want.nullifier_root, "batch root");
    }
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payout(nf0: u32, amount: u32) -> Payout {
        Payout {
            nullifier: [nf0, 2, 3, 4, 5, 6, 7, 8],
            destination: [0xaaaa, 0xbbbb, 0xcccc, 0xdddd, 0xeeee],
            amount,
        }
    }

    #[test]
    fn amount_changes_the_batch_root() {
        let a = egress(&[payout(1, 15), payout(2, 30)], 1);
        let b = egress(&[payout(1, 16), payout(2, 30)], 1);
        assert_eq!(a.payouts.len(), 2);
        assert_eq!(a.payouts[0].1, 15);
        assert_ne!(a.nullifier_root, b.nullifier_root);
        assert_eq!(a.rejected, 1);
        assert_eq!(egress(&[], 0).nullifier_root, [0u32; 8]);
    }

    #[test]
    fn circuit_publishes_the_batch_root() {
        let accepted = [payout(1, 15), payout(2, 30)];
        let _built = build(&accepted, 1);
    }
}
