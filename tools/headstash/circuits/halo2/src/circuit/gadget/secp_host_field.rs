//! Host secp256k1 field and scalar. Little-endian canonical integers.
//! Same `from_repr` / `to_repr` convention as halo2curves, without halo2-base.

use std::iter::{Product, Sum};
use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use ff::{Field, PrimeField};
use num_bigint::BigUint;
use num_traits::{One, Zero};
use rand_core::TryRng;
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq, CtOption};

const P_HEX: &str = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F";
const N_HEX: &str = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141";

fn le(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..bytes.len().min(32)].copy_from_slice(&bytes[..bytes.len().min(32)]);
    out
}

fn hex_mod(hex: &str) -> BigUint {
    BigUint::parse_bytes(hex.as_bytes(), 16).expect("modulus")
}

macro_rules! secp_field {
    ($name:ident, $hex:literal, $s:expr, $two:expr, $gen:expr, $root:expr, $root_inv:expr, $delta:expr) => {
        #[derive(Copy, Clone, Debug, Default)]
        pub struct $name([u8; 32]);

        impl $name {
            fn modulus() -> BigUint {
                hex_mod($hex)
            }
            fn big(&self) -> BigUint {
                BigUint::from_bytes_le(&self.0)
            }
            fn from_big(n: BigUint) -> Self {
                let n = n % Self::modulus();
                Self(le(&n.to_bytes_le()))
            }
        }

        impl From<u64> for $name {
            fn from(v: u64) -> Self {
                let mut b = [0u8; 32];
                b[..8].copy_from_slice(&v.to_le_bytes());
                Self(b)
            }
        }

        impl $name {
            /// Little-endian canonical bytes, same convention as PrimeField::from_repr.
            pub fn from_bytes(bytes: &[u8; 32]) -> Option<Self> {
                Option::from(<Self as PrimeField>::from_repr(*bytes))
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }
        impl Eq for $name {}

        impl ConstantTimeEq for $name {
            fn ct_eq(&self, other: &Self) -> Choice {
                self.0.ct_eq(&other.0)
            }
        }

        impl ConditionallySelectable for $name {
            fn conditional_select(a: &Self, b: &Self, choice: Choice) -> Self {
                let mut out = [0u8; 32];
                for i in 0..32 {
                    out[i] = u8::conditional_select(&a.0[i], &b.0[i], choice);
                }
                Self(out)
            }
        }

        impl Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                if self.big().is_zero() {
                    self
                } else {
                    Self::from_big(Self::modulus() - self.big())
                }
            }
        }
        impl Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self::from_big(self.big() + rhs.big())
            }
        }
        impl Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                let m = Self::modulus();
                Self::from_big(self.big() + &m - rhs.big())
            }
        }
        impl Mul for $name {
            type Output = Self;
            fn mul(self, rhs: Self) -> Self {
                Self::from_big(self.big() * rhs.big())
            }
        }
        impl Add<&$name> for $name {
            type Output = Self;
            fn add(self, rhs: &$name) -> Self {
                self + *rhs
            }
        }
        impl Sub<&$name> for $name {
            type Output = Self;
            fn sub(self, rhs: &$name) -> Self {
                self - *rhs
            }
        }
        impl Mul<&$name> for $name {
            type Output = Self;
            fn mul(self, rhs: &$name) -> Self {
                self * *rhs
            }
        }
        impl AddAssign for $name {
            fn add_assign(&mut self, rhs: Self) {
                *self = *self + rhs;
            }
        }
        impl SubAssign for $name {
            fn sub_assign(&mut self, rhs: Self) {
                *self = *self - rhs;
            }
        }
        impl MulAssign for $name {
            fn mul_assign(&mut self, rhs: Self) {
                *self = *self * rhs;
            }
        }
        impl AddAssign<&$name> for $name {
            fn add_assign(&mut self, rhs: &$name) {
                *self = *self + *rhs;
            }
        }
        impl SubAssign<&$name> for $name {
            fn sub_assign(&mut self, rhs: &$name) {
                *self = *self - *rhs;
            }
        }
        impl MulAssign<&$name> for $name {
            fn mul_assign(&mut self, rhs: &$name) {
                *self = *self * *rhs;
            }
        }
        impl Sum for $name {
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                iter.fold(Self::ZERO, Add::add)
            }
        }
        impl<'a> Sum<&'a $name> for $name {
            fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
                iter.copied().sum()
            }
        }
        impl Product for $name {
            fn product<I: Iterator<Item = Self>>(iter: I) -> Self {
                iter.fold(Self::ONE, Mul::mul)
            }
        }
        impl<'a> Product<&'a $name> for $name {
            fn product<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
                iter.copied().product()
            }
        }

        impl Field for $name {
            const ZERO: Self = Self([0u8; 32]);
            const ONE: Self = Self({
                let mut b = [0u8; 32];
                b[0] = 1;
                b
            });

            fn try_random<R: TryRng + ?Sized>(rng: &mut R) -> Result<Self, R::Error> {
                loop {
                    let mut buf = [0u8; 32];
                    rng.try_fill_bytes(&mut buf)?;
                    if BigUint::from_bytes_le(&buf) < Self::modulus() {
                        return Ok(Self(buf));
                    }
                }
            }

            fn square(&self) -> Self {
                *self * *self
            }

            fn double(&self) -> Self {
                *self + *self
            }

            fn invert(&self) -> CtOption<Self> {
                if self.big().is_zero() {
                    CtOption::new(Self::ZERO, Choice::from(0))
                } else {
                    let inv = self.big().modpow(&(Self::modulus() - 2u32), &Self::modulus());
                    CtOption::new(Self::from_big(inv), Choice::from(1))
                }
            }

            fn sqrt_ratio(num: &Self, div: &Self) -> (Choice, Self) {
                if div.big().is_zero() {
                    return (Choice::from(num.big().is_zero() as u8), Self::ZERO);
                }
                let v = *num * div.invert().unwrap();
                let m = Self::modulus();
                let exp = (&m + 1u32) >> 2;
                // Correct for S=1. For larger S, fall back to the same exp when
                // the result squares back to v; otherwise report a non-square.
                let root = Self::from_big(v.big().modpow(&exp, &m));
                if (root * root).big() == v.big() {
                    (Choice::from(1), root)
                } else {
                    (Choice::from(0), Self::ZERO)
                }
            }
        }

        impl PrimeField for $name {
            type Repr = [u8; 32];

            fn from_repr(repr: Self::Repr) -> CtOption<Self> {
                let n = BigUint::from_bytes_le(&repr);
                if n >= Self::modulus() {
                    CtOption::new(Self::ZERO, Choice::from(0))
                } else {
                    CtOption::new(Self(repr), Choice::from(1))
                }
            }

            fn to_repr(&self) -> Self::Repr {
                self.0
            }

            fn is_odd(&self) -> Choice {
                Choice::from(self.0[0] & 1)
            }

            const MODULUS: &'static str = $hex;
            const NUM_BITS: u32 = 256;
            const CAPACITY: u32 = 255;
            const TWO_INV: Self = Self($two);
            const MULTIPLICATIVE_GENERATOR: Self = Self($gen);
            const S: u32 = $s;
            const ROOT_OF_UNITY: Self = Self($root);
            const ROOT_OF_UNITY_INV: Self = Self($root_inv);
            const DELTA: Self = Self($delta);
        }
    };
}

secp_field!(
    SecpFp,
    "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F",
    1,
    [
        0x18, 0xfe, 0xff, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
    [
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0x2e, 0xfc, 0xff, 0xff, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff,
    ],
    [
        0x2e, 0xfc, 0xff, 0xff, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff,
    ],
    [
        0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ]
);

secp_field!(
    SecpFq,
    "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
    6,
    [
        0xa1, 0x20, 0x1b, 0x68, 0x46, 0x2f, 0xe9, 0xdf, 0x1d, 0x50, 0xa4, 0x57, 0x73, 0x6e, 0x57,
        0x5d, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
    [
        0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0x32, 0xd2, 0x2a, 0x2d, 0x3d, 0xf8, 0xb4, 0x15, 0xfa, 0x51, 0xe4, 0xbb, 0xbe, 0x9a, 0x6e,
        0xb9, 0xcb, 0x10, 0x57, 0x06, 0x0e, 0x81, 0xdc, 0xa7, 0xac, 0xd1, 0xdc, 0x98, 0xab, 0x8e,
        0x1f, 0x0d,
    ],
    [
        0x8d, 0x88, 0x49, 0x4d, 0xd5, 0xe4, 0x5d, 0x39, 0x3e, 0x1f, 0x70, 0x1c, 0xf2, 0xc6, 0x43,
        0xc0, 0xdd, 0xb4, 0xc7, 0x1a, 0xeb, 0x88, 0xb6, 0xde, 0x3c, 0x4c, 0xb3, 0x0d, 0xee, 0x58,
        0xfd, 0xa8,
    ],
    [
        0x01, 0x1f, 0x6a, 0xbf, 0x64, 0xed, 0x38, 0x6e, 0xed, 0x97, 0xa7, 0xda, 0xf4, 0xf9, 0x3f,
        0xe9, 0x03, 0x4f, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ]
);

#[allow(dead_code)]
fn _moduli_match_hex() {
    let _ = (P_HEX, N_HEX);
}
