//! Secp256k1 foreign-field arithmetic via ePrint 2025/695
//!
//! Replaces the old witness-only CRT approach with constrained custom gates:
//! - 4×64-bit limbs (B = 2^64)
//! - `foreign_mul` gate: x·y = z (mod q) with native + auxiliary checks
//! - `normalize` gate: reduce non-canonical limbs
//! - ECC identity gates: curve membership, λ-slope, λ-tangent, λ²
//! - GLV endomorphism for `esk·G` (`k = k1 + k2·λ`, two 128-bit chains)
//!
//! Public API preserved: `Secp256k1Chip::prove_key_pairing`, `fq_to_native`,
//! `Secp256k1Config`, `CrtInteger`, etc.

use ff::{Field, PrimeField};
use super::secp_host_field::{SecpFp as Fp, SecpFq as Fq};
use halo2_gadgets::utilities::lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig};
use halo2_proofs::circuit::Region;
use halo2_proofs::plonk::{Expression, Selector};
use halo2_proofs::poly::Rotation;
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, Value},
    plonk::{Advice, Column, ConstraintSystem, Error as PlonkError},
};
use pasta_curves::pallas;
/// secp256k1 generator (SEC 2), big-endian; in-crate so the browser build needs no libsecp.
const GENERATOR_X: [u8; 32] = [
    0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce, 0x87, 0x0b, 0x07,
    0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b, 0x16, 0xf8, 0x17, 0x98,
];
const GENERATOR_Y: [u8; 32] = [
    0x48, 0x3a, 0xda, 0x77, 0x26, 0xa3, 0xc4, 0x65, 0x5d, 0xa4, 0xfb, 0xfc, 0x0e, 0x11, 0x08, 0xa8,
    0xfd, 0x17, 0xb4, 0x48, 0xa6, 0x85, 0x54, 0x19, 0x9c, 0x47, 0xd0, 0x8f, 0xfb, 0x10, 0xd4, 0xb8,
];

use crate::spec::{
    biguint_to_fe_simple, decompose_biguint_simple, fe_to_biguint_for_field, fe_to_biguint_simple,
};

use num_bigint::{BigInt, BigUint};
use num_traits::{One, Zero};
use std::marker::PhantomData;
use std::vec::Vec;

// ============================================================================
// Constants
// ============================================================================

#[path = "secp256k1_chip/fixed_base.rs"]
mod fixed_base;
pub use fixed_base::{FB_ALWAYS_CHECKED, FB_NWIN, FB_W, KEYPAIR_GLV};

/// Number of 64-bit limbs per foreign field element.
const NUM_LIMBS: usize = 4;
/// Bits per limb.
const LIMB_BITS: usize = 64;
/// Limb base B = 2^64.
const B: u128 = 1u128 << 64;

/// secp256k1 base field modulus p.
const SECP_P_HEX: &str = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F";
/// secp256k1 scalar field modulus n.
const SECP_N_HEX: &str = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141";
// > source: https://std.neuromancer.sk/secg/secp256k1#

/// 2^256 mod secp256k1_p = 0x1000003D1
const SECP_P_REDUCTION: u64 = 0x1000003D1;

/// Montgomery ladder offset correction point: `-(2^256 * G)` on secp256k1.
///
/// The Montgomery ladder initializes `(r0, r1) = (G, 2G)` and processes 256
/// scalar bits MSB-first, maintaining the invariant `r1 - r0 = G`. After all
/// iterations, `r0 = (2^256 + scalar) * G`. To recover `scalar * G`, we add
/// the negation of `2^256 * G`.
///
/// Since the group order is `n`, the effective scalar is `2^256 mod n`:
///   `2^256 mod n = 0x14551231950B75FC4402DA1732FC9BEBF`
///
/// These coordinates are `-(s * G)` where `s = 2^256 mod n`:
///   - x-coordinate is the same as `(s * G).x`
///   - y-coordinate is `secp256k1_p - (s * G).y` (point negation)
///
/// To verify: `MONTGOMERY_OFFSET + (2^256 mod n)*G` should equal the identity.
/// A test (`test_montgomery_offset_correctness`) validates these constants.
const MONTGOMERY_OFFSET_X: [u8; 32] = [
    0x87, 0x97, 0x9a, 0xeb, 0xc4, 0x6c, 0xf7, 0x92, 0x80, 0x96, 0x59, 0x59, 0x81, 0xde, 0xbd, 0x89,
    0x8d, 0x78, 0xd3, 0xbb, 0x16, 0x97, 0x66, 0x74, 0x60, 0xa0, 0x5b, 0xef, 0xfa, 0x25, 0x36, 0xdd,
];
const MONTGOMERY_OFFSET_Y: [u8; 32] = [
    0xbc, 0x56, 0xbb, 0x39, 0xfe, 0x72, 0x09, 0xc8, 0xa6, 0xc6, 0x7c, 0xd7, 0x67, 0x9e, 0xeb, 0x6b,
    0x35, 0xce, 0xa8, 0xfb, 0xfe, 0xda, 0x25, 0x9e, 0x2b, 0xcf, 0xf1, 0xad, 0x5c, 0x70, 0xe7, 0x85,
];

/// secp256k1 GLV eigenvalue λ (`scalar * (x, y) = (βx, y)`). Bitcoin Core `secp256k1_const_lambda`.
const GLV_LAMBDA_HEX: &str = "5363AD4CC05C30E0A5261C028812645A122E22EA20816678DF02967C1B23BD72";
/// Cube root of unity on the base field, β. Bitcoin Core endomorphism.
const GLV_BETA_HEX: &str = "7AE96A2B657C07106E64479EAC3434E99CF0497512F58995C1396C28719501EE";
/// `round(2^384 · b2 / n)` from `secp256k1_scalar_split_lambda`.
const GLV_G1_HEX: &str = "3086D221A7D46BCDE86C90E49284EB153DAA8A1471E8CA7FE893209A45DBB031";
/// `round(2^384 · (−b1) / n)`.
const GLV_G2_HEX: &str = "E4437ED6010E88286F547FA90ABFE4C4221208AC9DF506C61571B4AE8AC47F71";
const GLV_MINUS_B1_HEX: &str = "E4437ED6010E88286F547FA90ABFE4C3";
const GLV_MINUS_B2_HEX: &str = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE8A280AC50774346DD765CDA83DB1562C";

fn parse_hex_uint(hex_be: &str) -> BigUint {
    BigUint::parse_bytes(hex_be.as_bytes(), 16).expect("hex constant")
}

fn secp_n() -> BigUint {
    parse_hex_uint(SECP_N_HEX)
}

fn secp_p() -> BigUint {
    parse_hex_uint(SECP_P_HEX)
}

fn biguint_to_be32(v: &BigUint) -> [u8; 32] {
    let bytes = v.to_bytes_be();
    assert!(bytes.len() <= 32, "integer does not fit in 32 bytes");
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    out
}

fn be32_to_fq(be: &[u8; 32]) -> Secp256k1Fq {
    secp_fq_from_secret_be(be)
}

fn be32_to_fp(be: &[u8; 32]) -> Secp256k1Fp {
    secp_fp_from_coord_be(be)
}

/// One GLV half: `residue ≡ ±mag (mod n)` with `mag < 2^128`.
#[derive(Clone, Debug)]
pub(crate) struct GlvHalf {
    pub mag: BigUint,
    pub neg: bool,
    /// Representative in `[0, n)`.
    pub residue: BigUint,
}

/// `k1 + k2·λ ≡ k (mod n)`, both magnitudes under the secp256k1 GLV bounds.
#[derive(Clone, Debug)]
pub(crate) struct GlvSplit {
    pub k1: GlvHalf,
    pub k2: GlvHalf,
}

fn glv_half(residue: BigUint, n: &BigUint) -> GlvHalf {
    let neg_r = if residue.is_zero() {
        BigUint::zero()
    } else {
        n - &residue
    };
    if residue <= neg_r {
        GlvHalf {
            mag: residue.clone(),
            neg: false,
            residue,
        }
    } else {
        GlvHalf {
            mag: neg_r,
            neg: true,
            residue,
        }
    }
}

/// Rounded shift `round(k·g / 2^384)`, matching `secp256k1_scalar_mul_shift_var(..., 384)`.
fn mul_shift_384(k: &BigUint, g: &BigUint) -> BigUint {
    let prod = k * g;
    let round = (&prod >> 383usize) & BigUint::one();
    (&prod >> 384usize) + round
}

/// Bitcoin Core `secp256k1_scalar_split_lambda`.
pub(crate) fn glv_split(k: &BigUint) -> GlvSplit {
    let n = secp_n();
    let k = k % &n;
    let lambda = parse_hex_uint(GLV_LAMBDA_HEX);
    let c1 =
        (mul_shift_384(&k, &parse_hex_uint(GLV_G1_HEX)) * parse_hex_uint(GLV_MINUS_B1_HEX)) % &n;
    let c2 =
        (mul_shift_384(&k, &parse_hex_uint(GLV_G2_HEX)) * parse_hex_uint(GLV_MINUS_B2_HEX)) % &n;
    let r2 = (&c1 + &c2) % &n;
    let r2_lambda = (&r2 * &lambda) % &n;
    let r1 = if &k >= &r2_lambda {
        &k - &r2_lambda
    } else {
        &k + &n - &r2_lambda
    };
    let split = GlvSplit {
        k1: glv_half(r1, &n),
        k2: glv_half(r2, &n),
    };
    assert!(split.k1.mag.bits() <= 128, "GLV k1 escaped 128 bits");
    assert!(split.k2.mag.bits() <= 128, "GLV k2 escaped 128 bits");
    split
}

fn glv_lambda_fq() -> Secp256k1Fq {
    be32_to_fq(&biguint_to_be32(&parse_hex_uint(GLV_LAMBDA_HEX)))
}

fn glv_beta_fp() -> Secp256k1Fp {
    be32_to_fp(&biguint_to_be32(&parse_hex_uint(GLV_BETA_HEX)))
}

fn generator_xy() -> (Secp256k1Fp, Secp256k1Fp) {
    let mut gx = GENERATOR_X;
    let mut gy = GENERATOR_Y;
    gx.reverse();
    gy.reverse();
    (secp_fp_from_le(gx), secp_fp_from_le(gy))
}

fn host_invert(x: Secp256k1Fp) -> Secp256k1Fp {
    Option::from(Field::invert(&x)).expect("nonzero field element")
}

fn host_neg_point(p: (Secp256k1Fp, Secp256k1Fp)) -> (Secp256k1Fp, Secp256k1Fp) {
    (p.0, -p.1)
}

/// Affine add. Incomplete: caller must not pass equal or inverse points.
fn host_add(
    p: (Secp256k1Fp, Secp256k1Fp),
    q: (Secp256k1Fp, Secp256k1Fp),
) -> (Secp256k1Fp, Secp256k1Fp) {
    let lam = (q.1 - p.1) * host_invert(q.0 - p.0);
    let x3 = lam * lam - p.0 - q.0;
    let y3 = lam * (p.0 - x3) - p.1;
    (x3, y3)
}

fn host_double(p: (Secp256k1Fp, Secp256k1Fp)) -> (Secp256k1Fp, Secp256k1Fp) {
    let lam = (Secp256k1Fp::from(3u64) * p.0 * p.0) * host_invert(p.1 + p.1);
    let x3 = lam * lam - p.0 - p.0;
    let y3 = lam * (p.0 - x3) - p.1;
    (x3, y3)
}

/// MSB double-and-add. `None` is the point at infinity (`k = 0`).
fn host_mul(point: (Secp256k1Fp, Secp256k1Fp), k: &BigUint) -> Option<(Secp256k1Fp, Secp256k1Fp)> {
    if k.is_zero() {
        return None;
    }
    let mut acc: Option<(Secp256k1Fp, Secp256k1Fp)> = None;
    for bit in k.to_radix_be(2) {
        if let Some(current) = acc {
            acc = Some(host_double(current));
        }
        if bit == 1 {
            acc = Some(match acc {
                None => point,
                Some(current) => host_add(current, point),
            });
        }
    }
    acc
}

fn host_psi(g: (Secp256k1Fp, Secp256k1Fp)) -> (Secp256k1Fp, Secp256k1Fp) {
    (glv_beta_fp() * g.0, g.1)
}

/// `k·G` via the GLV halves. Host-side check used by soundness tests.
pub(crate) fn glv_mul_generator(k: &BigUint) -> Option<(Secp256k1Fp, Secp256k1Fp)> {
    let split = glv_split(k);
    let g = generator_xy();
    let psi = host_psi(g);
    let b1 = if split.k1.neg { host_neg_point(g) } else { g };
    let b2 = if split.k2.neg {
        host_neg_point(psi)
    } else {
        psi
    };
    let p1 = host_mul(b1, &split.k1.mag);
    let p2 = host_mul(b2, &split.k2.mag);
    match (p1, p2) {
        (None, None) => None,
        (Some(p), None) | (None, Some(p)) => Some(p),
        (Some(a), Some(b)) => Some(host_add(a, b)),
    }
}

fn foreign_limbs<F: PrimeField>(v: &F) -> [pallas::Base; 4] {
    let big = fe_to_biguint_host(v);
    let decomposed = decompose_biguint_simple(&big, 4, 64);
    [decomposed[0], decomposed[1], decomposed[2], decomposed[3]]
}

fn base_as_u128(v: pallas::Base) -> u128 {
    let repr = v.to_repr();
    let bytes: &[u8] = repr.as_ref();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    u128::from_le_bytes(buf)
}

/// Compute native B-power coefficients: c[k] = B^k as a pallas::Base element.
///
/// For the native check, the integer identity x*y - z = u*q is evaluated
/// mod pallas_p. The coefficients are simply B^k (powers of the limb base),
/// automatically reduced mod pallas_p when converted to field elements.
///
/// NOTE: These are NOT B^k mod foreign_q. The foreign modulus reduction
/// (c[k] = B^k mod q) would be used in auxiliary checks, not the native check.
fn secp_fp_c_coeffs() -> [pallas::Base; 7] {
    let b = BigUint::from(B);
    let mut c = [pallas::Base::ZERO; 7];
    let mut pow = BigUint::one();
    for i in 0..7 {
        c[i] = biguint_to_fe_simple(&pow);
        pow *= &b;
    }
    c
}

/// Compute native B-power coefficients for Fq operations.
/// Same as secp_fp_c_coeffs — the native check uses B^k mod pallas_p
/// regardless of which foreign field we are emulating.
fn secp_fq_c_coeffs() -> [pallas::Base; 7] {
    secp_fp_c_coeffs()
}

// ============================================================================
// Types (preserving public API compatibility)
// ============================================================================

/// Type alias for secp256k1 base field (Fp).
pub type Secp256k1Fp = Fp;
/// Type alias for secp256k1 Fp chip.
pub type Secp256k1FpChip = FpChip<Secp256k1Fp>;
/// Type alias for secp256k1 scalar field (Fq).
pub type Secp256k1Fq = Fq;
/// Type alias for secp256k1 Fq chip.
pub type Secp256k1FqChip = FpChip<Secp256k1Fq>;

/// Load a secp256k1 scalar (`Fq`) from **big-endian** 32-byte secret-key encoding
/// (`secp256k1::SecretKey::secret_bytes`).
///
/// `halo2curves` `Fq::from_repr` / `from_bytes` expect **little-endian** limbs; callers
/// must reverse BE host encodings before use (see `test_secp256k1_key_pairing_valid`).
/// Pure-Rust `sk·G` (big-endian x, y). None for sk = 0 or sk >= n.
pub fn secp_pubkey_be_from_secret_be(sk_be: &[u8; 32]) -> Option<([u8; 32], [u8; 32])> {
    let k = BigUint::from_bytes_be(sk_be);
    if k.is_zero() || k >= secp_n() {
        return None;
    }
    let p = host_mul(generator_xy(), &k)?;
    let be = |v: &Secp256k1Fp| {
        let b = fe_to_biguint_host(v).to_bytes_be();
        let mut out = [0u8; 32];
        out[32 - b.len()..].copy_from_slice(&b);
        out
    };
    Some((be(&p.0), be(&p.1)))
}

pub fn secp_fq_from_secret_be(be: &[u8; 32]) -> Secp256k1Fq {
    let mut le = *be;
    le.reverse();
    secp_fq_from_le(le)
}

pub(crate) fn secp_fq_from_le(le: [u8; 32]) -> Secp256k1Fq {
    Option::from(<Secp256k1Fq as PrimeField>::from_repr(le))
        .expect("canonical secp256k1 scalar (Fq)")
}

/// Load a secp256k1 base-field coordinate (`Fp`) from **big-endian** 32-byte encoding
/// (`PublicKey::serialize_uncompressed` x/y).
pub fn secp_fp_from_coord_be(be: &[u8; 32]) -> Secp256k1Fp {
    let mut le = *be;
    le.reverse();
    secp_fp_from_le(le)
}

pub(crate) fn secp_fp_from_le(le: [u8; 32]) -> Secp256k1Fp {
    Option::from(<Secp256k1Fp as PrimeField>::from_repr(le)).expect("canonical secp256k1 Fp")
}

pub(crate) fn fe_to_biguint_host<F: PrimeField>(fe: &F) -> BigUint {
    BigUint::from_bytes_le(F::to_repr(fe).as_ref())
}

/// Pallas-base reduction of a BE secp scalar (same integer as [`secp_fq_from_secret_be`]).
pub fn secp_secret_be_to_pallas_base(be: &[u8; 32]) -> pallas::Base {
    biguint_to_fe_simple(&BigUint::from_bytes_be(be))
}

/// Pallas-base reduction of a BE secp affine coordinate.
pub fn secp_coord_be_to_pallas_base(be: &[u8; 32]) -> pallas::Base {
    biguint_to_fe_simple(&BigUint::from_bytes_be(be))
}

/// An integer represented as limbs with possible overflow (kept for API compat).
#[derive(Clone, Debug)]
pub struct OverflowInteger<F: ff::Field> {
    pub limbs: Vec<AssignedCell<F, F>>,
    pub max_limb_bits: usize,
}

impl<F: ff::Field> OverflowInteger<F> {
    pub fn new(limbs: Vec<AssignedCell<F, F>>, max_limb_bits: usize) -> Self {
        Self {
            limbs,
            max_limb_bits,
        }
    }
    pub fn num_limbs(&self) -> usize {
        self.limbs.len()
    }
}

/// A "proper" unsigned integer where each limb is in [0, 2^limb_bits).
#[derive(Clone, Debug)]
pub struct ProperUint<F: ff::Field> {
    pub limbs: Vec<AssignedCell<F, F>>,
}

impl<F: ff::Field> ProperUint<F> {
    pub fn new(limbs: Vec<AssignedCell<F, F>>) -> Self {
        Self { limbs }
    }
    pub fn num_limbs(&self) -> usize {
        self.limbs.len()
    }
    pub fn into_overflow(self, limb_bits: usize) -> OverflowInteger<F> {
        OverflowInteger::new(self.limbs, limb_bits)
    }
}

/// CRT representation of an integer — the key type consumed by downstream chips.
///
/// `truncation`: limb representation (value mod 2^t)
/// `native`: value mod pallas modulus (used by Poseidon/Sinsemilla)
/// `value`: actual integer (witness only)
#[derive(Clone, Debug)]
pub struct CrtInteger<F: ff::Field> {
    pub truncation: OverflowInteger<F>,
    pub native: AssignedCell<F, F>,
    pub value: Value<BigInt>,
}

impl<F: ff::Field> CrtInteger<F> {
    pub fn new(
        truncation: OverflowInteger<F>,
        native: AssignedCell<F, F>,
        value: Value<BigInt>,
    ) -> Self {
        Self {
            truncation,
            native,
            value,
        }
    }
    pub fn num_limbs(&self) -> usize {
        self.truncation.num_limbs()
    }
}

pub type ProperCrtUint<F> = CrtInteger<F>;

/// Fixed (constant) representation of a BigUint as limbs.
#[derive(Clone, Debug)]
pub struct FixedOverflowInteger<F> {
    pub limbs: Vec<F>,
}

impl FixedOverflowInteger<pallas::Base> {
    pub fn from_native(value: &BigUint, num_limbs: usize, limb_bits: usize) -> Self {
        let limbs = decompose_biguint_simple(value, num_limbs, limb_bits);
        Self { limbs }
    }
    pub fn to_biguint(&self, limb_bits: usize) -> BigUint {
        self.limbs.iter().rev().fold(BigUint::zero(), |acc, x| {
            (acc << limb_bits) + fe_to_biguint_simple(x)
        })
    }
}

/// Get the modulus of a prime field.
pub fn modulus_simple_pallas() -> BigUint {
    fe_to_biguint_simple(&-pallas::Base::ONE) + 1u64
}

pub fn modulus_simple_secp_fp() -> BigUint {
    let neg_one = {
        -Fp::ONE
    };
    BigUint::from_bytes_le(<Secp256k1Fp as PrimeField>::to_repr(&neg_one).as_ref()) + 1u64
}

// ============================================================================
// FpConfig — configuration for foreign field arithmetic
// ============================================================================

/// TEST-ONLY adversarial witness hooks for the sound foreign-field chip. A malicious prover can
/// witness anything; these flip one witness so the negative tests can show the gates reject it.
#[cfg(test)]
pub mod forge {
    use core::sync::atomic::{AtomicU8, Ordering};
    pub const NONE: u8 = 0;
    /// `add` output + 1 (identity broken)
    pub const ADD_OUT: u8 = 1;
    /// `sub` output forced to 0 (the old forgery)
    pub const SUB_OUT: u8 = 2;
    /// `mul` output + 1 (identity broken)
    pub const MUL_OUT: u8 = 3;
    /// `mul` quotient limb 0 += 2^70, limb 1 -= 2^6 (integer identity kept, limb out of range)
    pub const MUL_Q_RANGE: u8 = 4;
    /// `mul` output limb 0 += 2^64, limb 1 -= 1 (integer identity kept, r limb out of range)
    pub const MUL_R_RANGE: u8 = 5;
    /// pinned constants witnessed with limb 0 + 1 (copy of the constant differs from the fixed value)
    pub const CONST: u8 = 6;
    /// secret key witnessed as sk + n (same point, different integer / nullifier)
    pub const SK_PLUS_N: u8 = 7;
    /// GLV split witnessed with k1.mag + 1
    pub const GLV_K1: u8 = 8;
    /// one-shot sub forgery used only by `enforce_equal` (the original FORGE_EQ attack)
    pub const EQ_SUB: u8 = 9;
    pub const FB_DIGIT: u8 = 10;
    pub const FB_POINT: u8 = 11;
    /// window-1 lookup witnessed as the accumulator itself (acc == T, dx = dy = 0: plain div accepts any lambda)
    pub const FB_COLLIDE: u8 = 12;
    pub static MODE: AtomicU8 = AtomicU8::new(NONE);
    pub fn set(m: u8) {
        MODE.store(m, Ordering::SeqCst)
    }
    pub fn is(m: u8) -> bool {
        MODE.load(Ordering::SeqCst) == m
    }
}
#[cfg(not(test))]
#[allow(dead_code)]
pub(crate) mod forge {
    pub const ADD_OUT: u8 = 1;
    pub const SUB_OUT: u8 = 2;
    pub const MUL_OUT: u8 = 3;
    pub const MUL_Q_RANGE: u8 = 4;
    pub const MUL_R_RANGE: u8 = 5;
    pub const CONST: u8 = 6;
    pub const SK_PLUS_N: u8 = 7;
    pub const GLV_K1: u8 = 8;
    pub const EQ_SUB: u8 = 9;
    pub const FB_DIGIT: u8 = 10;
    pub const FB_POINT: u8 = 11;
    pub const FB_COLLIDE: u8 = 12;
    #[inline(always)]
    pub fn is(_m: u8) -> bool {
        false
    }
}

/// Lab switch: configure the ECDSA claim circuit with the fixed-base table (u1·G).
pub static ECDSA_FIXED_BASE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Lab switch: use the revived Montgomery ladder instead of GLV in `prove_key_pairing`.
pub static KEYPAIR_LADDER: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Kept for the old test name; now maps onto `forge::EQ_SUB`.
#[cfg(test)]
#[derive(Debug)] pub struct ForgeEqCompat;
#[cfg(test)]
impl ForgeEqCompat {
    pub fn store(&self, v: bool, _o: core::sync::atomic::Ordering) {
        forge::set(if v { forge::EQ_SUB } else { 0 })
    }
}
#[cfg(test)]
pub static FORGE_EQ: ForgeEqCompat = ForgeEqCompat;

// ============================================================================
// Sound foreign-field arithmetic (CRT limbs, carries checked per limb)
// ============================================================================
//
// Representation: x = Σ x_i·2^(64 i), i = 0..3, every limb range-checked to [0, 2^64) by the
// 10-bit lookup (6 words + a 4-bit short check on the residual running sum). The native cell
// is constrained to Σ x_i·2^(64 i) mod r (r = Pallas base modulus) in the same gate that
// produces the limbs, so `to_native` is bound to the limbs.
//
// mul:  a·b = q·p + r over the integers. With t_k = Σ_{i+j=k}(a_i b_j - q_i p_j) - r_k
//       (r_k = 0 for k ≥ 4), the gate checks, for k = 0..6,
//           t_k + c_{k-1} = c_k·2^64,   c_{-1} = 0, c_6 = 0,
//       with carries c_k = c'_k - 2^79, c'_k ∈ [0, 2^80) (8 lookup words, strict), q_i ∈ [0, 2^70)
//       (7 words, strict), r_i ∈ [0, 2^64). |t_k| < 4·2^128 + 4·2^134 + 2^64 < 2^137 and
//       |c_k·2^64| < 2^143, so every per-limb equation is < 2^144 ≪ r ≈ 2^254 in absolute
//       value: it holds over Z, and telescoping gives Σ t_k 2^(64k) = 0 over Z, i.e. a·b = q·p + r.
//       The gate also checks a·b - q·p - r ≡ 0 (mod r) on the recomposed natives (redundant).
// add:  a + b = q·p + r, q ∈ {0,1}; sub: a - b + q·p = r, q ∈ {0,1}; carries c_k ∈ {-1,0,1},
//       c_3 = 0. Per-limb terms < 2^66.
// lt:   a < m (m = p or n): a + d = m - 1 over Z with d limbs range-checked, boolean carries.
// select: r_i = b_i + c·(a_i - b_i), c boolean, on limbs and native.

#[derive(Clone, Debug)]
pub struct FpConfig {
    pub advices: [Column<Advice>; 9],
    pub q_compose: Selector,
    pub q_mul: Selector,
    pub q_add: Selector,
    pub q_sub: Selector,
    pub q_lt: Selector,
    pub q_select: Selector,
    pub range_check: LookupRangeCheckConfig<pallas::Base, 10>,
    pub modulus: BigUint,
}

const CARRY_OFFSET_BITS: usize = 79;
const CARRY_RC_WORDS: usize = 8; // 80 bits
const Q_RC_WORDS: usize = 7; // 70 bits

fn fe_big(v: &BigUint) -> pallas::Base {
    biguint_to_fe_simple(&(v % modulus_simple_pallas()))
}
fn fe_int(v: &BigInt) -> pallas::Base {
    let m = BigInt::from(modulus_simple_pallas());
    let r = ((v % &m) + &m) % &m;
    biguint_to_fe_simple(&r.to_biguint().unwrap())
}
fn floor_div64(t: &BigInt) -> BigInt {
    let d = BigInt::one() << 64usize;
    let q = t / &d;
    if t < &BigInt::zero() && (&q * &d) != *t { q - 1 } else { q }
}
fn limbs4(v: &BigUint) -> [BigUint; 4] {
    let mask = (BigUint::one() << 64usize) - 1u32;
    core::array::from_fn(|i| (v >> (64 * i)) & &mask)
}

impl FpConfig {
    pub fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        advices: [Column<Advice>; 9],
        range_check: LookupRangeCheckConfig<pallas::Base, 10>,
        _c_coeffs: [pallas::Base; 7],
        _q_native: pallas::Base,
        modulus: BigUint,
    ) -> Self {
        for a in advices.iter() {
            meta.enable_equality(*a);
        }
        let q_compose = meta.selector();
        let q_mul = meta.selector();
        let q_add = meta.selector();
        let q_sub = meta.selector();
        let q_lt = meta.selector();
        let q_select = meta.selector();
        let c = |v: pallas::Base| Expression::Constant(v);
        let bk: [pallas::Base; 4] =
            core::array::from_fn(|i| fe_big(&(BigUint::one() << (64 * i))));
        let b64 = fe_big(&(BigUint::one() << 64usize));
        let p_limbs: [pallas::Base; 4] = limbs4(&modulus).map(|l| fe_big(&l));
        let p_native = fe_big(&modulus);
        let m1_limbs: [pallas::Base; 4] = limbs4(&(&modulus - 1u32)).map(|l| fe_big(&l));
        let off = fe_big(&(BigUint::one() << CARRY_OFFSET_BITS));
        let recompose = |l: &[Expression<pallas::Base>]| {
            l.iter()
                .zip(bk.iter())
                .fold(c(pallas::Base::ZERO), |acc, (x, b)| acc + x.clone() * c(*b))
        };

        meta.create_gate("ff compose", |meta| {
            let s = meta.query_selector(q_compose);
            let l: Vec<_> = (0..4).map(|i| meta.query_advice(advices[i], Rotation::cur())).collect();
            let n = meta.query_advice(advices[4], Rotation::cur());
            vec![s * (n - recompose(&l))]
        });

        meta.create_gate("ff mul", |meta| {
            let s = meta.query_selector(q_mul);
            let a: Vec<_> = (0..4).map(|i| meta.query_advice(advices[i], Rotation::cur())).collect();
            let b: Vec<_> = (0..4).map(|i| meta.query_advice(advices[4 + i], Rotation::cur())).collect();
            let q: Vec<_> = (0..4).map(|i| meta.query_advice(advices[i], Rotation::next())).collect();
            let r: Vec<_> = (0..4).map(|i| meta.query_advice(advices[4 + i], Rotation::next())).collect();
            let rn = meta.query_advice(advices[8], Rotation::next());
            let cr: Vec<_> = (0..6)
                .map(|i| meta.query_advice(advices[i], Rotation(2)) - c(off))
                .collect();
            let mut out = Vec::new();
            for k in 0..7 {
                let mut t = c(pallas::Base::ZERO);
                for i in 0..4 {
                    if k >= i && k - i < 4 {
                        let j = k - i;
                        t = t + a[i].clone() * b[j].clone() - q[i].clone() * c(p_limbs[j]);
                    }
                }
                if k < 4 {
                    t = t - r[k].clone();
                }
                if k > 0 {
                    t = t + cr[k - 1].clone();
                }
                if k < 6 {
                    t = t - cr[k].clone() * c(b64);
                }
                out.push(s.clone() * t);
            }
            // redundant native-modulus check and binding of r's native cell
            out.push(
                s.clone()
                    * (recompose(&a) * recompose(&b) - recompose(&q) * c(p_native) - rn.clone()),
            );
            out.push(s * (rn - recompose(&r)));
            out
        });

        for (name, sel, sign) in [("ff add", q_add, false), ("ff sub", q_sub, true)] {
            meta.create_gate(name, |meta| {
                let s = meta.query_selector(sel);
                let a: Vec<_> = (0..4).map(|i| meta.query_advice(advices[i], Rotation::cur())).collect();
                let b: Vec<_> = (0..4).map(|i| meta.query_advice(advices[4 + i], Rotation::cur())).collect();
                let qb = meta.query_advice(advices[8], Rotation::cur());
                let r: Vec<_> = (0..4).map(|i| meta.query_advice(advices[i], Rotation::next())).collect();
                let cr: Vec<_> = (0..3)
                    .map(|i| meta.query_advice(advices[4 + i], Rotation::next()))
                    .collect();
                let rn = meta.query_advice(advices[8], Rotation::next());
                let one = c(pallas::Base::ONE);
                let mut out = vec![s.clone() * qb.clone() * (qb.clone() - one.clone())];
                for k in 0..3 {
                    out.push(
                        s.clone()
                            * cr[k].clone()
                            * (cr[k].clone() - one.clone())
                            * (cr[k].clone() - c(pallas::Base::from(2))),
                    );
                }
                for k in 0..4 {
                    // add: a + b - q p - r ; sub: a - b + q p - r
                    let mut t = if sign {
                        a[k].clone() - b[k].clone() + qb.clone() * c(p_limbs[k]) - r[k].clone()
                    } else {
                        a[k].clone() + b[k].clone() - qb.clone() * c(p_limbs[k]) - r[k].clone()
                    };
                    if k > 0 {
                        t = t + cr[k - 1].clone() - one.clone();
                    }
                    if k < 3 {
                        t = t - (cr[k].clone() - one.clone()) * c(b64);
                    }
                    out.push(s.clone() * t);
                }
                out.push(s * (rn - recompose(&r)));
                out
            });
        }

        meta.create_gate("ff lt const", |meta| {
            let s = meta.query_selector(q_lt);
            let a: Vec<_> = (0..4).map(|i| meta.query_advice(advices[i], Rotation::cur())).collect();
            let d: Vec<_> = (0..4).map(|i| meta.query_advice(advices[4 + i], Rotation::cur())).collect();
            let cr: Vec<_> = (0..3).map(|i| meta.query_advice(advices[i], Rotation::next())).collect();
            let one = c(pallas::Base::ONE);
            let mut out = Vec::new();
            for k in 0..3 {
                out.push(s.clone() * cr[k].clone() * (cr[k].clone() - one.clone()));
            }
            for k in 0..4 {
                let mut t = a[k].clone() + d[k].clone() - c(m1_limbs[k]);
                if k > 0 {
                    t = t + cr[k - 1].clone();
                }
                if k < 3 {
                    t = t - cr[k].clone() * c(b64);
                }
                out.push(s.clone() * t);
            }
            out
        });

        meta.create_gate("ff select", |meta| {
            let s = meta.query_selector(q_select);
            let cond = meta.query_advice(advices[8], Rotation::cur());
            let one = c(pallas::Base::ONE);
            let mut out = vec![s.clone() * cond.clone() * (cond.clone() - one)];
            let pick = |a: Expression<pallas::Base>, b: Expression<pallas::Base>, r: Expression<pallas::Base>| {
                r - b.clone() - cond.clone() * (a - b)
            };
            for k in 0..4 {
                let a = meta.query_advice(advices[k], Rotation::cur());
                let b = meta.query_advice(advices[4 + k], Rotation::cur());
                let r = meta.query_advice(advices[k], Rotation::next());
                out.push(s.clone() * pick(a, b, r));
            }
            let an = meta.query_advice(advices[4], Rotation::next());
            let bn = meta.query_advice(advices[5], Rotation::next());
            let rn = meta.query_advice(advices[6], Rotation::next());
            out.push(s * pick(an, bn, rn));
            out
        });

        Self { advices, q_compose, q_mul, q_add, q_sub, q_lt, q_select, range_check, modulus }
    }
}

/// Trait for foreign field instructions (preserved for API compatibility).
pub trait FpInstructions<Fp: PrimeField> {
    fn load_constant(&self, layouter: impl Layouter<pallas::Base>, v: Fp) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn load_private(&self, layouter: impl Layouter<pallas::Base>, value: Value<Fp>) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn range_check_limbs(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError>;
    fn to_native(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError>;
    fn enforce_zero(&self, layouter: impl Layouter<pallas::Base>, num: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError>;
    fn enforce_equal(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError>;
    fn add(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn sub(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn mul(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn div(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
}

#[derive(Clone, Debug)]
pub struct FpChip<Fp: PrimeField> {
    pub config: FpConfig,
    pub limb_bits: usize,
    pub num_limbs: usize,
    pub p_biguint: BigUint,
    pub native_modulus: BigUint,
    pub c_coeffs: [pallas::Base; 7],
    _marker: PhantomData<Fp>,
}

type Cell = AssignedCell<pallas::Base, pallas::Base>;

impl<Fp: PrimeField> FpChip<Fp> {
    pub fn construct(config: FpConfig, limb_bits: usize, num_limbs: usize, c_coeffs: [pallas::Base; 7], modulus: &BigUint) -> Self {
        assert_eq!(limb_bits, LIMB_BITS);
        assert_eq!(num_limbs, NUM_LIMBS);
        assert_eq!(&config.modulus, modulus);
        Self {
            config,
            limb_bits,
            num_limbs,
            p_biguint: modulus.clone(),
            native_modulus: modulus_simple_pallas(),
            c_coeffs,
            _marker: PhantomData,
        }
    }

    // ── range checks (lookup, exact bit widths) ──

    /// `cell < 2^bits`: `bits/10` lookup words plus a short lookup on the residual.
    pub fn rc_bits(&self, mut layouter: impl Layouter<pallas::Base>, cell: &Cell, bits: usize) -> Result<(), PlonkError> {
        let words = bits / 10;
        let rem = bits % 10;
        if rem == 0 {
            self.config.range_check.copy_check(layouter.namespace(|| "rc"), cell.clone(), words, true)?;
        } else {
            let zs = self.config.range_check.copy_check(layouter.namespace(|| "rc"), cell.clone(), words, false)?;
            self.config.range_check.copy_short_check(layouter.namespace(|| "rc top"), zs[words].clone(), rem)?;
        }
        Ok(())
    }

    fn rc_limbs(&self, mut layouter: impl Layouter<pallas::Base>, limbs: &[Cell]) -> Result<(), PlonkError> {
        for (i, l) in limbs.iter().enumerate() {
            self.rc_bits(layouter.namespace(|| format!("limb {i}")), l, LIMB_BITS)?;
        }
        Ok(())
    }

    /// Assign limbs (cols 0..3) and native (col 4) in one row under the compose gate.
    fn compose_row(&self, region: &mut Region<'_, pallas::Base>, row: usize, v: &Value<BigUint>) -> Result<(Vec<Cell>, Cell), PlonkError> {
        self.config.q_compose.enable(region, row)?;
        let mut limbs = Vec::with_capacity(4);
        for i in 0..4 {
            limbs.push(region.assign_advice(|| format!("limb {i}"), self.config.advices[i], row, || v.as_ref().map(|v| fe_big(&limbs4(v)[i])))?);
        }
        let n = region.assign_advice(|| "native", self.config.advices[4], row, || v.as_ref().map(fe_big))?;
        Ok((limbs, n))
    }

    /// Witness an integer < 2^256 as range-checked limbs with a bound native cell.
    pub fn assign_biguint(&self, mut layouter: impl Layouter<pallas::Base>, v: Value<BigUint>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let (limbs, n) = layouter.assign_region(|| "ff witness", |mut region| self.compose_row(&mut region, 0, &v))?;
        self.rc_limbs(layouter.namespace(|| "rc witness"), &limbs)?;
        Ok(CrtInteger::new(OverflowInteger::new(limbs, LIMB_BITS), n, v.map(BigInt::from)))
    }

    pub fn load_constant(&self, mut layouter: impl Layouter<pallas::Base>, v: Fp) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.load_constant_big(layouter.namespace(|| "const"), fe_to_biguint_host(&v))
    }

    /// Constant pinned to the fixed column limb by limb (constrain_constant); native bound by gate.
    pub fn load_constant_big(&self, mut layouter: impl Layouter<pallas::Base>, value: BigUint) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let expected = limbs4(&value).map(|l| fe_big(&l));
        let mut witness = value.clone();
        if forge::is(forge::CONST) {
            witness += 1u32;
        }
        let (limbs, n) = layouter.assign_region(
            || "ff constant",
            |mut region| {
                let (limbs, n) = self.compose_row(&mut region, 0, &Value::known(witness.clone()))?;
                for (l, e) in limbs.iter().zip(expected.iter()) {
                    region.constrain_constant(l.cell(), *e)?;
                }
                Ok((limbs, n))
            },
        )?;
        Ok(CrtInteger::new(OverflowInteger::new(limbs, LIMB_BITS), n, Value::known(BigInt::from(value))))
    }

    pub fn load_private(&self, mut layouter: impl Layouter<pallas::Base>, value: Value<Fp>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.assign_biguint(layouter.namespace(|| "private"), value.map(|v| fe_to_biguint_host(&v)))
    }

    /// Every CrtInteger produced by this chip is range-checked at creation; kept for API compat.
    pub fn range_check_limbs(&self, _layouter: impl Layouter<pallas::Base>, _a: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> {
        Ok(())
    }

    pub fn to_native(&self, _layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>) -> Result<Cell, PlonkError> {
        Ok(a.native.clone())
    }

    fn val(a: &ProperCrtUint<pallas::Base>) -> Value<BigUint> {
        a.value.as_ref().map(|v| v.to_biguint().expect("non-negative"))
    }

    fn copy4(region: &mut Region<'_, pallas::Base>, cols: &[Column<Advice>], row: usize, a: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> {
        for (i, l) in a.truncation.limbs.iter().enumerate() {
            l.copy_advice(|| "limb", region, cols[i], row)?;
        }
        Ok(())
    }

    /// a·b = q·p + r over Z (carry chain per limb + native check). Output r < 2^256, ≡ a·b (mod p).
    pub fn mul(&self, mut layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let p = self.p_biguint.clone();
        let p_l = limbs4(&p);
        let wit = Self::val(a).zip(Self::val(b)).map(|(a, b)| {
            let prod = &a * &b;
            let mut q = BigInt::from(&prod / &p);
            let mut r = BigInt::from(&prod % &p);
            if forge::is(forge::MUL_OUT) {
                r += 1;
            }
            let mut ql: [BigInt; 4] = core::array::from_fn(|i| BigInt::from(limbs4(&q.to_biguint().unwrap())[i].clone()));
            let mut rl: [BigInt; 4] = core::array::from_fn(|i| BigInt::from(limbs4(&r.to_biguint().unwrap())[i].clone()));
            if forge::is(forge::MUL_Q_RANGE) {
                ql[0] += BigInt::one() << 70usize;
                ql[1] -= BigInt::from(64);
            }
            if forge::is(forge::MUL_R_RANGE) {
                rl[0] += BigInt::one() << 64usize;
                rl[1] -= BigInt::one();
            }
            let _ = (&mut q, &mut r);
            let al = limbs4(&a).map(BigInt::from);
            let bl = limbs4(&b).map(BigInt::from);
            let pl: [BigInt; 4] = core::array::from_fn(|i| BigInt::from(p_l[i].clone()));
            let mut carries = Vec::with_capacity(6);
            let mut cprev = BigInt::zero();
            for k in 0..6 {
                let mut t = BigInt::zero();
                for i in 0..4 {
                    if k >= i && k - i < 4 {
                        t += &al[i] * &bl[k - i] - &ql[i] * &pl[k - i];
                    }
                }
                if k < 4 {
                    t -= &rl[k];
                }
                t += &cprev;
                // exact division in the honest case; floor otherwise (forged rows then fail)
                let c = floor_div64(&t);
                carries.push(&c + (BigInt::one() << CARRY_OFFSET_BITS));
                cprev = c;
            }
            (ql, rl, carries)
        });
        let ((ql, rl), carries) = {
            let ql = wit.as_ref().map(|w| w.0.clone());
            let rl = wit.as_ref().map(|w| w.1.clone());
            let cs = wit.as_ref().map(|w| w.2.clone());
            ((ql, rl), cs)
        };
        let (q_cells, r_cells, rn, c_cells) = layouter.assign_region(
            || "ff mul",
            |mut region| {
                self.config.q_mul.enable(&mut region, 0)?;
                let adv = self.config.advices;
                Self::copy4(&mut region, &adv[0..4], 0, a)?;
                Self::copy4(&mut region, &adv[4..8], 0, b)?;
                let mut qc = Vec::new();
                let mut rc = Vec::new();
                for i in 0..4 {
                    qc.push(region.assign_advice(|| "q", adv[i], 1, || ql.as_ref().map(|q| fe_int(&q[i])))?);
                    rc.push(region.assign_advice(|| "r", adv[4 + i], 1, || rl.as_ref().map(|r| fe_int(&r[i])))?);
                }
                let rn = region.assign_advice(
                    || "r native",
                    adv[8],
                    1,
                    || rl.as_ref().map(|r| (0..4).fold(pallas::Base::ZERO, |acc, i| acc + fe_int(&r[i]) * fe_big(&(BigUint::one() << (64 * i))))),
                )?;
                let mut cc = Vec::new();
                for k in 0..6 {
                    cc.push(region.assign_advice(|| "carry", adv[k], 2, || carries.as_ref().map(|c| fe_int(&c[k])))?);
                }
                Ok((qc, rc, rn, cc))
            },
        )?;
        for (i, q) in q_cells.iter().enumerate() {
            self.rc_bits(layouter.namespace(|| format!("rc q{i}")), q, Q_RC_WORDS * 10)?;
        }
        self.rc_limbs(layouter.namespace(|| "rc r"), &r_cells)?;
        for (k, c) in c_cells.iter().enumerate() {
            self.rc_bits(layouter.namespace(|| format!("rc carry{k}")), c, CARRY_RC_WORDS * 10)?;
        }
        let value = rl.map(|r| r.iter().enumerate().fold(BigInt::zero(), |acc, (i, l)| acc + (l << (64 * i))));
        Ok(CrtInteger::new(OverflowInteger::new(r_cells, LIMB_BITS), rn, value))
    }

    fn add_sub(&self, mut layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>, sub: bool, forge_zero: bool) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let p = BigInt::from(self.p_biguint.clone());
        let p_l = limbs4(&self.p_biguint).map(BigInt::from);
        let forge_out = (!sub && forge::is(forge::ADD_OUT)) || (sub && (forge::is(forge::SUB_OUT) || forge_zero));
        let wit = a.value.clone().zip(b.value.clone()).map(|(a, b)| {
            let (s, q) = if sub {
                let d = &a - &b;
                if d < BigInt::zero() { (d + &p, 1u32) } else { (d, 0) }
            } else {
                let s = &a + &b;
                if s >= p { (s - &p, 1u32) } else { (s, 0) }
            };
            let r = if forge_out { if sub { BigInt::zero() } else { s + 1 } } else { s };
            let al = limbs4(&a.to_biguint().unwrap()).map(BigInt::from);
            let bl = limbs4(&b.to_biguint().unwrap()).map(BigInt::from);
            let rl = limbs4(&r.to_biguint().unwrap()).map(BigInt::from);
            let mut cs = Vec::new();
            let mut cprev = BigInt::zero();
            for k in 0..3 {
                let t = if sub { &al[k] - &bl[k] + BigInt::from(q) * &p_l[k] - &rl[k] } else { &al[k] + &bl[k] - BigInt::from(q) * &p_l[k] - &rl[k] } + &cprev;
                let c = floor_div64(&t);
                let c = c.clamp(BigInt::from(-1), BigInt::from(1));
                cs.push(&c + 1);
                cprev = c;
            }
            (r, q, cs)
        });
        let (r_cells, rn) = layouter.assign_region(
            || if sub { "ff sub" } else { "ff add" },
            |mut region| {
                if sub { self.config.q_sub.enable(&mut region, 0)? } else { self.config.q_add.enable(&mut region, 0)? }
                let adv = self.config.advices;
                Self::copy4(&mut region, &adv[0..4], 0, a)?;
                Self::copy4(&mut region, &adv[4..8], 0, b)?;
                region.assign_advice(|| "q", adv[8], 0, || wit.as_ref().map(|w| pallas::Base::from(w.1 as u64)))?;
                let mut rc = Vec::new();
                for i in 0..4 {
                    rc.push(region.assign_advice(|| "r", adv[i], 1, || wit.as_ref().map(|w| fe_big(&limbs4(&w.0.to_biguint().unwrap())[i])))?);
                }
                for k in 0..3 {
                    region.assign_advice(|| "carry", adv[4 + k], 1, || wit.as_ref().map(|w| fe_int(&w.2[k])))?;
                }
                let rn = region.assign_advice(|| "r native", adv[8], 1, || wit.as_ref().map(|w| fe_int(&w.0)))?;
                Ok((rc, rn))
            },
        )?;
        self.rc_limbs(layouter.namespace(|| "rc r"), &r_cells)?;
        Ok(CrtInteger::new(OverflowInteger::new(r_cells, LIMB_BITS), rn, wit.map(|w| w.0)))
    }

    pub fn add(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.add_sub(layouter, a, b, false, false)
    }

    pub fn sub(&self, layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.add_sub(layouter, a, b, true, false)
    }

    /// Canonical-form check: a < modulus (over Z).
    pub fn assert_canonical(&self, mut layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> {
        let m1 = &self.p_biguint - 1u32;
        let m1_l = limbs4(&m1).map(BigInt::from);
        let wit = Self::val(a).map(|a| {
            let al = limbs4(&a).map(BigInt::from);
            // d = m-1-a computed limb-wise with borrows; honest only if a < m
            let d_int = BigInt::from(m1.clone()) - BigInt::from(a.clone());
            let d = if d_int < BigInt::zero() { BigUint::zero() } else { d_int.to_biguint().unwrap() };
            let dl = limbs4(&d).map(BigInt::from);
            let mut cs = Vec::new();
            let mut cprev = BigInt::zero();
            for k in 0..3 {
                let t = &al[k] + &dl[k] - &m1_l[k] + &cprev;
                let c = if t >= (BigInt::one() << 64usize) { BigInt::one() } else { BigInt::zero() };
                cs.push(c.clone());
                cprev = c;
            }
            (d, cs)
        });
        let d_cells = layouter.assign_region(
            || "ff lt",
            |mut region| {
                self.config.q_lt.enable(&mut region, 0)?;
                let adv = self.config.advices;
                Self::copy4(&mut region, &adv[0..4], 0, a)?;
                let mut dc = Vec::new();
                for i in 0..4 {
                    dc.push(region.assign_advice(|| "d", adv[4 + i], 0, || wit.as_ref().map(|w| fe_big(&limbs4(&w.0)[i])))?);
                }
                for k in 0..3 {
                    region.assign_advice(|| "borrow", adv[k], 1, || wit.as_ref().map(|w| fe_int(&w.1[k])))?;
                }
                Ok(dc)
            },
        )?;
        self.rc_limbs(layouter.namespace(|| "rc d"), &d_cells)
    }

    /// r = cond ? a : b (cond boolean, limbs and native).
    pub fn select(&self, mut layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>, cond: &Cell) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let value = cond.value().copied().zip(a.value.clone()).zip(b.value.clone()).map(|((c, a), b)| if c == pallas::Base::ONE { a } else { b });
        let (limbs, rn) = layouter.assign_region(
            || "ff select",
            |mut region| {
                self.config.q_select.enable(&mut region, 0)?;
                let adv = self.config.advices;
                Self::copy4(&mut region, &adv[0..4], 0, a)?;
                Self::copy4(&mut region, &adv[4..8], 0, b)?;
                cond.copy_advice(|| "cond", &mut region, adv[8], 0)?;
                let mut out = Vec::new();
                for i in 0..4 {
                    let v = cond.value().copied().zip(a.truncation.limbs[i].value().copied()).zip(b.truncation.limbs[i].value().copied()).map(|((c, a), b)| b + c * (a - b));
                    out.push(region.assign_advice(|| "r", adv[i], 1, || v)?);
                }
                a.native.copy_advice(|| "an", &mut region, adv[4], 1)?;
                b.native.copy_advice(|| "bn", &mut region, adv[5], 1)?;
                let v = cond.value().copied().zip(a.native.value().copied()).zip(b.native.value().copied()).map(|((c, a), b)| b + c * (a - b));
                let rn = region.assign_advice(|| "rn", adv[6], 1, || v)?;
                Ok((out, rn))
            },
        )?;
        Ok(CrtInteger::new(OverflowInteger::new(limbs, LIMB_BITS), rn, value))
    }

    /// c = a / b: witness c, check c·b ≡ a. Only for denominators known non-zero (see div_checked).
    pub fn div(&self, mut layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let p = self.p_biguint.clone();
        let c_v = Self::val(a).zip(Self::val(b)).map(|(a, b)| {
            let inv = b.modpow(&(&p - 2u32), &p);
            (a * inv) % &p
        });
        let c = self.assign_biguint(layouter.namespace(|| "div witness"), c_v)?;
        let prod = self.mul(layouter.namespace(|| "c*b"), &c, b)?;
        self.enforce_equal(layouter.namespace(|| "c*b == a"), &prod, a)?;
        Ok(c)
    }

    /// a / b with b proven invertible (b·inv ≡ 1), so b ≡ 0 is unsatisfiable.
    pub fn div_checked(&self, mut layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let p = self.p_biguint.clone();
        let inv_v = Self::val(b).map(|b| b.modpow(&(&p - 2u32), &p));
        let inv = self.assign_biguint(layouter.namespace(|| "inv witness"), inv_v)?;
        let one = self.load_constant_big(layouter.namespace(|| "one"), BigUint::one())?;
        let bi = self.mul(layouter.namespace(|| "b*inv"), b, &inv)?;
        self.enforce_equal(layouter.namespace(|| "b*inv == 1"), &bi, &one)?;
        self.mul(layouter.namespace(|| "a*inv"), a, &inv)
    }

    /// All limbs zero (x = 0 over Z).
    pub fn enforce_zero(&self, mut layouter: impl Layouter<pallas::Base>, num: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> {
        layouter.assign_region(
            || "enforce zero",
            |mut region| {
                for (i, limb) in num.truncation.limbs.iter().enumerate() {
                    let c = limb.copy_advice(|| "limb", &mut region, self.config.advices[i], 0)?;
                    region.constrain_constant(c.cell(), pallas::Base::ZERO)?;
                }
                Ok(())
            },
        )
    }

    /// a ≡ b (mod p): (a - b + q p) = 0 over Z with q ∈ {0,1}.
    pub fn enforce_equal(&self, mut layouter: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> {
        // forge::EQ_SUB: the original attack (witness a-b = 0 only inside enforce_equal)
        let diff = self.add_sub(layouter.namespace(|| "a-b"), a, b, true, forge::is(forge::EQ_SUB))?;
        self.enforce_zero(layouter.namespace(|| "enforce zero"), &diff)
    }
}

impl<Fp: PrimeField> FpInstructions<Fp> for FpChip<Fp> {
    fn load_constant(&self, l: impl Layouter<pallas::Base>, v: Fp) -> Result<ProperCrtUint<pallas::Base>, PlonkError> { FpChip::load_constant(self, l, v) }
    fn load_private(&self, l: impl Layouter<pallas::Base>, v: Value<Fp>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> { FpChip::load_private(self, l, v) }
    fn range_check_limbs(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> { FpChip::range_check_limbs(self, l, a) }
    fn to_native(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>) -> Result<Cell, PlonkError> { FpChip::to_native(self, l, a) }
    fn enforce_zero(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> { FpChip::enforce_zero(self, l, a) }
    fn enforce_equal(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<(), PlonkError> { FpChip::enforce_equal(self, l, a, b) }
    fn add(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> { FpChip::add(self, l, a, b) }
    fn sub(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> { FpChip::sub(self, l, a, b) }
    fn mul(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> { FpChip::mul(self, l, a, b) }
    fn div(&self, l: impl Layouter<pallas::Base>, a: &ProperCrtUint<pallas::Base>, b: &ProperCrtUint<pallas::Base>) -> Result<ProperCrtUint<pallas::Base>, PlonkError> { FpChip::div(self, l, a, b) }
}


// ============================================================================
// BigIntChip (kept for backward compatibility)
// ============================================================================

#[derive(Clone, Debug)]
pub struct BigIntConfig {
    pub advices: [Column<Advice>; 3],
    pub q_enable: Selector,
}

impl BigIntConfig {
    pub fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        advices: [Column<Advice>; 3],
    ) -> Self {
        let q_enable = meta.selector();
        for advice in advices.iter() {
            meta.enable_equality(*advice);
        }
        Self { advices, q_enable }
    }
}

#[derive(Clone, Debug)]
pub struct BigIntChip {
    pub config: BigIntConfig,
    pub limb_bits: usize,
    pub num_limbs: usize,
}

impl BigIntChip {
    pub fn construct(config: BigIntConfig, limb_bits: usize, num_limbs: usize) -> Self {
        Self {
            config,
            limb_bits,
            num_limbs,
        }
    }
    pub fn assign_constant(
        &self,
        region: &mut Region<'_, pallas::Base>,
        offset: usize,
        value: &BigUint,
    ) -> Result<ProperUint<pallas::Base>, PlonkError> {
        let fixed = FixedOverflowInteger::from_native(value, self.num_limbs, self.limb_bits);
        let mut limbs = Vec::with_capacity(self.num_limbs);
        for (i, &limb_value) in fixed.limbs.iter().enumerate() {
            let cell = region.assign_advice(
                || format!("constant limb {}", i),
                self.config.advices[0],
                offset + i,
                || Value::known(limb_value),
            )?;
            limbs.push(cell);
        }
        Ok(ProperUint::new(limbs))
    }
    pub fn assign_witness(
        &self,
        region: &mut Region<'_, pallas::Base>,
        offset: usize,
        value: Value<&BigUint>,
    ) -> Result<ProperUint<pallas::Base>, PlonkError> {
        let mut limbs = Vec::with_capacity(self.num_limbs);
        for i in 0..self.num_limbs {
            let limb_value =
                value.map(|v| decompose_biguint_simple(v, self.num_limbs, self.limb_bits)[i]);
            let cell = region.assign_advice(
                || format!("witness limb {}", i),
                self.config.advices[0],
                offset + i,
                || limb_value,
            )?;
            limbs.push(cell);
        }
        Ok(ProperUint::new(limbs))
    }
}

// ============================================================================
// Secp256k1Config / Secp256k1Chip
// ============================================================================

type SecpPoint<Base> = (ProperCrtUint<Base>, ProperCrtUint<Base>);

/// Configuration for secp256k1 elliptic curve operations.
#[derive(Clone, Debug)]
pub struct Secp256k1Config {
    pub fp_config: FpConfig,
    pub fq_config: FpConfig,
    /// Bit-decomposition step (`current = 2·next + bit`).
    q: Selector,
    /// Integer limb addition with a boolean carry: `a + b + cin = sum + cout·2^64`.
    q_carry: Selector,
    /// `b = bit · c` with `bit` boolean. Used to scale the scalar-field modulus by the GLV carry.
    q_scale: Selector,
    /// `out = bit ? a : b` on a single limb.
    q_blend: Selector,
    /// `packed = lo + hi · 2^64` for a public 128-bit half of the challenge.
    q_pack: Selector,
    /// Optional fixed-base window table for `sk·G` (lab, task 1a).
    pub fixed_base: Option<fixed_base::FixedBaseConfig>,
}

impl Secp256k1Config {
    /// Configure the secp256k1 chip.
    ///
    /// Uses 9 shared advice columns for both Fp and Fq operations.
    pub fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        advices: [Column<Advice>; 9],
        range_check: LookupRangeCheckConfig<pallas::Base, 10>,
    ) -> Self {
        // Compute c coefficients and q_native for both fields
        let fp_c = secp_fp_c_coeffs();
        let fq_c = secp_fq_c_coeffs();

        let p_mod = BigUint::parse_bytes(SECP_P_HEX.as_bytes(), 16).unwrap();
        let n_mod = BigUint::parse_bytes(SECP_N_HEX.as_bytes(), 16).unwrap();
        let native_mod = modulus_simple_pallas();
        let fp_q_native = biguint_to_fe_simple(&(&p_mod % &native_mod));
        let fq_q_native = biguint_to_fe_simple(&(&n_mod % &native_mod));

        let fp_config = FpConfig::configure(meta, advices, range_check.clone(), fp_c, fp_q_native, p_mod.clone());
        let fq_config = FpConfig::configure(meta, advices, range_check, fq_c, fq_q_native, n_mod.clone());

        let q = meta.selector();
        let q_carry = meta.selector();
        let q_scale = meta.selector();
        let q_blend = meta.selector();
        let q_pack = meta.selector();
        let two64 = biguint_to_fe_simple(&(BigUint::one() << 64));

        // Scalar decomposition gate (bit decomposition for MSM)
        meta.create_gate("scalar decomposition step", |meta| {
            let q = meta.query_selector(q);
            let current = meta.query_advice(advices[0], Rotation::cur());
            let bit = meta.query_advice(advices[1], Rotation::cur());
            let current_prime = meta.query_advice(advices[0], Rotation::next());

            vec![
                q.clone()
                    * (current
                        - (Expression::Constant(pallas::Base::one().double()) * current_prime
                            + bit.clone())),
                q * bit.clone() * (bit - Expression::Constant(pallas::Base::one())),
            ]
        });

        // Integer carry chain on 64-bit limbs. `cout` is boolean, so each limb input must be < 2^64
        // (range-checked by the caller) or the relation is unsatisfiable for a real sum.
        meta.create_gate("glv carry add", |meta| {
            let q = meta.query_selector(q_carry);
            let a = meta.query_advice(advices[0], Rotation::cur());
            let b = meta.query_advice(advices[1], Rotation::cur());
            let sum = meta.query_advice(advices[2], Rotation::cur());
            let cin = meta.query_advice(advices[3], Rotation::cur());
            let cout = meta.query_advice(advices[4], Rotation::cur());
            let two64 = Expression::Constant(two64);
            vec![
                q.clone() * (a + b + cin - sum - cout.clone() * two64),
                q * cout.clone() * (cout - Expression::Constant(pallas::Base::ONE)),
            ]
        });

        meta.create_gate("glv scale by bit", |meta| {
            let q = meta.query_selector(q_scale);
            let bit = meta.query_advice(advices[0], Rotation::cur());
            let constant = meta.query_advice(advices[1], Rotation::cur());
            let scaled = meta.query_advice(advices[2], Rotation::cur());
            vec![
                q.clone() * (scaled - bit.clone() * constant),
                q * bit.clone() * (bit - Expression::Constant(pallas::Base::ONE)),
            ]
        });

        // `out = bit ? a : b`.
        meta.create_gate("glv blend", |meta| {
            let q = meta.query_selector(q_blend);
            let bit = meta.query_advice(advices[0], Rotation::cur());
            let a = meta.query_advice(advices[1], Rotation::cur());
            let b = meta.query_advice(advices[2], Rotation::cur());
            let out = meta.query_advice(advices[3], Rotation::cur());
            vec![
                q.clone() * (out - b.clone() - bit.clone() * a + bit.clone() * b),
                q * bit.clone() * (bit - Expression::Constant(pallas::Base::ONE)),
            ]
        });

        // 128-bit public half of a secp scalar: packed = lo + hi·2^64.
        meta.create_gate("pack u128", |meta| {
            let q = meta.query_selector(q_pack);
            let lo = meta.query_advice(advices[0], Rotation::cur());
            let hi = meta.query_advice(advices[1], Rotation::cur());
            let packed = meta.query_advice(advices[2], Rotation::cur());
            vec![q * (packed - (lo + hi * Expression::Constant(two64)))]
        });

        Self {
            fp_config,
            fq_config,
            q,
            q_carry,
            q_scale,
            q_blend,
            q_pack,
            fixed_base: None,
        }
    }
}

/// Chip for secp256k1 elliptic curve operations.
#[derive(Clone, Debug)]
pub struct Secp256k1Chip {
    pub fp: Secp256k1FpChip,
    pub fq: Secp256k1FqChip,
    q: Selector,
    q_carry: Selector,
    q_scale: Selector,
    q_blend: Selector,
    q_pack: Selector,
    fixed_base: Option<fixed_base::FixedBaseConfig>,
}

impl Secp256k1Chip {
    /// Construct a new Secp256k1Chip. Uses 4×64-bit limbs.
    pub fn construct(config: Secp256k1Config) -> Self {
        Self {
            fp: Secp256k1FpChip::construct(
                config.fp_config,
                LIMB_BITS,
                NUM_LIMBS,
                secp_fp_c_coeffs(),
                &secp_p(),
            ),
            fq: Secp256k1FqChip::construct(
                config.fq_config,
                LIMB_BITS,
                NUM_LIMBS,
                secp_fq_c_coeffs(),
                &secp_n(),
            ),
            q: config.q,
            q_carry: config.q_carry,
            q_scale: config.q_scale,
            q_blend: config.q_blend,
            q_pack: config.q_pack,
            fixed_base: config.fixed_base,
        }
    }

    /// `packed = lo + hi·2^64`. Both limbs are already range-checked to 64 bits.
    pub fn pack_u128(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        lo: &AssignedCell<pallas::Base, pallas::Base>,
        hi: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError> {
        let two64 = pallas::Base::from_u128(1u128 << 64);
        let packed_val = lo
            .value()
            .zip(hi.value())
            .map(|(lo, hi)| *lo + *hi * two64);
        let advice = self.fq.config.advices;
        layouter.assign_region(
            || "pack u128",
            |mut region| {
                self.q_pack.enable(&mut region, 0)?;
                lo.copy_advice(|| "lo limb", &mut region, advice[0], 0)?;
                hi.copy_advice(|| "hi limb", &mut region, advice[1], 0)?;
                region.assign_advice(|| "packed", advice[2], 0, || packed_val)
            },
        )
    }

    // ── Scalar bit decomposition ──

    pub(crate) fn decompose_limb_to_bits(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        limb: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<Vec<AssignedCell<pallas::Base, pallas::Base>>, PlonkError> {
        let inv2 = pallas::Base::invert(&pallas::Base::from(2)).unwrap();

        layouter.assign_region(
            || "decompose limb to bits",
            |mut region: Region<'_, pallas::Base>| {
                let mut bits = Vec::with_capacity(LIMB_BITS);
                let mut current_offset = 0;
                limb.copy_advice(
                    || "copy limb",
                    &mut region,
                    self.fq.config.advices[0],
                    current_offset,
                )?;

                let mut current_value = limb.value().cloned();
                let mut last_assigned: Option<AssignedCell<pallas::Base, pallas::Base>> = None;

                for _ in 0..LIMB_BITS {
                    self.q.enable(&mut region, current_offset)?;

                    let bit_val = current_value.map(|v| {
                        let repr = v.to_repr();
                        pallas::Base::from((repr[0] & 1) as u64)
                    });
                    let bit_cell = region.assign_advice(
                        || "bit",
                        self.fq.config.advices[1],
                        current_offset,
                        || bit_val,
                    )?;
                    bits.push(bit_cell);

                    let next_val = current_value
                        .zip(bit_val)
                        .map(|(current, bit)| (current - bit) * inv2);
                    let next_cell = region.assign_advice(
                        || "next current",
                        self.fq.config.advices[0],
                        current_offset + 1,
                        || next_val,
                    )?;
                    last_assigned = Some(next_cell);

                    current_value = next_val;
                    current_offset += 1;
                }

                if let Some(final_assigned) = last_assigned {
                    region.constrain_constant(final_assigned.cell(), pallas::Base::zero())?;
                }
                Ok(bits)
            },
        )
    }

    pub(crate) fn decompose_scalar_to_bits(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        sk: &ProperCrtUint<pallas::Base>,
    ) -> Result<Vec<AssignedCell<pallas::Base, pallas::Base>>, PlonkError> {
        let mut bits = Vec::new();
        for limb in sk.truncation.limbs.iter() {
            let limb_bits =
                self.decompose_limb_to_bits(layouter.namespace(|| "decompose limb"), limb)?;
            bits.extend(limb_bits);
        }
        bits.truncate(256);
        Ok(bits)
    }

    // ── Point operations ──

    pub(crate) fn add_point(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        p: &SecpPoint<pallas::Base>,
        q: &SecpPoint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let fp = &self.fp;

        let dy = fp.sub(layouter.namespace(|| "dy"), &q.1, &p.1)?;
        let dx = fp.sub(layouter.namespace(|| "dx"), &q.0, &p.0)?;
        // dx must be invertible: P = ±Q would otherwise leave lambda free.
        let lambda = fp.div_checked(layouter.namespace(|| "lambda"), &dy, &dx)?;
        let lambda_sq = fp.mul(layouter.namespace(|| "lambda_sq"), &lambda, &lambda)?;
        let x3 = fp.sub(layouter.namespace(|| "x3 part1"), &lambda_sq, &p.0)?;
        let x3 = fp.sub(layouter.namespace(|| "x3"), &x3, &q.0)?;
        let dx_new = fp.sub(layouter.namespace(|| "dx_new"), &p.0, &x3)?;
        let temp = fp.mul(layouter.namespace(|| "temp"), &lambda, &dx_new)?;
        let y3 = fp.sub(layouter.namespace(|| "y3"), &temp, &p.1)?;
        Ok((x3, y3))
    }

    pub(crate) fn double_point(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        p: &SecpPoint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let fp = &self.fp;

        // 2 and 3 enter as additions (no witnessed constants to pin).
        let x_sq = fp.mul(layouter.namespace(|| "x_sq"), &p.0, &p.0)?;
        let two_x_sq = fp.add(layouter.namespace(|| "2x^2"), &x_sq, &x_sq)?;
        let three_x_sq = fp.add(layouter.namespace(|| "3x^2"), &two_x_sq, &x_sq)?;
        let two_y = fp.add(layouter.namespace(|| "two_y"), &p.1, &p.1)?;
        // 2y != 0 for every curve point (secp256k1 has no 2-torsion), so plain div is sound:
        // b ≡ 0 would force 3x^2 ≡ 0, i.e. (0,0), which is not on y^2 = x^3 + 7.
        let lambda = fp.div(layouter.namespace(|| "lambda"), &three_x_sq, &two_y)?;
        let lambda_sq = fp.mul(layouter.namespace(|| "lambda_sq"), &lambda, &lambda)?;
        let two_x = fp.add(layouter.namespace(|| "two_x"), &p.0, &p.0)?;
        let x3 = fp.sub(layouter.namespace(|| "x3"), &lambda_sq, &two_x)?;
        let dx = fp.sub(layouter.namespace(|| "dx"), &p.0, &x3)?;
        let temp = fp.mul(layouter.namespace(|| "temp"), &lambda, &dx)?;
        let y3 = fp.sub(layouter.namespace(|| "y3"), &temp, &p.1)?;

        Ok((x3, y3))
    }

    pub(crate) fn select(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
        cond: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.fp.select(layouter, a, b, cond)
    }

    // ── GLV scalar multiplication ──

    fn advice_col(&self) -> Column<Advice> {
        self.fp.config.advices[0]
    }

    fn pin_limbs(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        limbs: &[AssignedCell<pallas::Base, pallas::Base>],
        expected: &[pallas::Base],
    ) -> Result<(), PlonkError> {
        layouter.assign_region(
            || "pin limbs",
            |mut region| {
                for (i, (limb, exp)) in limbs.iter().zip(expected.iter()).enumerate() {
                    let cell =
                        limb.copy_advice(|| format!("pin {i}"), &mut region, self.advice_col(), i)?;
                    region.constrain_constant(cell.cell(), *exp)?;
                }
                Ok(())
            },
        )
    }

    fn load_pinned_fp(
        &self,
        layouter: impl Layouter<pallas::Base>,
        value: Secp256k1Fp,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.fp.load_constant(layouter, value)
    }

    fn load_pinned_fq(
        &self,
        layouter: impl Layouter<pallas::Base>,
        value: Secp256k1Fq,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.fq.load_constant(layouter, value)
    }

    /// Boolean witness. The decomposition gate forces `bit ∈ {0,1}`.
    fn assign_bit(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        bit: Value<pallas::Base>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError> {
        layouter.assign_region(
            || "assign bit",
            |mut region| {
                self.q.enable(&mut region, 0)?;
                let current =
                    region.assign_advice(|| "bit current", self.fq.config.advices[0], 0, || bit)?;
                let bit_cell =
                    region.assign_advice(|| "bit", self.fq.config.advices[1], 0, || bit)?;
                let next = region.assign_advice(
                    || "bit next",
                    self.fq.config.advices[0],
                    1,
                    || Value::known(pallas::Base::ZERO),
                )?;
                region.constrain_constant(next.cell(), pallas::Base::ZERO)?;
                // `current` equals `bit` because next is zero. Return the boolean cell.
                let _ = current;
                Ok(bit_cell)
            },
        )
    }

    fn constrain_cells_equal(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &AssignedCell<pallas::Base, pallas::Base>,
        b: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), PlonkError> {
        layouter.assign_region(
            || "equal cells",
            |mut region| {
                let a = a.copy_advice(|| "a", &mut region, self.advice_col(), 0)?;
                let b = b.copy_advice(|| "b", &mut region, self.fq.config.advices[1], 0)?;
                region.constrain_equal(a.cell(), b.cell())
            },
        )
    }

    /// `sum_limbs[i] + 2^64·carry = a[i] + b[i] + carry_in`, final carry returned.
    fn carry_add_limbs(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &[AssignedCell<pallas::Base, pallas::Base>],
        b: &[AssignedCell<pallas::Base, pallas::Base>],
    ) -> Result<
        (
            [AssignedCell<pallas::Base, pallas::Base>; 4],
            AssignedCell<pallas::Base, pallas::Base>,
        ),
        PlonkError,
    > {
        assert_eq!(a.len(), 4);
        assert_eq!(b.len(), 4);
        let mut cin = Value::known(pallas::Base::ZERO);
        let mut planned: Vec<(Value<pallas::Base>, Value<pallas::Base>)> = Vec::with_capacity(4);
        for i in 0..4 {
            let av = a[i].value().copied();
            let bv = b[i].value().copied();
            let total = av
                .zip(bv)
                .zip(cin)
                .map(|((a, b), c)| base_as_u128(a) + base_as_u128(b) + base_as_u128(c));
            let sum_v = total.map(|t| pallas::Base::from((t & ((1u128 << 64) - 1)) as u64));
            let cout_v = total.map(|t| pallas::Base::from((t >> 64) as u64));
            cin = cout_v;
            planned.push((sum_v, cout_v));
        }

        layouter.assign_region(
            || "carry add",
            |mut region| {
                let mut sums: [Option<AssignedCell<pallas::Base, pallas::Base>>; 4] =
                    [None, None, None, None];
                let mut cout_cell: Option<AssignedCell<pallas::Base, pallas::Base>> = None;
                for i in 0..4 {
                    self.q_carry.enable(&mut region, i)?;
                    a[i].copy_advice(
                        || format!("a {i}"),
                        &mut region,
                        self.fp.config.advices[0],
                        i,
                    )?;
                    b[i].copy_advice(
                        || format!("b {i}"),
                        &mut region,
                        self.fp.config.advices[1],
                        i,
                    )?;
                    if i == 0 {
                        let cin0 = region.assign_advice(
                            || "cin 0",
                            self.fp.config.advices[3],
                            0,
                            || Value::known(pallas::Base::ZERO),
                        )?;
                        region.constrain_constant(cin0.cell(), pallas::Base::ZERO)?;
                    } else {
                        cout_cell.as_ref().unwrap().copy_advice(
                            || format!("cin {i}"),
                            &mut region,
                            self.fp.config.advices[3],
                            i,
                        )?;
                    }
                    let (sum_v, cout_v) = planned[i];
                    sums[i] = Some(region.assign_advice(
                        || format!("sum {i}"),
                        self.fp.config.advices[2],
                        i,
                        || sum_v,
                    )?);
                    cout_cell = Some(region.assign_advice(
                        || format!("cout {i}"),
                        self.fp.config.advices[4],
                        i,
                        || cout_v,
                    )?);
                }
                Ok((
                    [
                        sums[0].take().unwrap(),
                        sums[1].take().unwrap(),
                        sums[2].take().unwrap(),
                        sums[3].take().unwrap(),
                    ],
                    cout_cell.unwrap(),
                ))
            },
        )
    }

    fn scale_modulus_by_bit(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        bit: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<[AssignedCell<pallas::Base, pallas::Base>; 4], PlonkError> {
        let n_limbs = {
            let decomposed = decompose_biguint_simple(&secp_n(), 4, 64);
            [decomposed[0], decomposed[1], decomposed[2], decomposed[3]]
        };
        layouter.assign_region(
            || "m * n",
            |mut region| {
                let mut out: [Option<AssignedCell<pallas::Base, pallas::Base>>; 4] =
                    [None, None, None, None];
                for i in 0..4 {
                    self.q_scale.enable(&mut region, i)?;
                    bit.copy_advice(
                        || format!("m {i}"),
                        &mut region,
                        self.fp.config.advices[0],
                        i,
                    )?;
                    let constant = region.assign_advice(
                        || format!("n limb {i}"),
                        self.fp.config.advices[1],
                        i,
                        || Value::known(n_limbs[i]),
                    )?;
                    region.constrain_constant(constant.cell(), n_limbs[i])?;
                    let scaled_v = bit.value().map(|m| *m * n_limbs[i]);
                    out[i] = Some(region.assign_advice(
                        || format!("scaled {i}"),
                        self.fp.config.advices[2],
                        i,
                        || scaled_v,
                    )?);
                }
                Ok([
                    out[0].take().unwrap(),
                    out[1].take().unwrap(),
                    out[2].take().unwrap(),
                    out[3].take().unwrap(),
                ])
            },
        )
    }

    fn enforce_carry_equal(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        left: &[AssignedCell<pallas::Base, pallas::Base>],
        left_cout: &AssignedCell<pallas::Base, pallas::Base>,
        right: &[AssignedCell<pallas::Base, pallas::Base>],
        right_cout: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), PlonkError> {
        layouter.assign_region(
            || "carry sums equal",
            |mut region| {
                for i in 0..4 {
                    let a = left[i].copy_advice(
                        || format!("L {i}"),
                        &mut region,
                        self.fp.config.advices[0],
                        i,
                    )?;
                    let b = right[i].copy_advice(
                        || format!("R {i}"),
                        &mut region,
                        self.fp.config.advices[1],
                        i,
                    )?;
                    region.constrain_equal(a.cell(), b.cell())?;
                }
                let a = left_cout.copy_advice(
                    || "L cout",
                    &mut region,
                    self.fp.config.advices[0],
                    4,
                )?;
                let b = right_cout.copy_advice(
                    || "R cout",
                    &mut region,
                    self.fp.config.advices[1],
                    4,
                )?;
                region.constrain_equal(a.cell(), b.cell())
            },
        )
    }

    /// `k1 + λ·k2 ≡ k (mod n)` with both halves shorter than 128 bits.
    fn constrain_glv_split(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        k: &ProperCrtUint<pallas::Base>,
        split: &Value<GlvSplit>,
    ) -> Result<
        (
            ProperCrtUint<pallas::Base>,
            AssignedCell<pallas::Base, pallas::Base>,
            AssignedCell<pallas::Base, pallas::Base>,
            bool,
            ProperCrtUint<pallas::Base>,
            AssignedCell<pallas::Base, pallas::Base>,
            AssignedCell<pallas::Base, pallas::Base>,
            bool,
        ),
        PlonkError,
    > {
        let mag1_v = split
            .as_ref()
            .map(|s| be32_to_fq(&biguint_to_be32(&s.k1.mag)));
        let mag2_v = split
            .as_ref()
            .map(|s| be32_to_fq(&biguint_to_be32(&s.k2.mag)));
        let mag1 = self
            .fq
            .load_private(layouter.namespace(|| "k1 mag"), mag1_v)?;
        let mag2 = self
            .fq
            .load_private(layouter.namespace(|| "k2 mag"), mag2_v)?;
        self.fq
            .range_check_limbs(layouter.namespace(|| "rc k1"), &mag1)?;
        self.fq
            .range_check_limbs(layouter.namespace(|| "rc k2"), &mag2)?;
        self.pin_limbs(
            layouter.namespace(|| "k1 high zero"),
            &mag1.truncation.limbs[2..4],
            &[pallas::Base::ZERO, pallas::Base::ZERO],
        )?;
        self.pin_limbs(
            layouter.namespace(|| "k2 high zero"),
            &mag2.truncation.limbs[2..4],
            &[pallas::Base::ZERO, pallas::Base::ZERO],
        )?;

        let sign1_v = split.as_ref().map(|s| pallas::Base::from(s.k1.neg as u64));
        let sign2_v = split.as_ref().map(|s| pallas::Base::from(s.k2.neg as u64));
        let sign1 = self.assign_bit(layouter.namespace(|| "k1 sign"), sign1_v)?;
        let sign2 = self.assign_bit(layouter.namespace(|| "k2 sign"), sign2_v)?;

        let (k1_zero, k2_zero) = {
            let mut z1 = false;
            let mut z2 = false;
            split.as_ref().map(|s| {
                z1 = s.k1.mag.is_zero();
                z2 = s.k2.mag.is_zero();
            });
            (z1, z2)
        };
        let z1 = self.assign_bit(
            layouter.namespace(|| "k1 zero"),
            Value::known(pallas::Base::from(k1_zero as u64)),
        )?;
        let z2 = self.assign_bit(
            layouter.namespace(|| "k2 zero"),
            Value::known(pallas::Base::from(k2_zero as u64)),
        )?;
        // `mag * zero_flag = 0` forces the flag to be clear whenever the magnitude is not.
        for (mag, flag, label) in [(&mag1, &z1, "z1"), (&mag2, &z2, "z2")] {
            let flag_fq = self.load_fq_from_bit(layouter.namespace(|| label), flag)?;
            let prod = self.fq.mul(
                layouter.namespace(|| format!("{label} prod")),
                mag,
                &flag_fq,
            )?;
            self.fq
                .enforce_zero(layouter.namespace(|| format!("{label} zero")), &prod)?;
        }

        let r1 = self.signed_residue(layouter.namespace(|| "r1"), &mag1, &sign1, split, true)?;
        let r2 = self.signed_residue(layouter.namespace(|| "r2"), &mag2, &sign2, split, false)?;

        let lambda = self.load_pinned_fq(layouter.namespace(|| "lambda"), glv_lambda_fq())?;
        let prod = self
            .fq
            .mul(layouter.namespace(|| "lambda * r2"), &lambda, &r2)?;
        self.fq
            .range_check_limbs(layouter.namespace(|| "rc lambda*r2"), &prod)?;

        let (left, left_cout) = self.carry_add_limbs(
            layouter.namespace(|| "r1 + lambda*r2"),
            &r1.truncation.limbs,
            &prod.truncation.limbs,
        )?;
        let m_bit = {
            let m_v = split.as_ref().zip(k.value.as_ref()).map(|(s, k_int)| {
                let n = secp_n();
                let r1 = &s.k1.residue;
                let r2 = &s.k2.residue;
                let lam = parse_hex_uint(GLV_LAMBDA_HEX);
                let reduced = (r2 * &lam) % &n;
                let sum = r1 + &reduced;
                let k_big = k_int.to_biguint().expect("secret scalar is non-negative");
                let m = if sum >= n { 1u64 } else { 0 };
                if !forge::is(forge::GLV_K1) {
                    assert_eq!(&sum - &(&n * m), k_big % &n, "GLV congruence witness");
                }
                pallas::Base::from(m)
            });
            self.assign_bit(layouter.namespace(|| "glv m"), m_v)?
        };
        let m_n = self.scale_modulus_by_bit(layouter.namespace(|| "m*n"), &m_bit)?;
        let (right, right_cout) =
            self.carry_add_limbs(layouter.namespace(|| "k + m*n"), &k.truncation.limbs, &m_n)?;
        self.enforce_carry_equal(
            layouter.namespace(|| "glv congruence"),
            &left,
            &left_cout,
            &right,
            &right_cout,
        )?;

        Ok((mag1, sign1, z1, k1_zero, mag2, sign2, z2, k2_zero))
    }

    fn load_fq_from_bit(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        bit: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let fq_v = bit.value().map(|b| {
            if *b == pallas::Base::ONE {
                Secp256k1Fq::from(1u64)
            } else {
                Secp256k1Fq::from(0u64)
            }
        });
        let assigned = self
            .fq
            .load_private(layouter.namespace(|| "bit as fq"), fq_v)?;
        self.pin_limbs(
            layouter.namespace(|| "bit fq high"),
            &assigned.truncation.limbs[1..4],
            &[pallas::Base::ZERO, pallas::Base::ZERO, pallas::Base::ZERO],
        )?;
        self.constrain_cells_equal(
            layouter.namespace(|| "bit fq limb0"),
            &assigned.truncation.limbs[0],
            bit,
        )?;
        Ok(assigned)
    }

    fn load_biguint(
        &self,
        layouter: impl Layouter<pallas::Base>,
        value: Value<BigUint>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        self.fq.assign_biguint(layouter, value)
    }

    fn blend_limb(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &AssignedCell<pallas::Base, pallas::Base>,
        b: &AssignedCell<pallas::Base, pallas::Base>,
        bit: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError> {
        layouter.assign_region(
            || "blend limb",
            |mut region| {
                self.q_blend.enable(&mut region, 0)?;
                bit.copy_advice(|| "bit", &mut region, self.fp.config.advices[0], 0)?;
                a.copy_advice(|| "a", &mut region, self.fp.config.advices[1], 0)?;
                b.copy_advice(|| "b", &mut region, self.fp.config.advices[2], 0)?;
                let out_v = bit
                    .value()
                    .copied()
                    .zip(a.value().copied())
                    .zip(b.value().copied())
                    .map(|((bit, a), b)| if bit == pallas::Base::ONE { a } else { b });
                region.assign_advice(|| "out", self.fp.config.advices[3], 0, || out_v)
            },
        )
    }

    /// `bit = 1` selects `a`, otherwise `b`. One blend gate per limb.
    fn blend_crt(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
        bit: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        // modulus-independent gate; binds limbs and native
        self.fp.select(layouter, a, b, bit)
    }

    fn signed_residue(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        mag: &ProperCrtUint<pallas::Base>,
        sign: &AssignedCell<pallas::Base, pallas::Base>,
        split: &Value<GlvSplit>,
        first: bool,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let neg_v = split.as_ref().map(|s| {
            let half = if first { &s.k1 } else { &s.k2 };
            &secp_n() - &half.mag
        });
        let neg = self.load_biguint(layouter.namespace(|| "n - mag"), neg_v)?;
        self.fq
            .range_check_limbs(layouter.namespace(|| "rc n-mag"), &neg)?;
        let (sum, cout) = self.carry_add_limbs(
            layouter.namespace(|| "mag + (n - mag)"),
            &mag.truncation.limbs,
            &neg.truncation.limbs,
        )?;
        let n_limbs = {
            let d = decompose_biguint_simple(&secp_n(), 4, 64);
            [d[0], d[1], d[2], d[3]]
        };
        // `mag + (n − mag) = n` exactly, final carry 0. Holds for mag = 0 as well.
        self.pin_limbs(layouter.namespace(|| "sum is n"), &sum, &n_limbs)?;
        layouter.assign_region(
            || "final carry zero",
            |mut region| {
                let c = cout.copy_advice(|| "cout", &mut region, self.advice_col(), 0)?;
                region.constrain_constant(c.cell(), pallas::Base::ZERO)
            },
        )?;
        // sign = 1 selects `n - mag`.
        self.blend_crt(layouter.namespace(|| "select residue"), &neg, mag, sign)
    }

    fn select_point(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &SecpPoint<pallas::Base>,
        b: &SecpPoint<pallas::Base>,
        cond: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let x = self.blend_crt(layouter.namespace(|| "x"), &a.0, &b.0, cond)?;
        let y = self.blend_crt(layouter.namespace(|| "y"), &a.1, &b.1, cond)?;
        Ok((x, y))
    }

    fn point_from_fp(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        point: (Secp256k1Fp, Secp256k1Fp),
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        Ok((
            self.load_pinned_fp(layouter.namespace(|| "x"), point.0)?,
            self.load_pinned_fp(layouter.namespace(|| "y"), point.1)?,
        ))
    }

    /// `(2^128 + mag)·base − 2^128·base`, with a fixed 128-step chain.
    ///
    /// A zero magnitude would make the correction the inverse of the accumulator, which the
    /// incomplete addition formula cannot compute. That case adds a harmless dummy and the
    /// caller drops the result.
    fn scalar_mul_128(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        base: &SecpPoint<pallas::Base>,
        mag: &ProperCrtUint<pallas::Base>,
        correction: &SecpPoint<pallas::Base>,
        magnitude_is_zero: bool,
        dummy_a: &SecpPoint<pallas::Base>,
        dummy_b: &SecpPoint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let mut bits = Vec::new();
        for limb in mag.truncation.limbs.iter().take(2) {
            let limb_bits =
                self.decompose_limb_to_bits(layouter.namespace(|| "glv limb bits"), limb)?;
            bits.extend(limb_bits);
        }
        bits.reverse();

        let mut acc = base.clone();
        for bit in &bits {
            let doubled = self.double_point(layouter.namespace(|| "glv double"), &acc)?;
            let added = self.add_point(layouter.namespace(|| "glv add"), &doubled, base)?;
            acc = self.select_point(layouter.namespace(|| "glv select"), &added, &doubled, bit)?;
        }

        if magnitude_is_zero {
            let _ = self.add_point(
                layouter.namespace(|| "glv dummy correction"),
                dummy_a,
                dummy_b,
            )?;
            Ok(dummy_a.clone())
        } else {
            self.add_point(layouter.namespace(|| "glv correction"), &acc, correction)
        }
    }

    fn scalar_mul_glv(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        sk: &ProperCrtUint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let split_v = sk.value.as_ref().map(|k| {
            let k_big = k.to_biguint().expect("secret scalar is non-negative");
            let mut sp = glv_split(&k_big);
            if forge::is(forge::GLV_K1) {
                sp.k1.mag += 1u32;
            }
            sp
        });
        let (mag1, sign1, z1_cell, z1, mag2, sign2, z2_cell, z2) =
            self.constrain_glv_split(layouter.namespace(|| "glv split"), sk, &split_v)?;

        let g = generator_xy();
        let g_neg = host_neg_point(g);
        let psi = host_psi(g);
        let psi_neg = host_neg_point(psi);
        let shift = BigUint::one() << 128usize;
        let c_g = host_neg_point(host_mul(g, &shift).expect("2^128 G"));
        let c_psi = host_neg_point(host_mul(psi, &shift).expect("2^128 psi(G)"));
        let g_pt = self.point_from_fp(layouter.namespace(|| "G"), g)?;
        let g_neg_pt = self.point_from_fp(layouter.namespace(|| "-G"), g_neg)?;
        let psi_pt = self.point_from_fp(layouter.namespace(|| "psi G"), psi)?;
        let psi_neg_pt = self.point_from_fp(layouter.namespace(|| "-psi G"), psi_neg)?;
        let c_g_pt = self.point_from_fp(layouter.namespace(|| "-2^128 G"), c_g)?;
        let c_g_neg_pt =
            self.point_from_fp(layouter.namespace(|| "2^128 G"), host_neg_point(c_g))?;
        let c_psi_pt = self.point_from_fp(layouter.namespace(|| "-2^128 psi"), c_psi)?;
        let c_psi_neg_pt =
            self.point_from_fp(layouter.namespace(|| "2^128 psi"), host_neg_point(c_psi))?;
        let dbl_g = self.point_from_fp(layouter.namespace(|| "2G"), host_double(g))?;

        let base1 =
            self.select_point(layouter.namespace(|| "signed G"), &g_neg_pt, &g_pt, &sign1)?;
        let corr1 = self.select_point(
            layouter.namespace(|| "signed correction G"),
            &c_g_neg_pt,
            &c_g_pt,
            &sign1,
        )?;
        let base2 = self.select_point(
            layouter.namespace(|| "signed psi"),
            &psi_neg_pt,
            &psi_pt,
            &sign2,
        )?;
        let corr2 = self.select_point(
            layouter.namespace(|| "signed correction psi"),
            &c_psi_neg_pt,
            &c_psi_pt,
            &sign2,
        )?;

        let p1 = self.scalar_mul_128(
            layouter.namespace(|| "k1 · G"),
            &base1,
            &mag1,
            &corr1,
            z1,
            &g_pt,
            &dbl_g,
        )?;
        let p2 = self.scalar_mul_128(
            layouter.namespace(|| "k2 · psi(G)"),
            &base2,
            &mag2,
            &corr2,
            z2,
            &g_pt,
            &dbl_g,
        )?;

        // Fixed shape: always add once. A zero half makes that add a dummy `G+2G`.
        let sum = if z1 || z2 {
            self.add_point(layouter.namespace(|| "glv combine dummy"), &g_pt, &dbl_g)?
        } else {
            self.add_point(layouter.namespace(|| "k1 G + k2 psi G"), &p1, &p2)?
        };
        // result = z1 ? p2 : (z2 ? p1 : sum). A zero secret (both halves zero) is not a key.
        let inner = self.select_point(
            layouter.namespace(|| "use k1 if k2 zero"),
            &p1,
            &sum,
            &z2_cell,
        )?;
        self.select_point(
            layouter.namespace(|| "use k2 if k1 zero"),
            &p2,
            &inner,
            &z1_cell,
        )
    }

    // ── Montgomery ladder scalar multiplication ──
    #[allow(dead_code)]
    pub(crate) fn scalar_mul_montgomery(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        sk: &ProperCrtUint<pallas::Base>,
        g: &SecpPoint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let mut bits =
            self.decompose_scalar_to_bits(layouter.namespace(|| "decompose scalar"), sk)?;

        bits.reverse(); // MSB first

        let mut r0 = g.clone();
        let mut r1 = self.double_point(layouter.namespace(|| "initial double"), &r0)?;

        for bit in bits {
            let added = self.add_point(layouter.namespace(|| "montgomery add"), &r0, &r1)?;
            let doubled0 = self.double_point(layouter.namespace(|| "montgomery double0"), &r0)?;
            let doubled1 = self.double_point(layouter.namespace(|| "montgomery double1"), &r1)?;

            r0.0 = self.select(
                layouter.namespace(|| "select r0.x"),
                &added.0,
                &doubled0.0,
                &bit,
            )?;
            r0.1 = self.select(
                layouter.namespace(|| "select r0.y"),
                &added.1,
                &doubled0.1,
                &bit,
            )?;
            r1.0 = self.select(
                layouter.namespace(|| "select r1.x"),
                &doubled1.0,
                &added.0,
                &bit,
            )?;
            r1.1 = self.select(
                layouter.namespace(|| "select r1.y"),
                &doubled1.1,
                &added.1,
                &bit,
            )?;
        }

        // Montgomery ladder computes (2^256 + k)*G, subtract 2^256*G.
        // See MONTGOMERY_OFFSET_X/Y constants for derivation and verification.
        let neg_offset_x = secp_fp_from_le(MONTGOMERY_OFFSET_X);
        let neg_offset_y = secp_fp_from_le(MONTGOMERY_OFFSET_Y);
        let neg_offset = (
            self.fp.load_constant(layouter.namespace(|| "neg_offset_x"), neg_offset_x)?,
            self.fp.load_constant(layouter.namespace(|| "neg_offset_y"), neg_offset_y)?,
        );
        let corrected = self.add_point(
            layouter.namespace(|| "montgomery correction"),
            &r0,
            &neg_offset,
        )?;

        Ok(corrected)
    }

    /// Prove key pairing: `public_key = esk * G`
    pub fn prove_key_pairing(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        esk: Value<Secp256k1Fq>,
        epkx: Value<Secp256k1Fp>,
        epky: Value<Secp256k1Fp>,
    ) -> Result<
        (
            ProperCrtUint<pallas::Base>,
            (ProperCrtUint<pallas::Base>, ProperCrtUint<pallas::Base>),
        ),
        PlonkError,
    > {
        // Load secret key
        let sk_assigned = {
            let n = secp_n();
            let v = esk.map(|v| {
                let k = fe_to_biguint_host(&v);
                if forge::is(forge::SK_PLUS_N) { k + &n } else { k }
            });
            self.fq.assign_biguint(layouter.namespace(|| "load secret key"), v)?
        };
        // sk < n: one integer per scalar, so one nullifier per key
        self.fq.assert_canonical(layouter.namespace(|| "sk < n"), &sk_assigned)?;
        self.fq
            .range_check_limbs(layouter.namespace(|| "range check sk"), &sk_assigned)?;

        // Load public key
        let pk_x_assigned = self
            .fp
            .load_private(layouter.namespace(|| "load pk.x"), epkx)?;
        let pk_y_assigned = self
            .fp
            .load_private(layouter.namespace(|| "load pk.y"), epky)?;
        self.fp
            .range_check_limbs(layouter.namespace(|| "range check pk.x"), &pk_x_assigned)?;
        self.fp
            .range_check_limbs(layouter.namespace(|| "range check pk.y"), &pk_y_assigned)?;
        // canonical coordinates: pk natives (hashed into pk_com) are unique per point
        self.fp.assert_canonical(layouter.namespace(|| "pk.x < p"), &pk_x_assigned)?;
        self.fp.assert_canonical(layouter.namespace(|| "pk.y < p"), &pk_y_assigned)?;

        // esk·G via GLV: k1·G + k2·ψ(G), ψ(x, y) = (βx, y).
        let computed_pk = if KEYPAIR_LADDER.load(core::sync::atomic::Ordering::SeqCst) {
            // Revived ea8ddf5 Montgomery-ladder path (lab variant, selected at synthesis time).
            let g = self.point_from_fp(layouter.namespace(|| "G"), generator_xy())?;
            self.scalar_mul_montgomery(layouter.namespace(|| "ladder scalar mul"), &sk_assigned, &g)?
        } else if self.has_fixed_base() {
            self.scalar_mul_fixed_base(layouter.namespace(|| "fixed-base scalar mul"), &sk_assigned)?
        } else {
            self.scalar_mul_glv(layouter.namespace(|| "glv scalar mul"), &sk_assigned)?
        };

        // Enforce computed_pk == public_key
        self.fp.enforce_equal(
            layouter.namespace(|| "enforce x equal"),
            &computed_pk.0,
            &pk_x_assigned,
        )?;
        self.fp.enforce_equal(
            layouter.namespace(|| "enforce y equal"),
            &computed_pk.1,
            &pk_y_assigned,
        )?;

        Ok((sk_assigned, (pk_x_assigned, pk_y_assigned)))
    }

    /// Convert a secp256k1 Fq element to native Pallas::Base.
    pub fn fq_to_native(
        &self,
        layouter: impl Layouter<pallas::Base>,
        fq: &ProperCrtUint<pallas::Base>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError> {
        self.fq.to_native(layouter, fq)
    }

    /// ECDSA verify under `Q`: `R = (e·s⁻¹)·G + (r·s⁻¹)·Q`, and `R.x ≡ r (mod n)`.
    ///
    /// `e`, `r`, and `s` are secp256k1 scalars. `e` is the challenge the contract
    /// computed as `keccak256` of the EIP-191 claim message, reduced modulo `n`.
    /// `Q` is the eligible public key (a witness). The scalar `esk` is not an input.
    pub fn prove_ecdsa_verify(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        e: Value<Secp256k1Fq>,
        r: Value<Secp256k1Fq>,
        s: Value<Secp256k1Fq>,
        qx: Value<Secp256k1Fp>,
        qy: Value<Secp256k1Fp>,
    ) -> Result<
        (
            AssignedCell<pallas::Base, pallas::Base>,
            AssignedCell<pallas::Base, pallas::Base>,
            ProperCrtUint<pallas::Base>,
        ),
        PlonkError,
    > {
        let e = self.fq.load_private(layouter.namespace(|| "e"), e)?;
        let r = self.fq.load_private(layouter.namespace(|| "r"), r)?;
        let s = self.fq.load_private(layouter.namespace(|| "s"), s)?;
        self.fq.range_check_limbs(layouter.namespace(|| "rc e"), &e)?;
        self.fq.range_check_limbs(layouter.namespace(|| "rc r"), &r)?;
        self.fq.range_check_limbs(layouter.namespace(|| "rc s"), &s)?;

        let one = self.load_pinned_fq(layouter.namespace(|| "one"), Secp256k1Fq::from(1u64))?;
        let s_inv = self.fq.div_checked(layouter.namespace(|| "s inverse"), &one, &s)?;
        let u1 = self.fq.mul(layouter.namespace(|| "u1 = e/s"), &e, &s_inv)?;
        let u2 = self.fq.mul(layouter.namespace(|| "u2 = r/s"), &r, &s_inv)?;

        // Fixed-base table when configured: digits of u1 (< 2^256, possibly non-canonical: same
        // point mod n). Adds 1..30 plain div (bound holds for every digit choice), last add checked.
        let r_g = if self.fixed_base.is_some() {
            self.scalar_mul_fixed_base(layouter.namespace(|| "u1·G fb"), &u1)?
        } else {
            self.scalar_mul_glv(layouter.namespace(|| "u1·G"), &u1)?
        };
        let q = (
            self.fp
                .load_private(layouter.namespace(|| "Q.x"), qx)?,
            self.fp
                .load_private(layouter.namespace(|| "Q.y"), qy)?,
        );
        self.fp
            .range_check_limbs(layouter.namespace(|| "rc Q.x"), &q.0)?;
        self.fp
            .range_check_limbs(layouter.namespace(|| "rc Q.y"), &q.1)?;
        // Variable base: ψ(Q) = (β·x, y). The 2^128 offset is doubled out of Q,
        // not pinned, because it depends on Q.
        let r_q = self.scalar_mul_glv_at(layouter.namespace(|| "u2·Q"), &u2, &q)?;
        let point = self.add_point(layouter.namespace(|| "R = u1G + u2Q"), &r_g, &r_q)?;

        // R.x = m·n + r, with m ∈ {0, 1}, because the base field prime is less than 2n.
        let m_v = point.0.value.as_ref().map(|x| {
            let x = x.to_biguint().expect("x >= 0");
            pallas::Base::from(u64::from(x >= secp_n()))
        });
        let m = self.assign_bit(layouter.namespace(|| "x high"), m_v)?;
        let m_n = self.scale_modulus_by_bit(layouter.namespace(|| "m·n"), &m)?;
        let (sum, cout) =
            self.carry_add_limbs(layouter.namespace(|| "m·n + r"), &m_n, &r.truncation.limbs)?;
        let zero = layouter.assign_region(
            || "x carry zero",
            |mut region| {
                let cell = region.assign_advice(
                    || "zero",
                    self.advice_col(),
                    0,
                    || Value::known(pallas::Base::ZERO),
                )?;
                region.constrain_constant(cell.cell(), pallas::Base::ZERO)?;
                Ok(cell)
            },
        )?;
        self.enforce_carry_equal(
            layouter.namespace(|| "R.x = m·n + r"),
            &sum,
            &cout,
            &point.0.truncation.limbs,
            &zero,
        )?;
        Ok((q.0.native, q.1.native, e))
    }

    /// `[k]P` for a variable point, using the same GLV split as `[k]G`.
    fn scalar_mul_glv_at(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        scalar: &ProperCrtUint<pallas::Base>,
        base: &SecpPoint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let split_v = scalar.value.as_ref().map(|k| {
            let k_big = k.to_biguint().expect("scalar is non-negative");
            glv_split(&k_big)
        });
        let (mag1, sign1, z1_cell, z1, mag2, sign2, z2_cell, z2) =
            self.constrain_glv_split(layouter.namespace(|| "glv split"), scalar, &split_v)?;

        let beta = self.load_pinned_fp(layouter.namespace(|| "beta"), glv_beta_fp())?;
        let psi = (
            self.fp
                .mul(layouter.namespace(|| "β·x"), &beta, &base.0)?,
            base.1.clone(),
        );
        let base_pow = self.double_times(layouter.namespace(|| "2^128·P"), base, 128)?;
        let psi_pow = self.double_times(layouter.namespace(|| "2^128·ψP"), &psi, 128)?;
        let c_base = self.negate_point(layouter.namespace(|| "-2^128·P"), &base_pow)?;
        let c_psi = self.negate_point(layouter.namespace(|| "-2^128·ψP"), &psi_pow)?;
        let base_neg = self.negate_point(layouter.namespace(|| "-P"), base)?;
        let psi_neg = self.negate_point(layouter.namespace(|| "-ψP"), &psi)?;
        let c_base_neg = self.negate_point(layouter.namespace(|| "2^128·P"), &c_base)?;
        let c_psi_neg = self.negate_point(layouter.namespace(|| "2^128·ψP"), &c_psi)?;
        let dbl = self.double_point(layouter.namespace(|| "2P"), base)?;

        let b1 = self.select_point(layouter.namespace(|| "±P"), &base_neg, base, &sign1)?;
        let c1 = self.select_point(
            layouter.namespace(|| "±corr P"),
            &c_base_neg,
            &c_base,
            &sign1,
        )?;
        let b2 = self.select_point(layouter.namespace(|| "±ψP"), &psi_neg, &psi, &sign2)?;
        let c2 = self.select_point(
            layouter.namespace(|| "±corr ψ"),
            &c_psi_neg,
            &c_psi,
            &sign2,
        )?;
        let p1 = self.scalar_mul_128(
            layouter.namespace(|| "k1·P"),
            &b1,
            &mag1,
            &c1,
            z1,
            base,
            &dbl,
        )?;
        let p2 = self.scalar_mul_128(
            layouter.namespace(|| "k2·ψP"),
            &b2,
            &mag2,
            &c2,
            z2,
            base,
            &dbl,
        )?;
        let sum = if z1 || z2 {
            self.add_point(layouter.namespace(|| "var combine dummy"), base, &dbl)?
        } else {
            self.add_point(layouter.namespace(|| "k1 P + k2 ψP"), &p1, &p2)?
        };
        let inner = self.select_point(layouter.namespace(|| "var k1"), &p1, &sum, &z2_cell)?;
        self.select_point(layouter.namespace(|| "var k2"), &p2, &inner, &z1_cell)
    }

    fn double_times(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        point: &SecpPoint<pallas::Base>,
        times: usize,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let mut acc = point.clone();
        for i in 0..times {
            acc = self.double_point(layouter.namespace(|| format!("double {i}")), &acc)?;
        }
        Ok(acc)
    }

    /// Affine negation. Constrains `y + (−y) = p` with the carry chain.
    fn negate_point(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        point: &SecpPoint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let y_neg_v = point.1.value.as_ref().map(|y| {
            let y = y.to_biguint().expect("y >= 0");
            be32_to_fp(&biguint_to_be32(&(secp_p() - y)))
        });
        let y_neg = self
            .fp
            .load_private(layouter.namespace(|| "-y"), y_neg_v)?;
        let (sum, cout) = self.carry_add_limbs(
            layouter.namespace(|| "y + -y"),
            &point.1.truncation.limbs,
            &y_neg.truncation.limbs,
        )?;
        let p_limbs = {
            let d = decompose_biguint_simple(&secp_p(), 4, 64);
            [d[0], d[1], d[2], d[3]]
        };
        self.pin_limbs(layouter.namespace(|| "sum is p"), &sum, &p_limbs)?;
        layouter.assign_region(
            || "neg carry zero",
            |mut region| {
                let c = cout.copy_advice(|| "cout", &mut region, self.advice_col(), 0)?;
                region.constrain_constant(c.cell(), pallas::Base::ZERO)
            },
        )?;
        Ok((point.0.clone(), y_neg))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use halo2_proofs::circuit::Layouter;
    use halo2_proofs::dev::MockProver;
    use halo2_proofs::plonk::Circuit;
    use num_traits::Zero;
    use pasta_curves::pallas;
    use std::println;

    #[test]
    fn test_fe_conversion() {
        let value = pallas::Base::from(123);
        let big = crate::spec::fe_to_biguint_simple(&value);
        let back = crate::spec::biguint_to_fe_simple(&big);
        assert_eq!(value, back);

        let zero = pallas::Base::zero();
        let big_zero = crate::spec::fe_to_biguint_simple(&zero);
        assert!(big_zero.is_zero());
    }

    #[test]
    fn test_glv_split_matches_full_scalar_mul() {
        use secp256k1::{PublicKey, Secp256k1, SecretKey};

        let secp = Secp256k1::new();
        let n = secp_n();
        let lambda = parse_hex_uint(GLV_LAMBDA_HEX);
        let k1_bound = parse_hex_uint("A2A8918CA85BAFE22016D0B917E4DD77");
        let k2_bound = parse_hex_uint("8A65287BD47179FB2BE08846CEA267ED");
        let test_key = BigUint::parse_bytes(
            b"0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
            16,
        )
        .unwrap();
        let keys = [
            BigUint::from(1u64),
            BigUint::from(2u64),
            BigUint::one() << 120usize,
            BigUint::one() << 128usize,
            &n - 1u64,
            test_key,
            BigUint::parse_bytes(b"42", 16).unwrap().pow(32u32),
        ];

        let g = generator_xy();
        let psi = host_psi(g);
        assert_eq!(psi.0, glv_beta_fp() * g.0, "ψ is the β multiply on x");
        assert_eq!(psi.1, g.1);

        for k in keys {
            let split = glv_split(&k);
            let reduced = (&split.k2.residue * &lambda) % &n;
            let sum = (&split.k1.residue + &reduced) % &n;
            assert_eq!(sum, &k % &n, "k1 + λ k2 = k");
            assert!(split.k1.mag < k1_bound, "k1 bound");
            assert!(split.k2.mag < k2_bound, "k2 bound");
            for half in [&split.k1, &split.k2] {
                if half.neg {
                    assert_eq!(&half.residue + &half.mag, n);
                } else {
                    assert_eq!(half.residue, half.mag);
                }
            }

            let got = glv_mul_generator(&k).expect("nonzero scalar");
            let sk = SecretKey::from_byte_array(biguint_to_be32(&k)).unwrap();
            let pk = PublicKey::from_secret_key(&secp, &sk);
            let bytes = pk.serialize_uncompressed();
            let x = secp_fp_from_coord_be(bytes[1..33].try_into().unwrap());
            let y = secp_fp_from_coord_be(bytes[33..65].try_into().unwrap());
            assert_eq!((got.0, got.1), (x, y), "GLV point must equal k·G");

            let mut bumped = split.k1.mag.clone() + 1u64;
            if bumped.bits() > 128 {
                bumped = BigUint::from(1u64);
            }
            let lie = host_mul(if split.k1.neg { host_neg_point(g) } else { g }, &bumped);
            let other = host_mul(
                if split.k2.neg {
                    host_neg_point(psi)
                } else {
                    psi
                },
                &split.k2.mag,
            );
            let lied = match (lie, other) {
                (Some(a), Some(b)) => host_add(a, b),
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => panic!("bumped half still produced infinity"),
            };
            assert_ne!(
                (lied.0, lied.1),
                (x, y),
                "a wrong half must not land on k·G"
            );
        }
    }

    #[test]
    fn test_montgomery_offset_correctness() {
        // Verify that MONTGOMERY_OFFSET_X/Y are the coordinates of -(2^256 * G).
        //
        // The Montgomery ladder computes (2^256 + k)*G. To correct, we add -(2^256 * G).
        // Since EC scalars are mod group order n: 2^256 * G = (2^256 mod n) * G.
        use secp256k1::constants::GENERATOR_X as GX;
        use secp256k1::constants::GENERATOR_Y as GY;

        let secp_p = BigUint::parse_bytes(SECP_P_HEX.as_bytes(), 16).unwrap();
        let secp_n = BigUint::parse_bytes(SECP_N_HEX.as_bytes(), 16).unwrap();

        // s = 2^256 mod n (the effective scalar the ladder adds)
        let two_256 = BigUint::from(1u64) << 256;
        let s: BigUint = &two_256 % &secp_n;

        // Compute s*G using the secp256k1 library
        let s_bytes: [u8; 32] = {
            let b = s.to_bytes_be();
            let mut arr = [0u8; 32];
            arr[32 - b.len()..].copy_from_slice(&b);
            arr
        };
        let secp = secp256k1::Secp256k1::new();
        let sk = secp256k1::SecretKey::from_byte_array(s_bytes).unwrap();
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pk_bytes = pk.serialize_uncompressed();
        // pk_bytes[0] = 0x04, pk_bytes[1..33] = x, pk_bytes[33..65] = y
        let sg_x = BigUint::from_bytes_be(&pk_bytes[1..33]);
        let sg_y = BigUint::from_bytes_be(&pk_bytes[33..65]);

        // -(s*G) has the same x but negated y: neg_y = p - y
        let neg_sg_y = &secp_p - &sg_y;

        // Convert our constants to BigUint for comparison
        let offset_x = secp_fp_from_le(MONTGOMERY_OFFSET_X);
        let offset_y = secp_fp_from_le(MONTGOMERY_OFFSET_Y);
        let offset_x_big =
            BigUint::from_bytes_le(<Secp256k1Fp as PrimeField>::to_repr(&offset_x).as_ref());
        let offset_y_big =
            BigUint::from_bytes_le(<Secp256k1Fp as PrimeField>::to_repr(&offset_y).as_ref());

        assert_eq!(
            offset_x_big, sg_x,
            "MONTGOMERY_OFFSET_X should be x-coord of (2^256 mod n)*G"
        );
        assert_eq!(
            offset_y_big, neg_sg_y,
            "MONTGOMERY_OFFSET_Y should be negated y-coord of (2^256 mod n)*G"
        );
    }
    #[test]
    fn test_fixed_integer_conversion() {
        let value = BigUint::from(1u64) << 100 | BigUint::from(0x01020304u64);
        let fixed = FixedOverflowInteger::from_native(&value, 4, 64);
        let reconstructed = fixed.to_biguint(64);
        assert_eq!(value, reconstructed);
    }

    #[derive(Debug, Default)]
    struct TestCircuit {
        value: BigUint,
    }

    impl Circuit<pallas::Base> for TestCircuit {
        type Config = BigIntConfig;
        type FloorPlanner = halo2_proofs::circuit::SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            Self::default()
        }

        fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
            let advices = [
                meta.advice_column(),
                meta.advice_column(),
                meta.advice_column(),
            ];
            BigIntConfig::configure(meta, advices)
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<pallas::Base>,
        ) -> Result<(), PlonkError> {
            let chip = BigIntChip::construct(config, 8, 4);

            layouter.assign_region(
                || "test region",
                |mut region| {
                    let constant_uint = chip.assign_constant(&mut region, 0, &self.value)?;
                    assert_eq!(constant_uint.num_limbs(), 4);

                    let witness_uint =
                        chip.assign_witness(&mut region, 4, Value::known(&self.value))?;
                    assert_eq!(witness_uint.num_limbs(), 4);

                    Ok(())
                },
            )?;

            Ok(())
        }
    }

    const K: u32 = 3;

    #[test]
    fn test_circuit_assignment() {
        let value = BigUint::from(12345u32);
        let circuit = TestCircuit { value };
        let prover = MockProver::run(17, &circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()));
        let cost =
            halo2_proofs::dev::CircuitCost::<pasta_curves::vesta::Point, _>::measure(K, &circuit);
        let proof_size = usize::from(cost.proof_size(1));
        assert!(proof_size > 0, "Proof size should be non-zero");
        println!(" proof_size: {}", proof_size);
        println!(" cost: {:#?}", cost);
    }

    #[test]
    fn test_c_coefficients() {
        let c = secp_fp_c_coeffs();
        // c[0] should be 1
        assert_eq!(c[0], pallas::Base::ONE);
        // c[1] should be 2^64
        let expected = biguint_to_fe_simple(&(BigUint::from(1u64) << 64));
        assert_eq!(c[1], expected);
        println!("c coefficients computed successfully");
    }
}
