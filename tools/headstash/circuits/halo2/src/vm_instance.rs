//! Eight public scalars the wasmvm accepts for this circuit.
//!
//! `VestaInstance::new_from_vm` requires a multiple of 32 bytes. Each limb is
//! `pallas::Base::to_repr` (little-endian). Row order matches
//! `Instance::to_halo2_instance` indices 0..8: anchor, denom, value,
//! recipient, nullifier, cmx, challenge low 128, challenge high 128.
//! The 200-byte `to_bytes` wire stays the personal_sign prefix.

use alloc::vec::Vec;

use ff::PrimeField;
use pasta_curves::pallas;

pub fn encode_vm_public_inputs(
    anchor: pallas::Base,
    nd: pallas::Base,
    value: u64,
    recp: pallas::Base,
    nf: pallas::Base,
    cmx: pallas::Base,
    e: [u8; 32],
) -> Vec<u8> {
    let e_lo = u128::from_be_bytes(e[16..32].try_into().expect("e lo"));
    let e_hi = u128::from_be_bytes(e[0..16].try_into().expect("e hi"));
    let scalars = [
        anchor,
        nd,
        pallas::Base::from(value),
        recp,
        nf,
        cmx,
        pallas::Base::from_u128(e_lo),
        pallas::Base::from_u128(e_hi),
    ];
    let mut out = Vec::with_capacity(8 * 32);
    for s in scalars {
        out.extend_from_slice(s.to_repr().as_ref());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_scalars_are_256_bytes() {
        let mut e = [0u8; 32];
        e[31] = 9;
        let out = encode_vm_public_inputs(
            pallas::Base::from(1u64),
            pallas::Base::from(2u64),
            5_000_000,
            pallas::Base::from(4u64),
            pallas::Base::from(5u64),
            pallas::Base::from(6u64),
            e,
        );
        assert_eq!(out.len(), 256);
        assert_eq!(&out[64..72], &5_000_000u64.to_le_bytes());
        assert!(out[72..96].iter().all(|b| *b == 0));
        let mut lo = [0u8; 32];
        lo[0] = 9;
        assert_eq!(&out[192..224], &lo);
        assert!(out[224..256].iter().all(|b| *b == 0));
    }
}
