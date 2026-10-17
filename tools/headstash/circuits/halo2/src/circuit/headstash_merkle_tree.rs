//! **Sinsemilla-legacy** leaf hash with canonicity constraints.
//!
//! # Migration
//!
//! New Headstash **public inclusion** paths use Poseidon-v1
//! ([`crate::circuit::distro_poseidon_gadget`]). This module remains for recovery /
//! historical tooling and for comparison tests. Do not wire new claim circuits here.

use core::iter;

use group::ff::PrimeField;
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, Value},
    plonk::{Advice, Column, ConstraintSystem, Constraints, Error, Expression, Selector},
    poly::Rotation,
};
use pasta_curves::pallas;

use crate::{
    circuit::gadget::secp256k1_chip::CrtInteger,
    constants::{OrchardCommitDomains, OrchardFixedBases, OrchardHashDomains, T_P},
    value::NoteValue,
};
use halo2_gadgets::{
    ecc::{chip::EccChip, NonIdentityPoint},
    sinsemilla::{chip::SinsemillaChip, HashDomain, Message, MessagePiece},
    utilities::{
        bool_check,
        lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig},
        RangeConstrained,
    },
};

/// Derive the leaf hash for a note in the merkle tree with sinsemilla and canonicity constraints.
pub fn derive_leaf(
    lo: impl Layouter<pallas::Base>,
    sc: &SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
    ecc_chip: &EccChip<OrchardFixedBases>,
    leaf_hash_chip: &LeafHashChip,
    epk: (CrtInteger<pallas::Base>, CrtInteger<pallas::Base>),
    fdi: AssignedCell<pallas::Base, pallas::Base>,
    v: AssignedCell<NoteValue, pallas::Base>,
    nd: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<NonIdentityPoint<pallas::Affine, EccChip<OrchardFixedBases>>, Error> {
    gadgets::hash_leaf(
        sc.clone(),
        ecc_chip.clone(),
        leaf_hash_chip.clone(),
        lo,
        epk,
        fdi,
        v,
        nd,
    )
}

/// LeafHashConfig
#[derive(Clone, Debug)]
pub struct LeafHashConfig {
    /// q_leaf_hash
    pub q_leaf_hash: Selector,
    /// advices
    pub advices: [Column<Advice>; 10],
}

/// LeafHashChip
#[derive(Clone, Debug)]
pub struct LeafHashChip {
    /// configuration params
    pub config: LeafHashConfig,
}

impl LeafHashChip {
    /// configure
    pub fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        advices: [Column<Advice>; 10],
    ) -> LeafHashConfig {
        let q_leaf_hash = meta.selector();

        let config = LeafHashConfig {
            q_leaf_hash,
            advices,
        };

        // Gate for constraining decompositions and canonicity of leaf hash inputs.
        meta.create_gate("LeafHash canonicity check", |meta| {
            let q_leaf_hash = meta.query_selector(config.q_leaf_hash);

            // let two_pow_5 = pallas::Base::from(1 << 5);
            // let two_pow_64 = pallas::Base::from_u128(1u128 << 64);

            let epk_x = meta.query_advice(config.advices[0], Rotation::cur());
            let nd = meta.query_advice(config.advices[2], Rotation::cur());
            let v = meta.query_advice(config.advices[3], Rotation::cur());
            let fdi = meta.query_advice(config.advices[4], Rotation::cur());

            let b0 = meta.query_advice(config.advices[5], Rotation::cur());
            let z13_a = meta.query_advice(config.advices[6], Rotation::cur());
            let a_prime = meta.query_advice(config.advices[7], Rotation::cur());
            let z13_a_prime = meta.query_advice(config.advices[8], Rotation::cur());

            let two_pow_250 = pallas::Base::from_u128(1u128 << 125).square();
            let epk_x_decomposition = {
                // y parity byte
                let a = meta.query_advice(config.advices[1], Rotation::cur());
                a.clone() + b0.clone() * Expression::Constant(two_pow_250) - epk_x.clone()
            };

            let a_prime_check = {
                let a = meta.query_advice(config.advices[1], Rotation::cur());
                let two_pow_250_expr = Expression::Constant(two_pow_250);
                let t_p = Expression::Constant(pallas::Base::from_u128(T_P));
                a + two_pow_250_expr - t_p - a_prime.clone()
            };

            let d0 = meta.query_advice(config.advices[5], Rotation::next());
            let z13_d = meta.query_advice(config.advices[6], Rotation::next());
            let nd_prime = meta.query_advice(config.advices[7], Rotation::next());
            let z13_nd_prime = meta.query_advice(config.advices[8], Rotation::next());

            let two_pow_4 = pallas::Base::from(1 << 4);
            let two_pow_254 = two_pow_250 * pallas::Base::from(1 << 4);
            let nd_decomposition = {
                let c = meta.query_advice(config.advices[1], Rotation::next());
                let b2 = meta.query_advice(config.advices[9], Rotation::cur());
                b2 + c.clone() * Expression::Constant(two_pow_4)
                    + d0.clone() * Expression::Constant(two_pow_254)
                    - nd.clone()
            };

            // nd_prime_check: uses piece `c` (250 bits = nd[4..254)) with 2^250
            // This is consistent with the witness computation in nd_canonicity()
            let nd_prime_check = {
                let c = meta.query_advice(config.advices[1], Rotation::next());
                let t_p = Expression::Constant(pallas::Base::from_u128(T_P));
                c + Expression::Constant(two_pow_250) - t_p - nd_prime.clone()
            };

            // Only d0 gets a bool_check because it's a 1-bit value (nd[254..255))
            // b0 is NOT checked as boolean because it's 5 bits (epk.x[250..255))
            let d0_bool = bool_check(d0);

            Constraints::with_selector(
                q_leaf_hash,
                iter::empty()
                    .chain(Some(("epk_x_decomposition", epk_x_decomposition)))
                    .chain(Some(("a_prime_check", a_prime_check)))
                    .chain(Some(("nd_decomposition", nd_decomposition)))
                    .chain(Some(("nd_prime_check", nd_prime_check)))
                    .chain(Some(("d0_bool", d0_bool))),
            )
        });

        config
    }

    /// Construct a LeafHashChip from its configuration.
    pub fn construct(config: LeafHashConfig) -> Self {
        Self { config }
    }
}

/// Gadgets for genesis merkle tree leaf hash computation.
///
/// Re-exported for use in test circuits.
pub mod gadgets {
    use halo2_gadgets::ecc::NonIdentityPoint;
    use halo2_gadgets::utilities::{FieldValue, RangeConstrained};
    use halo2_proofs::circuit::Chip;

    use super::*;

    /// Hash a leaf in the merkle tree with sinsemilla and constrain canonicity.
    ///
    /// This function implements the spec-compliant 8-piece message decomposition
    /// with proper canonicity constraints for the genesis merkle tree.
    #[allow(non_snake_case)]
    pub fn hash_leaf(
        sc: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        ecc_chip: EccChip<OrchardFixedBases>,
        leaf_hash_chip: LeafHashChip,
        mut lo: impl Layouter<pallas::Base>,
        epk: (CrtInteger<pallas::Base>, CrtInteger<pallas::Base>),
        fdi: AssignedCell<pallas::Base, pallas::Base>,
        v: AssignedCell<NoteValue, pallas::Base>,
        nd: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<NonIdentityPoint<pallas::Affine, EccChip<OrchardFixedBases>>, Error> {
        let lc = sc.config().lookup_config();
        let domain = OrchardHashDomains::Leaf;

        let vv = v.value().map(|v| pallas::Base::from(v.inner()));

        // Piece a: bits 0-249 of epk.x (250 bits)
        let a = MessagePiece::from_subpieces(
            sc.clone(),
            lo.namespace(|| "a: epk.x[0..250)"),
            [RangeConstrained::bitrange_of(epk.0.native.value(), 0..250)],
        )?;

        // Piece b: epk.x[250..255] || epk.y[0] || nd[0..4] (10 bits)
        let (b_0, b_1, b_2, b) = {
            let b_0 = RangeConstrained::witness_short(
                &lc,
                lo.namespace(|| "b_0: epk.x[250..255)"),
                epk.0.native.value(),
                250..255,
            )?;
            let b_1 = RangeConstrained::bitrange_of(epk.1.native.value(), 0..1);
            let b_2 = RangeConstrained::bitrange_of(nd.value(), 0..4);

            let b = MessagePiece::from_subpieces(
                sc.clone(),
                lo.namespace(|| "b: epk.x[250..255) || epk.y[0] || nd[0..4)"),
                [b_0.value(), b_1, b_2],
            )?;

            (b_0, b_1, b_2, b)
        };

        // Piece c: bits 4..254 of nd (250 bits)
        let c = MessagePiece::from_subpieces(
            sc.clone(),
            lo.namespace(|| "c: nd[4..254)"),
            [RangeConstrained::bitrange_of(nd.value(), 4..254)],
        )?;

        // Piece d: nd[254..255] || v[0..9] (10 bits)
        let ([d_0, d_1], d) = {
            let d0 = RangeConstrained::bitrange_of(nd.value(), 254..255);
            let d1 = RangeConstrained::bitrange_of(vv.value(), 0..9);
            (
                [d0, d1],
                MessagePiece::from_subpieces(
                    sc.clone(),
                    lo.namespace(|| "d: nd[254] || v[0..9)"),
                    [d0, d1],
                )?,
            )
        };

        // Piece e: v[9..59] (50 bits)
        let e = MessagePiece::from_subpieces(
            sc.clone(),
            lo.namespace(|| "e: v[9..59)"),
            [RangeConstrained::bitrange_of(vv.value(), 9..59)],
        )?;

        // Piece f: v[59..64] || fdi[0..5] (10 bits)
        let ([f_0, f_1], f) = {
            let f0 = RangeConstrained::bitrange_of(vv.value(), 59..64);
            let f1 = RangeConstrained::bitrange_of(fdi.value(), 0..5);
            (
                [f0, f1],
                MessagePiece::from_subpieces(
                    sc.clone(),
                    lo.namespace(|| "f: v[59..64) || fdi[0..5)"),
                    [f0, f1],
                )?,
            )
        };

        // Piece g: fdi[5..55] (50 bits)
        let g = MessagePiece::from_subpieces(
            sc.clone(),
            lo.namespace(|| "g: fdi[5..55)"),
            [RangeConstrained::bitrange_of(fdi.value(), 5..55)],
        )?;

        // Piece h: fdi[55..64] || padding (10 bits)
        let ([h_0, h_1], h) = {
            let h0 = RangeConstrained::bitrange_of(fdi.value(), 55..64);
            let h1 = RangeConstrained::bitrange_of(Value::known(&pallas::Base::zero()), 0..1);
            (
                [h0, h1],
                MessagePiece::from_subpieces(
                    sc.clone(),
                    lo.namespace(|| "h: fdi[55..64) || padding"),
                    [h0, h1],
                )?,
            )
        };

        let message =
            Message::from_pieces(sc.clone(), vec![a.clone(), b, c.clone(), d, e, f, g, h]);

        let (hash, zs) = {
            let domain_inst = HashDomain::new(sc, ecc_chip, &domain);
            domain_inst.hash_to_point(lo.namespace(|| "Hash leaf"), message)?
        };

        let z13_a = zs[0][13].clone();
        let z13_c = zs[2][13].clone();

        let (a_prime, z13_a_prime) = epk_x_canonicity(
            &lc,
            lo.namespace(|| "epk.x canonicity"),
            a.inner().cell_value(),
        )?;

        let (nd_prime, z13_nd_prime) = nd_canonicity(
            &lc,
            lo.namespace(|| "nd canonicity"),
            c.inner().cell_value(),
        )?;

        let gate_cells = GateCells {
            a: a.inner().cell_value(),
            c: c.inner().cell_value(),
            epk_x: epk.0.native.clone(),
            epk_y: epk.1.native.clone(),
            nd: nd.clone(),
            v,
            fdi: fdi.clone(),
            b_0,
            b_1,
            b_2,
            d_0,
            d_1,
            f_0,
            f_1,
            h_0,
            h_1,
            z13_a,
            z13_c,
            a_prime,
            z13_a_prime,
            nd_prime,
            z13_nd_prime,
        };

        leaf_hash_chip.config.assign_gate(
            lo.namespace(|| "Assign cells for leaf hash canonicity gate"),
            gate_cells,
        )?;

        Ok(hash)
    }

    fn epk_x_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        a: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            AssignedCell<pallas::Base, pallas::Base>,
            AssignedCell<pallas::Base, pallas::Base>,
        ),
        Error,
    > {
        let a_prime = {
            let two_pow_250 = Value::known(pallas::Base::from_u128(1u128 << 125).square());
            let t_p = Value::known(pallas::Base::from_u128(T_P));
            a.value() + two_pow_250 - t_p
        };
        let zs = lc.witness_check(
            lo.namespace(|| "Decompose low 250 bits of (a + 2^250 - t_P)"),
            a_prime,
            25,
            false,
        )?;
        let a_prime = zs[0].clone();
        assert_eq!(zs.len(), 26);

        Ok((a_prime, zs[25].clone()))
    }

    fn nd_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        c: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            AssignedCell<pallas::Base, pallas::Base>,
            AssignedCell<pallas::Base, pallas::Base>,
        ),
        Error,
    > {
        // c is the 250-bit piece nd[4..254), so we use 2^250 for canonicity
        let nd_prime = {
            let two_pow_250 = Value::known(pallas::Base::from_u128(1u128 << 125).square());
            let t_p = Value::known(pallas::Base::from_u128(T_P));
            c.value() + two_pow_250 - t_p
        };
        let zs = lc.witness_check(
            lo.namespace(|| "Decompose low 250 bits of (c + 2^250 - t_P)"),
            nd_prime,
            25,
            false,
        )?;
        let nd_prime = zs[0].clone();
        assert_eq!(zs.len(), 26);

        Ok((nd_prime, zs[25].clone()))
    }
}

impl LeafHashConfig {
    fn assign_gate(
        &self,
        mut lo: impl Layouter<pallas::Base>,
        gate_cells: GateCells,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "Assign cells for leaf hash canonicity gate",
            |mut region| {
                self.q_leaf_hash.enable(&mut region, 0)?;

                {
                    let offset = 0;

                    gate_cells.epk_x.copy_advice(
                        || "epk_x",
                        &mut region,
                        self.advices[0],
                        offset,
                    )?;

                    gate_cells
                        .a
                        .copy_advice(|| "a", &mut region, self.advices[1], offset)?;

                    gate_cells
                        .nd
                        .copy_advice(|| "nd", &mut region, self.advices[2], offset)?;

                    gate_cells
                        .v
                        .copy_advice(|| "v", &mut region, self.advices[3], offset)?;

                    gate_cells
                        .fdi
                        .copy_advice(|| "fdi", &mut region, self.advices[4], offset)?;

                    gate_cells.b_0.inner().copy_advice(
                        || "b_0",
                        &mut region,
                        self.advices[5],
                        offset,
                    )?;

                    gate_cells.z13_a.copy_advice(
                        || "z13_a",
                        &mut region,
                        self.advices[6],
                        offset,
                    )?;

                    gate_cells.a_prime.copy_advice(
                        || "a_prime",
                        &mut region,
                        self.advices[7],
                        offset,
                    )?;

                    gate_cells.z13_a_prime.copy_advice(
                        || "z13_a_prime",
                        &mut region,
                        self.advices[8],
                        offset,
                    )?;

                    region.assign_advice(
                        || "b_2",
                        self.advices[9],
                        offset,
                        || *gate_cells.b_2.inner(),
                    )?;
                }

                {
                    let offset = 1;

                    gate_cells
                        .c
                        .copy_advice(|| "c", &mut region, self.advices[1], offset)?;

                    region.assign_advice(
                        || "d_0",
                        self.advices[5],
                        offset,
                        || *gate_cells.d_0.inner(),
                    )?;

                    gate_cells.z13_c.copy_advice(
                        || "z13_c",
                        &mut region,
                        self.advices[6],
                        offset,
                    )?;

                    gate_cells.nd_prime.copy_advice(
                        || "nd_prime",
                        &mut region,
                        self.advices[7],
                        offset,
                    )?;

                    gate_cells.z13_nd_prime.copy_advice(
                        || "z13_nd_prime",
                        &mut region,
                        self.advices[8],
                        offset,
                    )?;
                }

                Ok(())
            },
        )
    }
}

struct GateCells {
    a: AssignedCell<pallas::Base, pallas::Base>,
    c: AssignedCell<pallas::Base, pallas::Base>,
    epk_x: AssignedCell<pallas::Base, pallas::Base>,
    epk_y: AssignedCell<pallas::Base, pallas::Base>,
    nd: AssignedCell<pallas::Base, pallas::Base>,
    v: AssignedCell<NoteValue, pallas::Base>,
    fdi: AssignedCell<pallas::Base, pallas::Base>,
    b_0: RangeConstrained<pallas::Base, AssignedCell<pallas::Base, pallas::Base>>,
    b_1: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    b_2: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    d_0: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    d_1: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    f_0: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    f_1: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    h_0: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    h_1: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    z13_a: AssignedCell<pallas::Base, pallas::Base>,
    z13_c: AssignedCell<pallas::Base, pallas::Base>,
    a_prime: AssignedCell<pallas::Base, pallas::Base>,
    z13_a_prime: AssignedCell<pallas::Base, pallas::Base>,
    nd_prime: AssignedCell<pallas::Base, pallas::Base>,
    z13_nd_prime: AssignedCell<pallas::Base, pallas::Base>,
}
