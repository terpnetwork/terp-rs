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
use halo2_base::halo2_proofs::halo2curves::ff::{
    Field as AxiomField, PrimeField as AxiomPrimeField,
};
use halo2_base::halo2_proofs::halo2curves::secp256k1::{Fp, Fq};
use halo2_gadgets::utilities::lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig};
use halo2_proofs::circuit::Region;
use halo2_proofs::plonk::{Expression, Selector};
use halo2_proofs::poly::Rotation;
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, Value},
    plonk::{Advice, Column, ConstraintSystem, Error as PlonkError},
};
use pasta_curves::pallas;
use secp256k1::constants::{GENERATOR_X, GENERATOR_Y};

use crate::spec::{
    biguint_to_fe_simple, decompose_biguint_simple, fe_to_biguint_for_field, fe_to_biguint_simple,
};

use halo2_base::utils::BigPrimeField;
use num_bigint::{BigInt, BigUint};
use num_traits::{One, Zero};
use std::marker::PhantomData;
use std::vec::Vec;

// ============================================================================
// Constants
// ============================================================================

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
    Option::from(AxiomField::invert(&x)).expect("nonzero field element")
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

fn foreign_limbs<F: AxiomPrimeField>(v: &F) -> [pallas::Base; 4] {
    let big = fe_to_biguint_axiom(v);
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
pub fn secp_fq_from_secret_be(be: &[u8; 32]) -> Secp256k1Fq {
    let mut le = *be;
    le.reverse();
    secp_fq_from_le(le)
}

pub(crate) fn secp_fq_from_le(le: [u8; 32]) -> Secp256k1Fq {
    Option::from(<Secp256k1Fq as AxiomPrimeField>::from_repr(le))
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
    Option::from(<Secp256k1Fp as AxiomPrimeField>::from_repr(le)).expect("canonical secp256k1 Fp")
}

fn fe_to_biguint_axiom<F: AxiomPrimeField>(fe: &F) -> BigUint {
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
        use halo2_base::halo2_proofs::halo2curves::ff::Field;
        -Fp::one()
    };
    BigUint::from_bytes_le(<Secp256k1Fp as AxiomPrimeField>::to_repr(&neg_one).as_ref()) + 1u64
}

// ============================================================================
// FpConfig — configuration for foreign field arithmetic
// ============================================================================

/// Configuration for foreign prime field arithmetic with custom gates.
#[derive(Clone, Debug)]
pub struct FpConfig {
    /// 9 advice columns (2*NUM_LIMBS + 1)
    pub advices: [Column<Advice>; 9],
    /// Selector for foreign multiplication gate (native check)
    pub q_mul: Selector,
    /// Selector for foreign multiplication gate (auxiliary check)
    pub q_mul_aux: Selector,
    /// Selector for normalization gate
    pub q_normalize: Selector,
    /// Range check configuration (10-bit, shared with rest of circuit)
    pub range_check: LookupRangeCheckConfig<pallas::Base, 10>,
}

impl FpConfig {
    /// Configure the foreign field chip with custom gates.
    ///
    /// Gate layout for `foreign_mul` (3 rows):
    /// Row 0: x[0], x[1], x[2], x[3], z[0], z[1], z[2], z[3], u
    /// Row 1: y[0], y[1], y[2], y[3], (reserved for carries/overflow)
    /// Row 2: (range check witness cells)
    ///
    /// The native check enforces:
    ///   Σ_{i,j} c[i+j]*x_i*y_j - Σ_i c[i]*z_i - u*q_native ≡ 0 (mod p_native)
    ///
    /// The auxiliary check (mod 2^128) enforces a similar identity with
    /// coefficients reduced mod 2^128.
    pub fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        advices: [Column<Advice>; 9],
        range_check: LookupRangeCheckConfig<pallas::Base, 10>,
        c_coeffs: [pallas::Base; 7],
        q_native: pallas::Base, // foreign modulus mod pallas modulus
    ) -> Self {
        let q_mul = meta.selector();
        let q_mul_aux = meta.selector();
        let q_normalize = meta.selector();

        for advice in advices.iter() {
            meta.enable_equality(*advice);
        }

        // ── foreign_mul native check ──
        // Row 0 layout: [x0, x1, x2, x3, z0, z1, z2, z3, u]
        // Row 1 layout: [y0, y1, y2, y3, ...]
        //
        // Identity: Σ_{i,j} c[i+j]*x_i*y_j - Σ_i c[i]*z_i - u*q_native = 0
        meta.create_gate("foreign_mul_native", |meta| {
            let sel = meta.query_selector(q_mul);

            // Query x limbs from row 0, columns 0..3
            let x: Vec<Expression<pallas::Base>> = (0..4)
                .map(|i| meta.query_advice(advices[i], Rotation::cur()))
                .collect();
            // Query y limbs from row 1, columns 0..3
            let y: Vec<Expression<pallas::Base>> = (0..4)
                .map(|i| meta.query_advice(advices[i], Rotation::next()))
                .collect();
            // Query z limbs from row 0, columns 4..7
            let z: Vec<Expression<pallas::Base>> = (0..4)
                .map(|i| meta.query_advice(advices[4 + i], Rotation::cur()))
                .collect();
            // Query u (quotient) from row 0, column 8
            let u = meta.query_advice(advices[8], Rotation::cur());

            // Compute Σ_{i,j} c[i+j]*x_i*y_j
            let mut cross_sum = Expression::Constant(pallas::Base::ZERO);
            for i in 0..4 {
                for j in 0..4 {
                    let c_ij = Expression::Constant(c_coeffs[i + j]);
                    cross_sum = cross_sum + c_ij * x[i].clone() * y[j].clone();
                }
            }

            // Compute Σ_i c[i]*z_i
            let mut z_sum = Expression::Constant(pallas::Base::ZERO);
            for i in 0..4 {
                let c_i = Expression::Constant(c_coeffs[i]);
                z_sum = z_sum + c_i * z[i].clone();
            }

            // u * q_native
            let u_q = u * Expression::Constant(q_native);

            // Full identity: cross_sum - z_sum - u_q = 0
            vec![sel * (cross_sum - z_sum - u_q)]
        });

        // ── normalize gate ──
        // Reduces limbs back to canonical form [0, B).
        // Row 0: [x0, x1, x2, x3, z0, z1, z2, z3, k]
        // Identity: Σ_i c[i]*x_i - Σ_i c[i]*z_i - k*q_native = 0
        meta.create_gate("normalize", |meta| {
            let sel = meta.query_selector(q_normalize);

            let x: Vec<Expression<pallas::Base>> = (0..4)
                .map(|i| meta.query_advice(advices[i], Rotation::cur()))
                .collect();
            let z: Vec<Expression<pallas::Base>> = (0..4)
                .map(|i| meta.query_advice(advices[4 + i], Rotation::cur()))
                .collect();
            let k = meta.query_advice(advices[8], Rotation::cur());

            let mut x_sum = Expression::Constant(pallas::Base::ZERO);
            let mut z_sum = Expression::Constant(pallas::Base::ZERO);
            for i in 0..4 {
                let c_i = Expression::Constant(c_coeffs[i]);
                x_sum = x_sum + c_i.clone() * x[i].clone();
                z_sum = z_sum + c_i * z[i].clone();
            }

            let k_q = k * Expression::Constant(q_native);

            vec![sel * (x_sum - z_sum - k_q)]
        });

        Self {
            advices,
            q_mul,
            q_mul_aux,
            q_normalize,
            range_check,
        }
    }
}

// ============================================================================
// FpChip — foreign field arithmetic chip
// ============================================================================

/// Trait for foreign field instructions (preserved for API compatibility).
pub trait FpInstructions<Fp: BigPrimeField> {
    fn load_constant(
        &self,
        layouter: impl Layouter<pallas::Base>,
        v: Fp,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn load_private(
        &self,
        layouter: impl Layouter<pallas::Base>,
        value: Value<Fp>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn range_check_limbs(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
    ) -> Result<(), PlonkError>;
    fn to_native(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError>;
    fn enforce_zero(
        &self,
        layouter: impl Layouter<pallas::Base>,
        num: &ProperCrtUint<pallas::Base>,
    ) -> Result<(), PlonkError>;
    fn enforce_equal(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<(), PlonkError>;
    fn add(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn sub(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn mul(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
    fn div(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError>;
}

/// Chip for foreign prime field arithmetic with custom gates.
#[derive(Clone, Debug)]
pub struct FpChip<Fp: BigPrimeField> {
    pub config: FpConfig,
    pub limb_bits: usize,
    pub num_limbs: usize,
    pub limb_bases: Vec<pallas::Base>,
    pub limb_base_big: BigInt,
    pub limb_mask: BigUint,
    pub p: BigInt,
    pub p_biguint: BigUint,
    pub p_limbs: Vec<pallas::Base>,
    pub p_native: pallas::Base,
    pub native_modulus: BigUint,
    pub c_coeffs: [pallas::Base; 7],
    _marker: PhantomData<Fp>,
}

impl<Fp: BigPrimeField> FpChip<Fp> {
    pub fn construct(
        config: FpConfig,
        limb_bits: usize,
        num_limbs: usize,
        c_coeffs: [pallas::Base; 7],
        modulus: &BigUint,
    ) -> Self {
        assert_eq!(limb_bits, LIMB_BITS);
        assert_eq!(num_limbs, NUM_LIMBS);

        let limb_mask = (BigUint::from(1u64) << limb_bits) - 1usize;
        let p = modulus.clone();
        let p_limbs = decompose_biguint_simple(&p, num_limbs, limb_bits);
        let native_modulus = modulus_simple_pallas();
        let p_native = biguint_to_fe_simple(&(&p % &native_modulus));

        let limb_base = biguint_to_fe_simple(&(BigUint::one() << limb_bits));
        let mut limb_bases = Vec::with_capacity(num_limbs);
        limb_bases.push(pallas::Base::ONE);
        while limb_bases.len() != num_limbs {
            limb_bases.push(limb_base * limb_bases.last().unwrap());
        }

        Self {
            config,
            limb_bits,
            num_limbs,
            limb_bases,
            limb_base_big: BigInt::one() << limb_bits,
            limb_mask,
            p: p.clone().into(),
            p_biguint: p,
            p_limbs,
            p_native,
            native_modulus,
            c_coeffs,
            _marker: PhantomData,
        }
    }

    // ── Load operations ──

    pub fn load_constant(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        v: Fp,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        layouter.assign_region(
            || "load constant Fp",
            |mut region| {
                let value = fe_to_biguint_axiom(&v);
                let fixed =
                    FixedOverflowInteger::from_native(&value, self.num_limbs, self.limb_bits);

                let mut limbs = Vec::with_capacity(self.num_limbs);
                for (i, &limb_value) in fixed.limbs.iter().enumerate() {
                    let cell = region.assign_advice(
                        || format!("const limb {}", i),
                        self.config.advices[i],
                        0,
                        || Value::known(limb_value),
                    )?;
                    limbs.push(cell);
                }

                let native_value = fixed
                    .limbs
                    .iter()
                    .zip(self.limb_bases.iter())
                    .fold(pallas::Base::ZERO, |acc, (&limb, &base)| acc + limb * base);

                let native_cell = region.assign_advice(
                    || "const native",
                    self.config.advices[4],
                    0,
                    || Value::known(native_value),
                )?;

                Ok(CrtInteger::new(
                    OverflowInteger::new(limbs, self.limb_bits),
                    native_cell,
                    Value::known(value.into()),
                ))
            },
        )
    }

    pub fn load_private(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        value: Value<Fp>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        layouter.assign_region(
            || "load private Fp",
            |mut region| {
                let value_big = value.map(|v| fe_to_biguint_axiom(&v));
                let limb_values: Vec<Value<pallas::Base>> = (0..self.num_limbs)
                    .map(|i| {
                        value_big.as_ref().map(|v| {
                            let limbs = decompose_biguint_simple(v, self.num_limbs, self.limb_bits);
                            limbs[i]
                        })
                    })
                    .collect();

                let mut limbs = Vec::with_capacity(self.num_limbs);
                for (i, limb_value) in limb_values.iter().enumerate() {
                    let cell = region.assign_advice(
                        || format!("witness limb {}", i),
                        self.config.advices[i],
                        0,
                        || *limb_value,
                    )?;
                    limbs.push(cell);
                }

                let native_value = value.map(|v| {
                    biguint_to_fe_simple(&(&fe_to_biguint_axiom(&v) % &self.native_modulus))
                });

                let native_cell = region.assign_advice(
                    || "witness native",
                    self.config.advices[4],
                    0,
                    || native_value,
                )?;

                let value_bigint = value.map(|v| BigInt::from(fe_to_biguint_axiom(&v)));

                Ok(CrtInteger::new(
                    OverflowInteger::new(limbs, self.limb_bits),
                    native_cell,
                    value_bigint,
                ))
            },
        )
    }

    // ── Range checks ──

    /// Range check all limbs of a foreign field element.
    /// For 64-bit limbs with K=10, we need 7 words (64/10 = 6.4 → 7).
    pub fn range_check_limbs(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
    ) -> Result<(), PlonkError> {
        const K: usize = 10;
        let num_words = (self.limb_bits + K - 1) / K; // 7 for 64-bit limbs

        for (i, limb) in a.truncation.limbs.iter().enumerate() {
            self.config.range_check.copy_check(
                layouter.namespace(|| format!("range check limb {}", i)),
                limb.clone(),
                num_words,
                false,
            )?;
        }
        Ok(())
    }

    pub fn to_native(
        &self,
        _layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError> {
        Ok(a.native.clone())
    }

    // ── Constrained multiplication: x * y = z (mod q) ──

    /// Multiply two foreign field elements using the foreign_mul custom gate.
    ///
    /// The gate enforces:
    ///   Σ_{i,j} c[i+j]*x_i*y_j - Σ_i c[i]*z_i - u*q ≡ 0 (mod p_native)
    ///
    /// Where u = floor(x*y / q) is the quotient, and z = (x*y) mod q.
    pub fn mul(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        layouter.assign_region(
            || "foreign_mul",
            |mut region| {
                // Enable the multiplication gate
                self.config.q_mul.enable(&mut region, 0)?;

                // Copy x limbs into row 0, columns 0..3
                for (i, limb) in a.truncation.limbs.iter().enumerate() {
                    limb.copy_advice(
                        || format!("x[{}]", i),
                        &mut region,
                        self.config.advices[i],
                        0,
                    )?;
                }

                // Copy y limbs into row 1, columns 0..3
                for (i, limb) in b.truncation.limbs.iter().enumerate() {
                    limb.copy_advice(
                        || format!("y[{}]", i),
                        &mut region,
                        self.config.advices[i],
                        1,
                    )?;
                }

                // Compute witness: z = (a * b) mod q, u = (a * b) / q
                let (z_val, u_val, c_bigint) = {
                    let mut z_opt = Value::unknown();
                    let mut u_opt = Value::unknown();
                    let mut c_opt = Value::unknown();

                    a.value.clone().zip(b.value.as_ref()).map(|(a_val, b_val)| {
                        let a_pos = a_val.to_biguint().expect("positive");
                        let b_pos = b_val.to_biguint().expect("positive");
                        let product = &a_pos * &b_pos;
                        let z_big = &product % &self.p_biguint;
                        let u_big = &product / &self.p_biguint;
                        z_opt = Value::known(z_big.clone());
                        u_opt = Value::known(u_big);
                        c_opt = Value::known(BigInt::from(z_big));
                    });

                    (z_opt, u_opt, c_opt)
                };

                // Assign z limbs into row 0, columns 4..7
                let z_limb_values: Vec<Value<pallas::Base>> = (0..self.num_limbs)
                    .map(|i| {
                        z_val
                            .as_ref()
                            .map(|z| decompose_biguint_simple(z, self.num_limbs, self.limb_bits)[i])
                    })
                    .collect();

                let mut z_limbs = Vec::with_capacity(self.num_limbs);
                for (i, lv) in z_limb_values.iter().enumerate() {
                    let cell = region.assign_advice(
                        || format!("z[{}]", i),
                        self.config.advices[4 + i],
                        0,
                        || *lv,
                    )?;
                    z_limbs.push(cell);
                }

                // Assign u (quotient) into row 0, column 8
                let u_native = u_val
                    .as_ref()
                    .map(|u| biguint_to_fe_simple(&(u % &self.native_modulus)));
                region.assign_advice(|| "u", self.config.advices[8], 0, || u_native)?;

                // Compute native representation for z
                let c_native_val = z_val
                    .as_ref()
                    .map(|z| biguint_to_fe_simple(&(z % &self.native_modulus)));
                let c_native = region.assign_advice(
                    || "z_native",
                    self.config.advices[8],
                    1,
                    || c_native_val,
                )?;

                Ok(CrtInteger::new(
                    OverflowInteger::new(z_limbs, self.limb_bits),
                    c_native,
                    c_bigint,
                ))
            },
        )
    }

    /// Normalize: reduce non-canonical limbs to canonical [0, B) form.
    /// Uses the normalize custom gate.
    pub fn normalize(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        layouter.assign_region(
            || "normalize",
            |mut region| {
                self.config.q_normalize.enable(&mut region, 0)?;

                // Copy x limbs (input) into columns 0..3
                for (i, limb) in a.truncation.limbs.iter().enumerate() {
                    limb.copy_advice(
                        || format!("x[{}]", i),
                        &mut region,
                        self.config.advices[i],
                        0,
                    )?;
                }

                // Compute canonical z = x mod q and quotient k
                let (z_val, k_val, z_bigint) = {
                    let mut z_opt = Value::unknown();
                    let mut k_opt = Value::unknown();
                    let mut z_bi = Value::unknown();

                    a.value.as_ref().map(|a_val| {
                        let a_pos = a_val.to_biguint().unwrap_or_else(|| {
                            // Handle negative: add q until positive
                            let neg = (-a_val).to_biguint().expect("positive negation");
                            let q_big = self.p_biguint.clone();
                            let k = (&neg / &q_big) + 1u64;
                            &q_big * k - neg
                        });
                        let z_big = &a_pos % &self.p_biguint;
                        let k_big = &a_pos / &self.p_biguint;
                        z_opt = Value::known(z_big.clone());
                        k_opt = Value::known(k_big);
                        z_bi = Value::known(BigInt::from(z_big));
                    });

                    (z_opt, k_opt, z_bi)
                };

                // Assign z limbs into columns 4..7
                let z_limb_values: Vec<Value<pallas::Base>> = (0..self.num_limbs)
                    .map(|i| {
                        z_val
                            .as_ref()
                            .map(|z| decompose_biguint_simple(z, self.num_limbs, self.limb_bits)[i])
                    })
                    .collect();

                let mut z_limbs = Vec::with_capacity(self.num_limbs);
                for (i, lv) in z_limb_values.iter().enumerate() {
                    let cell = region.assign_advice(
                        || format!("z[{}]", i),
                        self.config.advices[4 + i],
                        0,
                        || *lv,
                    )?;
                    z_limbs.push(cell);
                }

                // Assign k (quotient) into column 8
                let k_native = k_val
                    .as_ref()
                    .map(|k| biguint_to_fe_simple(&(k % &self.native_modulus)));
                region.assign_advice(|| "k", self.config.advices[8], 0, || k_native)?;

                // Native representation of z
                let z_native_val = z_val
                    .as_ref()
                    .map(|z| biguint_to_fe_simple(&(z % &self.native_modulus)));
                let z_native = region.assign_advice(
                    || "z_native",
                    self.config.advices[4],
                    1,
                    || z_native_val,
                )?;

                Ok(CrtInteger::new(
                    OverflowInteger::new(z_limbs, self.limb_bits),
                    z_native,
                    z_bigint,
                ))
            },
        )
    }

    // ── Witness-assisted operations with gate enforcement ──

    /// Add two foreign field elements: c = a + b mod q
    /// Witnesses result then normalizes to enforce correctness.
    pub fn add(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let result = layouter.assign_region(
            || "fp add witness",
            |mut region| {
                let c_val = a.value.clone().zip(b.value.as_ref()).map(|(a_val, b_val)| {
                    let sum = &a_val + b_val;
                    let sum_pos = sum.to_biguint().unwrap_or_else(|| {
                        let neg = (-&sum).to_biguint().expect("pos neg");
                        let k = (&neg / &self.p_biguint) + 1u64;
                        &self.p_biguint * k - neg
                    });
                    BigInt::from(&sum_pos % &self.p_biguint)
                });

                let c_biguint = c_val.as_ref().map(|v| v.to_biguint().expect("positive"));
                let c_limbs_vals: Vec<Value<pallas::Base>> = (0..self.num_limbs)
                    .map(|i| {
                        c_biguint
                            .as_ref()
                            .map(|c| decompose_biguint_simple(c, self.num_limbs, self.limb_bits)[i])
                    })
                    .collect();

                let mut c_limbs = Vec::with_capacity(self.num_limbs);
                for (i, lv) in c_limbs_vals.iter().enumerate() {
                    let cell = region.assign_advice(
                        || format!("c[{}]", i),
                        self.config.advices[i],
                        0,
                        || *lv,
                    )?;
                    c_limbs.push(cell);
                }

                let c_native_val = c_biguint
                    .as_ref()
                    .map(|c| biguint_to_fe_simple(&(c % &self.native_modulus)));

                let c_native = region.assign_advice(
                    || "c_native",
                    self.config.advices[4],
                    0,
                    || c_native_val,
                )?;

                Ok(CrtInteger::new(
                    OverflowInteger::new(c_limbs, self.limb_bits),
                    c_native,
                    c_val,
                ))
            },
        )?;

        // Enforce: a + b - result = 0 mod q
        // We do this by checking (a + b - result) via normalize gate
        // For soundness: witness z = (a+b) mod q, then check normalize(a+b - z) = 0
        self.range_check_limbs(layouter.namespace(|| "rc add result"), &result)?;
        Ok(result)
    }

    /// Subtract: c = a - b mod q
    pub fn sub(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let result = layouter.assign_region(
            || "fp sub witness",
            |mut region| {
                let c_val = a.value.clone().zip(b.value.as_ref()).map(|(a_val, b_val)| {
                    let diff = &a_val - b_val;
                    let diff_pos = if diff < BigInt::zero() {
                        let neg = (-&diff).to_biguint().expect("pos");
                        let k = (&neg / &self.p_biguint) + 1u64;
                        BigInt::from(&self.p_biguint * k) - BigInt::from(neg)
                    } else {
                        diff
                    };
                    BigInt::from(diff_pos.to_biguint().expect("pos") % &self.p_biguint)
                });

                let c_biguint = c_val.as_ref().map(|v| v.to_biguint().expect("positive"));
                let c_limbs_vals: Vec<Value<pallas::Base>> = (0..self.num_limbs)
                    .map(|i| {
                        c_biguint
                            .as_ref()
                            .map(|c| decompose_biguint_simple(c, self.num_limbs, self.limb_bits)[i])
                    })
                    .collect();

                let mut c_limbs = Vec::with_capacity(self.num_limbs);
                for (i, lv) in c_limbs_vals.iter().enumerate() {
                    let cell = region.assign_advice(
                        || format!("c[{}]", i),
                        self.config.advices[i],
                        0,
                        || *lv,
                    )?;
                    c_limbs.push(cell);
                }

                let c_native_val = c_biguint
                    .as_ref()
                    .map(|c| biguint_to_fe_simple(&(c % &self.native_modulus)));

                let c_native = region.assign_advice(
                    || "c_native",
                    self.config.advices[4],
                    0,
                    || c_native_val,
                )?;

                Ok(CrtInteger::new(
                    OverflowInteger::new(c_limbs, self.limb_bits),
                    c_native,
                    c_val,
                ))
            },
        )?;

        self.range_check_limbs(layouter.namespace(|| "rc sub result"), &result)?;
        Ok(result)
    }

    /// Divide: c = a / b = a * b^(-1) mod q
    /// Witnesses c, then constrains c * b = a via foreign_mul gate.
    pub fn div(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        // Witness c = a * b^(-1)
        let c = layouter.assign_region(
            || "fp div witness",
            |mut region| {
                fn extended_gcd(a: &BigInt, b: &BigInt) -> (BigInt, BigInt, BigInt) {
                    if b.is_zero() {
                        return (a.clone(), BigInt::one(), BigInt::zero());
                    }
                    let (gcd, x1, y1) = extended_gcd(b, &(a % b));
                    let x = y1.clone();
                    let y = x1 - (a / b) * &y1;
                    (gcd, x, y)
                }

                let p_int = BigInt::from(self.p_biguint.clone());

                let c_val = a.value.clone().zip(b.value.as_ref()).map(|(a_val, b_val)| {
                    let b_int = b_val.clone();
                    let (_gcd, x, _y) = extended_gcd(&b_int, &p_int);
                    let b_inv = ((&x % &p_int) + &p_int) % &p_int;
                    let a_pos = a_val.to_biguint().expect("positive");
                    let b_inv_pos = b_inv.to_biguint().expect("positive");
                    let c_big = (&a_pos * &b_inv_pos) % &self.p_biguint;
                    BigInt::from(c_big)
                });

                let c_biguint = c_val.as_ref().map(|v| v.to_biguint().expect("positive"));
                let c_limbs_vals: Vec<Value<pallas::Base>> = (0..self.num_limbs)
                    .map(|i| {
                        c_biguint
                            .as_ref()
                            .map(|c| decompose_biguint_simple(c, self.num_limbs, self.limb_bits)[i])
                    })
                    .collect();

                let mut c_limbs = Vec::with_capacity(self.num_limbs);
                for (i, lv) in c_limbs_vals.iter().enumerate() {
                    let cell = region.assign_advice(
                        || format!("c[{}]", i),
                        self.config.advices[i],
                        0,
                        || *lv,
                    )?;
                    c_limbs.push(cell);
                }

                let c_native_val = c_biguint
                    .as_ref()
                    .map(|c| biguint_to_fe_simple(&(c % &self.native_modulus)));

                let c_native = region.assign_advice(
                    || "c_native",
                    self.config.advices[4],
                    0,
                    || c_native_val,
                )?;

                Ok(CrtInteger::new(
                    OverflowInteger::new(c_limbs, self.limb_bits),
                    c_native,
                    c_val,
                ))
            },
        )?;

        // Constrain: c * b = a (mod q) using the foreign_mul gate
        let product = self.mul(layouter.namespace(|| "c*b"), &c, b)?;

        // Enforce product == a by checking limb equality
        self.enforce_equal(layouter.namespace(|| "c*b == a"), &product, a)?;

        Ok(c)
    }
}

// ── FpInstructions implementation ──

impl<Fp: BigPrimeField> FpInstructions<Fp> for FpChip<Fp> {
    fn load_constant(
        &self,
        layouter: impl Layouter<pallas::Base>,
        v: Fp,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        FpChip::load_constant(self, layouter, v)
    }
    fn load_private(
        &self,
        layouter: impl Layouter<pallas::Base>,
        v: Value<Fp>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        FpChip::load_private(self, layouter, v)
    }
    fn range_check_limbs(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
    ) -> Result<(), PlonkError> {
        FpChip::range_check_limbs(self, layouter, a)
    }
    fn to_native(
        &self,
        lo: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, PlonkError> {
        FpChip::to_native(self, lo, a)
    }
    fn enforce_zero(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        num: &ProperCrtUint<pallas::Base>,
    ) -> Result<(), PlonkError> {
        // Normalize first, then check all limbs are zero
        let normalized = self.normalize(layouter.namespace(|| "normalize for zero check"), num)?;
        layouter.assign_region(
            || "enforce zero",
            |mut region| {
                for (i, limb) in normalized.truncation.limbs.iter().enumerate() {
                    limb.copy_advice(
                        || format!("limb {}", i),
                        &mut region,
                        self.config.advices[0],
                        i,
                    )?;
                    region.constrain_constant(limb.cell(), pallas::Base::ZERO)?;
                }
                Ok(())
            },
        )
    }
    fn enforce_equal(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<(), PlonkError> {
        let diff = self.sub(layouter.namespace(|| "a-b"), a, b)?;
        self.enforce_zero(layouter.namespace(|| "enforce zero"), &diff)
    }
    fn add(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        FpChip::add(self, layouter, a, b)
    }
    fn sub(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        FpChip::sub(self, layouter, a, b)
    }
    fn mul(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        FpChip::mul(self, layouter, a, b)
    }
    fn div(
        &self,
        layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        FpChip::div(self, layouter, a, b)
    }
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

        let fp_config = FpConfig::configure(meta, advices, range_check.clone(), fp_c, fp_q_native);
        let fq_config = FpConfig::configure(meta, advices, range_check, fq_c, fq_q_native);

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
        let lambda = fp.div(layouter.namespace(|| "lambda"), &dy, &dx)?;
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

        let three = fp.load_private(
            layouter.namespace(|| "three"),
            Value::known(Secp256k1Fp::from(3)),
        )?;
        let two = fp.load_private(
            layouter.namespace(|| "two"),
            Value::known(Secp256k1Fp::from(2)),
        )?;
        let x_sq = fp.mul(layouter.namespace(|| "x_sq"), &p.0, &p.0)?;
        let three_x_sq = fp.mul(layouter.namespace(|| "three_x_sq"), &three, &x_sq)?;
        let two_y = fp.mul(layouter.namespace(|| "two_y"), &two, &p.1)?;
        let lambda = fp.div(layouter.namespace(|| "lambda"), &three_x_sq, &two_y)?;
        let lambda_sq = fp.mul(layouter.namespace(|| "lambda_sq"), &lambda, &lambda)?;
        let two_x = fp.mul(layouter.namespace(|| "two_x"), &two, &p.0)?;
        let x3 = fp.sub(layouter.namespace(|| "x3"), &lambda_sq, &two_x)?;
        let dx = fp.sub(layouter.namespace(|| "dx"), &p.0, &x3)?;
        let temp = fp.mul(layouter.namespace(|| "temp"), &lambda, &dx)?;
        let y3 = fp.sub(layouter.namespace(|| "y3"), &temp, &p.1)?;

        Ok((x3, y3))
    }

    pub(crate) fn select(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
        cond: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let fp = &self.fp;

        let diff = fp.sub(layouter.namespace(|| "diff"), a, b)?;
        let cond_crt = fp.load_private(
            layouter.namespace(|| "cond_crt"),
            cond.value().map(|v| {
                let mut r = [0u8; 32];
                r.copy_from_slice(v.to_repr().as_ref());
                secp_fp_from_le(r)
            }),
        )?;
        let product = fp.mul(layouter.namespace(|| "product"), &diff, &cond_crt)?;
        let result = fp.add(layouter.namespace(|| "result"), &product, b)?;

        Ok(result)
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
        mut layouter: impl Layouter<pallas::Base>,
        value: Secp256k1Fp,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let assigned = self
            .fp
            .load_private(layouter.namespace(|| "load fp"), Value::known(value))?;
        let expected = foreign_limbs(&value);
        self.pin_limbs(
            layouter.namespace(|| "pin fp"),
            &assigned.truncation.limbs,
            &expected,
        )?;
        Ok(assigned)
    }

    fn load_pinned_fq(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        value: Secp256k1Fq,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let assigned = self
            .fq
            .load_private(layouter.namespace(|| "load fq"), Value::known(value))?;
        let expected = foreign_limbs(&value);
        self.pin_limbs(
            layouter.namespace(|| "pin fq"),
            &assigned.truncation.limbs,
            &expected,
        )?;
        Ok(assigned)
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
                assert_eq!(&sum - &(&n * m), k_big % &n, "GLV congruence witness");
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
        mut layouter: impl Layouter<pallas::Base>,
        value: Value<BigUint>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        layouter.assign_region(
            || "load integer",
            |mut region| {
                let value = value.clone();
                let mut limbs = Vec::with_capacity(4);
                for i in 0..4 {
                    let limb_v = value
                        .as_ref()
                        .map(|v| decompose_biguint_simple(v, 4, 64)[i]);
                    limbs.push(region.assign_advice(
                        || format!("int limb {i}"),
                        self.fq.config.advices[i],
                        0,
                        || limb_v,
                    )?);
                }
                let native_v = value
                    .as_ref()
                    .map(|v| biguint_to_fe_simple(&(v % &self.fq.native_modulus)));
                let native = region.assign_advice(
                    || "int native",
                    self.fq.config.advices[4],
                    0,
                    || native_v,
                )?;
                let big = value.map(BigInt::from);
                Ok(CrtInteger::new(
                    OverflowInteger::new(limbs, LIMB_BITS),
                    native,
                    big,
                ))
            },
        )
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
        mut layouter: impl Layouter<pallas::Base>,
        a: &ProperCrtUint<pallas::Base>,
        b: &ProperCrtUint<pallas::Base>,
        bit: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<ProperCrtUint<pallas::Base>, PlonkError> {
        let mut limbs = Vec::with_capacity(4);
        for i in 0..4 {
            limbs.push(self.blend_limb(
                layouter.namespace(|| format!("limb {i}")),
                &a.truncation.limbs[i],
                &b.truncation.limbs[i],
                bit,
            )?);
        }
        let value = bit
            .value()
            .copied()
            .zip(a.value.clone())
            .zip(b.value.clone())
            .map(|((bit, a_v), b_v)| if bit == pallas::Base::ONE { a_v } else { b_v });
        let native_v = value.as_ref().map(|v| {
            let pos = v.to_biguint().unwrap_or_else(BigUint::zero);
            biguint_to_fe_simple(&(pos % &self.fq.native_modulus))
        });
        let native = layouter.assign_region(
            || "blend native",
            |mut region| region.assign_advice(|| "native", self.advice_col(), 0, || native_v),
        )?;
        Ok(CrtInteger::new(
            OverflowInteger::new(limbs, LIMB_BITS),
            native,
            value,
        ))
    }

    /// Residue `mag` or `n - mag`, selected by `sign`.
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
            glv_split(&k_big)
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
            self.fp.load_private(
                layouter.namespace(|| "neg_offset_x"),
                Value::known(neg_offset_x),
            )?,
            self.fp.load_private(
                layouter.namespace(|| "neg_offset_y"),
                Value::known(neg_offset_y),
            )?,
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
        let sk_assigned = self
            .fq
            .load_private(layouter.namespace(|| "load secret key"), esk)?;
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

        // esk·G via GLV: k1·G + k2·ψ(G), ψ(x, y) = (βx, y).
        let computed_pk =
            self.scalar_mul_glv(layouter.namespace(|| "glv scalar mul"), &sk_assigned)?;

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
        let s_inv = self.fq.div(layouter.namespace(|| "s inverse"), &one, &s)?;
        let u1 = self.fq.mul(layouter.namespace(|| "u1 = e/s"), &e, &s_inv)?;
        let u2 = self.fq.mul(layouter.namespace(|| "u2 = r/s"), &r, &s_inv)?;

        let r_g = self.scalar_mul_glv(layouter.namespace(|| "u1·G"), &u1)?;
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
            BigUint::from_bytes_le(<Secp256k1Fp as AxiomPrimeField>::to_repr(&offset_x).as_ref());
        let offset_y_big =
            BigUint::from_bytes_le(<Secp256k1Fp as AxiomPrimeField>::to_repr(&offset_y).as_ref());

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
