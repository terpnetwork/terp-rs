// each piece has bit length that is of multiple of 10.
// if message is built from > 1 messagePieces, each piece must be < 64 bits
// we hav n pieces of data ordered in ∫ sequence to create note-commit.
// l is length of total sum of bits in n
// we keep track of each n bounds coordiates (start,end) in set of ∫
// generate circuits to automatically of decompose sections based on unique ∫n

use core::iter;
// use std::println;

use group::ff::PrimeField;
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, Value},
    plonk::{Advice, Column, ConstraintSystem, Constraints, Error, Expression, Selector},
    poly::Rotation,
};
use pasta_curves::pallas;

use crate::{
    constants::{OrchardCommitDomains, OrchardFixedBases, OrchardHashDomains, T_P},
    value::NoteValue,
};
use halo2_gadgets::{
    ecc::{chip::EccChip, CircuitVersion, Point, ScalarFixed},
    sinsemilla::{
        chip::{SinsemillaChip, SinsemillaConfig},
        CommitDomain, Message, MessagePiece,
    },
    utilities::{
        lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig},
        FieldValue, RangeConstrained,
    },
};

type NoteCommitPiece = MessagePiece<
    pallas::Affine,
    SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
    10,
    253,
>;

/// The vs of the running sum at the start and end of the range being used for a
/// canonicity check.
type CanonicityBounds = (
    AssignedCell<pallas::Base, pallas::Base>,
    AssignedCell<pallas::Base, pallas::Base>,
);

// Piece b: bits 250..255 of nd || 0..55 of v  (5 + 55 = 60 bits)
///
/// | A_6 | A_7 | A_8 | q_notecommit_b |
/// ------------------------------------
/// |  b  | b0  | b1  |       1        |
///
#[derive(Clone, Debug)]
struct DecomposeB {
    q_notecommit_b: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
}

impl DecomposeB {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        two_pow_5: pallas::Base,
    ) -> Self {
        let q_notecommit_b = meta.selector();

        meta.create_gate("NoteCommit MessagePiece b", |meta| {
            let q_notecommit_b = meta.query_selector(q_notecommit_b);

            // b has been constrained to 60 bits by the Sinsemilla hash.
            let b = meta.query_advice(col_l, Rotation::cur());
            // b0 has been constrained to be 5 bits outside this gate.
            let b0 = meta.query_advice(col_m, Rotation::cur());
            // This gate constrains to be 55 bits outside this gate.
            let b1 = meta.query_advice(col_r, Rotation::cur());

            // b = b0 + (2^1) b1 + (2^5)
            let decomposition_check = b - (b0 + b1.clone() * two_pow_5);

            Constraints::with_selector(q_notecommit_b, [("decomposition", decomposition_check)])
        });

        Self {
            q_notecommit_b,
            col_l,
            col_m,
            col_r,
        }
    }

    #[allow(clippy::type_complexity)]
    fn decompose(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        lo: &mut impl Layouter<pallas::Base>,
        nd: &AssignedCell<pallas::Base, pallas::Base>,
        v: &AssignedCell<NoteValue, pallas::Base>,
    ) -> Result<
        (
            NoteCommitPiece,                                                          // b
            RangeConstrained<pallas::Base, AssignedCell<pallas::Base, pallas::Base>>, // b0
            RangeConstrained<pallas::Base, Value<pallas::Base>>,                      // b1
        ),
        Error,
    > {
        let value_val = v.value().map(|v| pallas::Base::from(v.inner()));

        // Constrain b_0 to be 5 bits
        let b0 = RangeConstrained::witness_short(lc, lo.namespace(|| "b_0"), nd.value(), 250..255)?;
        let b1 = RangeConstrained::bitrange_of(value_val.value(), 0..55); // 55 v
                                                                          // println!("b: {:#?}", (b0.num_bits(), b1.num_bits()));
        let b = MessagePiece::from_subpieces(
            chip.clone(),
            lo.namespace(|| "piece_b: nd[250..255) || v[0..55)"),
            [b0.value(), b1],
        )?;
        Ok((b, b0, b1))
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        b: NoteCommitPiece,
        b0: RangeConstrained<pallas::Base, AssignedCell<pallas::Base, pallas::Base>>,
        b1: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    ) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
        lo.assign_region(
            || "NoteCommit MessagePiece b",
            |mut region| {
                self.q_notecommit_b.enable(&mut region, 0)?;
                // Assign the full 60-bit value b
                b.inner()
                    .cell_value()
                    .copy_advice(|| "b", &mut region, self.col_l, 0)?;
                // Assign b0 (5 bits from nd[250..255))
                b0.inner()
                    .copy_advice(|| "b0", &mut region, self.col_m, 0)?;
                // Assign b1 (55 bits from v[0..55))
                let b1 = region.assign_advice(|| "b1", self.col_r, 0, || *b1.inner())?;

                Ok(b1)
            },
        )
    }
}

#[derive(Clone, Debug)]
struct DecomposeC {
    q_notecommit_c: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
}

impl DecomposeC {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        two_pow_9: pallas::Base,
    ) -> Self {
        let q_notecommit_c = meta.selector();

        meta.create_gate("NoteCommit MessagePiece c: configure", |meta| {
            let q_notecommit_c = meta.query_selector(q_notecommit_c);
            // Piece c: bits 55-64 of v || 0-51 of fdi  (9 + 51 = 60 bits)
            let c = meta.query_advice(col_l, Rotation::cur());
            // c0 has been constrained to be 9 bits outside this gate.
            let c0 = meta.query_advice(col_m, Rotation::cur());
            // c1 has been constrained to be 51 bits outside this gate.
            let c1 = meta.query_advice(col_r, Rotation::cur());

            // Piece c is the bit-concatenation of c0 (9 bits = v[55..64)) and
            // c1 (51 bits = fdi[0..51)): c = c0 + c1 * 2^9.
            // c1 is a multi-bit limb (range-checked via MessagePiece packing),
            // NOT a boolean — do not bool_check it (that broke suite leaves with fdi > 1).
            let decomposition_check = c - (c0 + c1 * two_pow_9);

            Constraints::with_selector(
                q_notecommit_c,
                [("decomposition", decomposition_check)],
            )
        });

        Self {
            q_notecommit_c,
            col_l,
            col_m,
            col_r,
        }
    }

    #[allow(clippy::type_complexity)]
    fn decompose(
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        lo: &mut impl Layouter<pallas::Base>,
        v: &AssignedCell<NoteValue, pallas::Base>,
        fdi: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            NoteCommitPiece,
            RangeConstrained<pallas::Base, Value<pallas::Base>>, // c0
            RangeConstrained<pallas::Base, Value<pallas::Base>>, // c1
        ),
        Error,
    > {
        let value_val = v.value().map(|v| pallas::Base::from(v.inner()));
        // Piece c: bits 55-64 of v || 0-51 of fdi  (9 + 51 = 60 bits)
        let (c, c0, c1) = {
            let c0 = RangeConstrained::bitrange_of(value_val.value(), 55..64); // 9 v
            let c1 = RangeConstrained::bitrange_of(fdi.value(), 0..51); // 51 fdi
                                                                        // println!("c: {:#?}", (c0.num_bits(), c1.num_bits()));
            (
                MessagePiece::from_subpieces(
                    chip.clone(),
                    lo.namespace(|| "piece_c: v[55..64) || pad(0)"),
                    [c0, c1],
                )?,
                c0,
                c1,
            )
        };

        Ok((c, c0, c1))
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        c: NoteCommitPiece,
        c0: RangeConstrained<pallas::Base, Value<pasta_curves::Fp>>,
        c1: RangeConstrained<pallas::Base, Value<pallas::Base>>,
    ) -> Result<[AssignedCell<pallas::Base, pallas::Base>; 2], Error> {
        lo.assign_region(
            || "NoteCommit MessagePiece c: assign",
            |mut region| {
                self.q_notecommit_c.enable(&mut region, 0)?;

                c.inner()
                    .cell_value()
                    .copy_advice(|| "c", &mut region, self.col_l, 0)?;
                let c0 = region.assign_advice(|| "c", self.col_m, 0, || *c0.inner())?;
                let c1 = region.assign_advice(|| "c1", self.col_r, 0, || *c1.inner())?;

                Ok([c0, c1])
            },
        )
    }
}

// Piece d: bits  51-64 of fdi || 0..7 of recp (13 + 7 = 20 bits)
///   For the gate, we decompose a 10-bit boundary: d0 || d1
///
/// | A_6 | A_7 | A_8 | q_notecommit_d |
/// ------------------------------------
/// |  d  | d0 | d1 |       1        |
/// |     | d2 | d_3 |       0        |
#[derive(Clone, Debug)]
struct DecomposeD {
    q_notecommit_d: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
}

impl DecomposeD {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        two_pow_13: pallas::Base,
    ) -> Self {
        let q_notecommit_d = meta.selector();

        meta.create_gate("NoteCommit MessagePiece d", |meta| {
            let q_notecommit_d = meta.query_selector(q_notecommit_d);

            // d has been constrained to 20 bits by the Sinsemilla hash.
            let d = meta.query_advice(col_l, Rotation::cur());
            // d0 has been constrained to 13 bits  of fdi.
            let d0 = meta.query_advice(col_m, Rotation::cur());
            // d1 has been constrained to 7 bits  of recp.
            let d1 = meta.query_advice(col_r, Rotation::cur());
            // d = d0 + 2^13 * d1
            let decomposition_check = d - (d0 + d1 * two_pow_13);

            Constraints::with_selector(q_notecommit_d, [("decomposition", decomposition_check)])
        });

        Self {
            q_notecommit_d,
            col_l,
            col_m,
            col_r,
        }
    }

    #[allow(clippy::type_complexity)]
    fn decompose(
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        lo: &mut impl Layouter<pallas::Base>,
        fdi: &AssignedCell<pallas::Base, pallas::Base>,
        recp: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            NoteCommitPiece,
            RangeConstrained<pallas::Base, Value<pallas::Base>>,
            RangeConstrained<pallas::Base, Value<pallas::Base>>,
        ),
        Error,
    > {
        // Piece d: bits  51-64 of fdi || 0..7 of recp (13 + 7 = 20 bits)
        let (d0, d1) = (
            RangeConstrained::bitrange_of(fdi.value(), 51..64), // 13 fdi
            RangeConstrained::bitrange_of(recp.value(), 0..7),  // 7 recp
        );
        // println!("d: {:#?}", (d0.num_bits(), d1.num_bits(),));
        let d = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "d"), [d0, d1])?;

        Ok((d, d0, d1))
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        d: NoteCommitPiece,
        d0: RangeConstrained<pallas::Base, Value<pallas::Base>>,
        d1: RangeConstrained<pallas::Base, Value<pallas::Base>>,
        z1_d: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<[AssignedCell<pallas::Base, pallas::Base>; 2], Error> {
        lo.assign_region(
            || "NoteCommit MessagePiece d",
            |mut region| {
                self.q_notecommit_d.enable(&mut region, 0)?;
                d.inner()
                    .cell_value()
                    .copy_advice(|| "d", &mut region, self.col_l, 0)?;
                let d0 = region.assign_advice(|| "d0", self.col_m, 0, || *d0.inner())?;
                let d1 = region.assign_advice(|| "d1", self.col_r, 0, || *d1.inner())?;
                z1_d.copy_advice(|| "d_3 = z1_d", &mut region, self.col_r, 1)?;

                Ok([d0, d1])
            },
        )
    }
}

#[derive(Clone, Debug)]
struct DecomposeF {
    q_notecommit_f: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
}

impl DecomposeF {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        two_pow_8: pallas::Base,
    ) -> Self {
        let q_notecommit_f = meta.selector();

        meta.create_gate("NoteCommit MessagePiece e", |meta| {
            let q_notecommit_f = meta.query_selector(q_notecommit_f);
            // f is the full 10-bit value
            let f = meta.query_advice(col_l, Rotation::cur());
            // f0: bits 247..255 of recp (8 bits)
            let f0 = meta.query_advice(col_m, Rotation::cur());
            // f1: bits 0..2 of esk (2 bits)
            let f1 = meta.query_advice(col_r, Rotation::cur());

            // f = f0 + 2^8 * f1
            let decomposition_check = f - (f0 + f1 * two_pow_8);

            Constraints::with_selector(q_notecommit_f, Some(("decomposition", decomposition_check)))
        });

        Self {
            q_notecommit_f,
            col_l,
            col_m,
            col_r,
        }
    }

    #[allow(clippy::type_complexity)]
    fn decompose(
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        lo: &mut impl Layouter<pallas::Base>,
        recp: &AssignedCell<pallas::Base, pallas::Base>,
        esk: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            NoteCommitPiece,
            [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2],
        ),
        Error,
    > {
        // Piece f: bits 247..255 of recp (8) || 0..2 of esk (8 + 2 = 10 bits)
        let (f0, f1) = (
            RangeConstrained::bitrange_of(recp.value(), 247..255), // 8
            RangeConstrained::bitrange_of(esk.value(), 0..2),      // 2
        );
        // println!("f: {:#?}", (f0.num_bits(), f1.num_bits()));

        let f = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "f"), [f0, f1])?;

        Ok((f, [f0, f1]))
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        f: NoteCommitPiece,
        f_ranges: [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2],
    ) -> Result<[AssignedCell<pallas::Base, pallas::Base>; 2], Error> {
        lo.assign_region(
            || "NoteCommit MessagePiece e",
            |mut region| {
                let [f0, f1] = f_ranges;
                // Enable selector on the row
                self.q_notecommit_f.enable(&mut region, 0)?;
                // Copy f to col_l
                f.inner()
                    .cell_value()
                    .copy_advice(|| "f", &mut region, self.col_l, 0)?;
                // Assign f0 and f1
                let f0_assigned = region.assign_advice(|| "f0", self.col_m, 0, || *f0.inner())?;
                let f1_assigned = region.assign_advice(|| "f1", self.col_r, 0, || *f1.inner())?;
                Ok([f0_assigned, f1_assigned])
            },
        )
    }
}

#[derive(Clone, Debug)]
struct DecomposeH {
    q_notecommit_h: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
}

impl DecomposeH {
    #[allow(clippy::too_many_arguments)]
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        two_pow_3: pallas::Base,
    ) -> Self {
        let q_notecommit_h = meta.selector();

        meta.create_gate("NoteCommit MessagePiece h", |meta| {
            let q_notecommit_h = meta.query_selector(q_notecommit_h);
            // h is the full 10-bit value
            let h = meta.query_advice(col_l, Rotation::cur());
            // h0: bits 252..255 of esk (3 bits)
            let h0 = meta.query_advice(col_m, Rotation::cur());
            // h1: bits 0..7 of rho (7 bits)
            let h1 = meta.query_advice(col_r, Rotation::cur());
            // h = h0 + 2^8 * h1
            let decomposition_check = h - (h0 + h1 * two_pow_3);
            Constraints::with_selector(
                q_notecommit_h,
                Some(("h decomposition", decomposition_check)),
            )
        });
        Self {
            q_notecommit_h,
            col_l,
            col_m,
            col_r,
        }
    }

    #[allow(clippy::type_complexity)]
    fn decompose(
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        lo: &mut impl Layouter<pallas::Base>,
        esk: &AssignedCell<pallas::Base, pallas::Base>,
        rho: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            NoteCommitPiece,
            [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2],
        ),
        Error,
    > {
        // Piece h: bits 252..255 of esk (3) || bits 0..7 of rho (7) (7+3 = 10 bits)
        let (h0, h1) = (
            RangeConstrained::bitrange_of(esk.value(), 252..255), //  3 bits
            RangeConstrained::bitrange_of(rho.value(), 0..7),     //  7 bits
        );

        let h = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "h"), [h0, h1])?;
        // println!("h: {:#?}", (h0.num_bits(), h1.num_bits()));

        Ok((h, [h0, h1]))
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        h: NoteCommitPiece,
        h_ranges: [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2], // h0,h1
    ) -> Result<[AssignedCell<pallas::Base, pallas::Base>; 2], Error> {
        lo.assign_region(
            || "NoteCommit MessagePiece h",
            |mut region| {
                let [h0, h1] = h_ranges;
                // Enable selector on the row
                self.q_notecommit_h.enable(&mut region, 0)?;
                // Copy h to col_l
                h.inner()
                    .cell_value()
                    .copy_advice(|| "h", &mut region, self.col_l, 0)?;
                // Assign h0 and h1
                let h0_cell = region.assign_advice(|| "h0", self.col_m, 0, || *h0.inner())?;
                let h1_cell = region.assign_advice(|| "h1", self.col_r, 0, || *h1.inner())?;
                Ok([h0_cell, h1_cell])
            },
        )
    }
}

#[derive(Clone, Debug)]
struct DecomposeJ {
    q_notecommit_j: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
}

impl DecomposeJ {
    #[allow(clippy::too_many_arguments)]
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        two_pow_8: pallas::Base,
    ) -> Self {
        let q_notecommit_j = meta.selector();

        meta.create_gate("NoteCommit MessagePiece i", |meta| {
            let q_notecommit_j = meta.query_selector(q_notecommit_j);
            // j is the full 10-bit value
            let j = meta.query_advice(col_l, Rotation::cur());
            // j0: bits 247..255 of rho (8 bits)
            let j0 = meta.query_advice(col_m, Rotation::cur());
            // j1: bits  0..2 of psi (2 bits)
            let j1 = meta.query_advice(col_r, Rotation::cur());
            // j = j0 + 2^3 * j1
            let decomposition_check = j - (j0 + j1 * two_pow_8);
            Constraints::with_selector(
                q_notecommit_j,
                Some(("j decomposition", decomposition_check)),
            )
        });

        Self {
            q_notecommit_j,
            col_l,
            col_m,
            col_r,
        }
    }

    #[allow(clippy::type_complexity)]
    fn decompose(
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        lo: &mut impl Layouter<pallas::Base>,
        rho: &AssignedCell<pallas::Base, pallas::Base>,
        psi: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            NoteCommitPiece,
            [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2],
        ),
        Error,
    > {
        // Piece j: bits 247..255 of rho (8) || bits 0..2 of psi (8+2 = 10 bits)
        let (j0, j1) = (
            RangeConstrained::bitrange_of(rho.value(), 247..255),
            RangeConstrained::bitrange_of(psi.value(), 0..2),
        );
        // println!("j: {:#?}", (j0.num_bits(), j1.num_bits()));
        let j = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "j"), [j0, j1])?;

        Ok((j, [j0, j1]))
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        j: NoteCommitPiece,
        j_ranges: [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2], // [j0, j1]
    ) -> Result<[AssignedCell<pallas::Base, pallas::Base>; 2], Error> {
        lo.assign_region(
            || "NoteCommit MessagePiece i",
            |mut region| {
                let [j0, j1] = j_ranges;
                // Enable selector on the row
                self.q_notecommit_j.enable(&mut region, 0)?;
                // Copy j to col_l
                j.inner()
                    .cell_value()
                    .copy_advice(|| "j", &mut region, self.col_l, 0)?;
                // Assign j0 and j1
                let j0_cell = region.assign_advice(|| "j0", self.col_m, 0, || *j0.inner())?;
                let j1_cell = region.assign_advice(|| "j1", self.col_r, 0, || *j1.inner())?;
                Ok([j0_cell, j1_cell])
            },
        )
    }
}

#[derive(Clone, Debug)]
struct DecomposeL {
    q_notecommit_l: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
}

impl DecomposeL {
    #[allow(clippy::too_many_arguments)]
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        two_pow_3: pallas::Base,
    ) -> Self {
        let q_notecommit_l = meta.selector();

        meta.create_gate("NoteCommit MessagePiece l", |meta| {
            let q_notecommit_l = meta.query_selector(q_notecommit_l);
            // l is the full 10-bit value
            let l = meta.query_advice(col_l, Rotation::cur());
            // l0: bits 247..255 of psi (3 bits)
            let l0 = meta.query_advice(col_m, Rotation::cur());
            // l1: 7 bits of padding (7)
            let l1 = meta.query_advice(col_r, Rotation::cur());
            // l = l0 + 2^8 * l1
            let decomposition_check = l - (l0 + l1 * two_pow_3);
            Constraints::with_selector(
                q_notecommit_l,
                Some(("l decomposition", decomposition_check)),
            )
        });

        Self {
            q_notecommit_l,
            col_l,
            col_m,
            col_r,
        }
    }

    #[allow(clippy::type_complexity)]
    fn decompose(
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        lo: &mut impl Layouter<pallas::Base>,

        psi: &AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<
        (
            NoteCommitPiece,
            [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2],
        ),
        Error,
    > {
        // Piece l: bits 252..255 of psi (3 bits) || 7 bit padding (3+7 = 10 bits)
        let (l0, l1) = (
            RangeConstrained::bitrange_of(psi.value(), 252..255), //  3
            RangeConstrained::bitrange_of(Value::known(&pallas::Base::zero()), 0..7), // two bit padding
        );
        // println!("l: {:#?}", (l0.num_bits(), l1.num_bits()));
        let l = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "l"), [l0, l1])?;
        Ok((l, [l0, l1]))
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        l: NoteCommitPiece,
        l_ranges: [RangeConstrained<pallas::Base, Value<pallas::Base>>; 2], // [l0, l1]
    ) -> Result<[AssignedCell<pallas::Base, pallas::Base>; 2], Error> {
        lo.assign_region(
            || "NoteCommit MessagePiece i",
            |mut region| {
                let [l0, l1] = l_ranges;
                // Enable selector on the row
                self.q_notecommit_l.enable(&mut region, 0)?;
                // Copy l to col_l
                l.inner()
                    .cell_value()
                    .copy_advice(|| "l", &mut region, self.col_l, 0)?;
                // Assign l0 and l1
                let l0_cell = region.assign_advice(|| "l0", self.col_m, 0, || *l0.inner())?;
                let l1_cell = region.assign_advice(|| "l1", self.col_r, 0, || *l1.inner())?;
                Ok([l0_cell, l1_cell])
            },
        )
    }
}

/// |  A_6   | A_7 |   A_8   |     A_9     | q_notecommit_nd |
/// -----------------------------------------------------------
/// | nd     | b0  | a       | z13_a       |        1         |
/// |        |     | a_prime | z13_a_prime |        0         |
///
#[derive(Clone, Debug)]
struct NdCanonicity {
    q_notecommit_nd: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
    col_z: Column<Advice>,
}

impl NdCanonicity {
    #[allow(clippy::too_many_arguments)]
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        col_z: Column<Advice>,
        two_pow_250: pallas::Base,
        t_p: Expression<pallas::Base>,
    ) -> Self {
        let q_notecommit_nd = meta.selector();

        meta.create_gate("NoteCommit input nd", |meta| {
            let q_notecommit_nd = meta.query_selector(q_notecommit_nd);

            // nd is the full 255-bit value
            let nd = meta.query_advice(col_l, Rotation::cur());
            // b0 is bits 250-254 of nd (5 bits)
            let b0 = meta.query_advice(col_m, Rotation::cur());
            // a is bits 0-249 of nd (250 bits)
            let a = meta.query_advice(col_r, Rotation::cur());
            let a_prime = meta.query_advice(col_r, Rotation::next());
            let z13_a = meta.query_advice(col_z, Rotation::cur());
            let z13_a_prime = meta.query_advice(col_z, Rotation::next());
            // Decomposition: nd = a + b0 * 2^250
            let decomposition_check = a.clone() + b0.clone() * two_pow_250 - nd;

            // a_prime = a + 2^250 - t_P
            let two_pow_250_expr = Expression::Constant(two_pow_250);
            let a_prime_check = a + two_pow_250_expr - t_p - a_prime;

            // Note: The "high nd" canonicity constraints (z13_a = 0 when b0 >= 16) are not needed
            // because NoteDenom::new_for_proof() clears bits 253-255 via `bytes[31] &= 0x1F`,
            // ensuring b0 (bits 250-254) is always < 8. The field element is guaranteed canonical
            // by this bit-trimming, so no additional runtime canonicity check is required.

            Constraints::with_selector(
                q_notecommit_nd,
                iter::empty()
                    .chain(Some(("decomposition", decomposition_check)))
                    .chain(Some(("a_prime_check", a_prime_check))),
            )
        });

        Self {
            q_notecommit_nd,
            col_l,
            col_m,
            col_r,
            col_z,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        nd: &AssignedCell<pallas::Base, pallas::Base>,
        a: NoteCommitPiece,
        b0: RangeConstrained<pallas::Base, AssignedCell<pallas::Base, pallas::Base>>,
        b1: AssignedCell<pallas::Base, pallas::Base>,
        a_prime: AssignedCell<pallas::Base, pallas::Base>,
        z13_a: AssignedCell<pallas::Base, pallas::Base>,
        z13_a_prime: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "NoteCommit input nd",
            |mut region| {
                nd.copy_advice(|| "nd", &mut region, self.col_l, 0)?;

                b0.inner()
                    .copy_advice(|| "b0", &mut region, self.col_m, 0)?;

                a.inner()
                    .cell_value()
                    .copy_advice(|| "a", &mut region, self.col_r, 0)?;
                a_prime.copy_advice(|| "a_prime", &mut region, self.col_r, 1)?;

                z13_a.copy_advice(|| "z13_a", &mut region, self.col_z, 0)?;
                z13_a_prime.copy_advice(|| "z13_a_prime", &mut region, self.col_z, 1)?;

                self.q_notecommit_nd.enable(&mut region, 0)
            },
        )
    }
}

/// | A_6  | A_7 | A_8 | A_9      | q_notecommit_v |
/// -------------------------------------------------
/// | value| b1  | c0  | b1_c0'   |       1        |
/// |      |     |     | z6_b1_c0'|       0        |
#[derive(Clone, Debug)]
struct ValueCanonicity {
    q_notecommit_v: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
    col_z: Column<Advice>,
}

impl ValueCanonicity {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        col_z: Column<Advice>,
        two_pow_55: pallas::Base, // 2^55 for c0 (v[0..55), 55 bits)
        two_pow_64: pallas::Base,
        t_p: Expression<pallas::Base>,
    ) -> Self {
        let q_notecommit_v = meta.selector();

        meta.create_gate("NoteCommit input v", |meta| {
            let q_notecommit_v = meta.query_selector(q_notecommit_v);
            // full 64 bits of v
            let value = meta.query_advice(col_l, Rotation::cur());
            // b1 (bottom 55 bits of v, bits 0..55) - assigned as witness
            let b1 = meta.query_advice(col_m, Rotation::cur());
            // c0 (top 9 bits of v, bits 55..64) - constrained to 9 bits in DecomposeC
            let c0 = meta.query_advice(col_r, Rotation::cur());
            let b1_c0_prime = meta.query_advice(col_z, Rotation::cur());
            let z6_b1_c0_prime = meta.query_advice(col_z, Rotation::next());

            // value = b1 + 2^55 * c0
            let value_check = b1.clone() + c0.clone() * two_pow_55 - value;

            // Canonicity check: b1_c0_prime = b1 + c0 * 2^55 + 2^64 - t_P
            let b1_c0_prime_check =
                b1 + c0 * two_pow_55 + Expression::Constant(two_pow_64) - t_p - b1_c0_prime.clone();

            Constraints::with_selector(
                q_notecommit_v,
                [
                    ("value_check", value_check),
                    ("b1_c0_prime_check", b1_c0_prime_check),
                ],
            )
        });

        Self {
            q_notecommit_v,
            col_l,
            col_m,
            col_r,
            col_z,
        }
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        value: AssignedCell<NoteValue, pallas::Base>,
        b1: RangeConstrained<pallas::Base, Value<pallas::Base>>, // v[0..55) - 55 bits
        c0: AssignedCell<pallas::Base, pallas::Base>,
        b1_c0_prime: AssignedCell<pallas::Base, pallas::Base>,
        z6_b1_c0_prime: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "NoteCommit input value",
            |mut region| {
                self.q_notecommit_v.enable(&mut region, 0)?;

                // Row 0
                value.copy_advice(|| "value", &mut region, self.col_l, 0)?;
                region.assign_advice(|| "b1", self.col_m, 0, || *b1.inner())?;
                c0.copy_advice(|| "c0", &mut region, self.col_r, 0)?;
                b1_c0_prime.copy_advice(|| "b1_c0_prime", &mut region, self.col_z, 0)?;

                // Row 1
                z6_b1_c0_prime.copy_advice(|| "z6_b1_c0_prime", &mut region, self.col_z, 1)?;

                Ok(())
            },
        )
    }
}

/// | A_6  | A_7 | A_8 |  q_notecommit_fdi |
/// --------------------------------------------
/// | fdi  | c1  | d0  |        1         |
///
#[derive(Clone, Debug)]
struct FdiCanonicity {
    q_notecommit_fdi: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
    col_z: Column<Advice>,
}

impl FdiCanonicity {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        col_z: Column<Advice>,
        two_pow_51: pallas::Base,
        two_pow_64: pallas::Base,
        t_p: Expression<pallas::Base>,
    ) -> Self {
        let q_notecommit_fdi = meta.selector();

        meta.create_gate("NoteCommit input value", |meta| {
            let q_notecommit_fdi = meta.query_selector(q_notecommit_fdi);
            // Row 0
            let fdi = meta.query_advice(col_l, Rotation::cur());
            let c1 = meta.query_advice(col_m, Rotation::cur()); // bits 0..51 (51 bits)
            let d0 = meta.query_advice(col_r, Rotation::cur()); // bits 51..64 (13 bits)
            let c1_d0_prime = meta.query_advice(col_z, Rotation::cur());

            // Row 1
            let z6_c1_d0_prime = meta.query_advice(col_z, Rotation::next());

            // Decomposition check: fdi = c1 + d0 * 2^51
            let fdi_check = c1.clone() + d0.clone() * two_pow_51 - fdi;

            // Canonicity check: c1_d0_prime = c1 + d0 * 2^51 + 2^64 - t_P
            let c1_d0_prime_check =
                c1 + d0 * two_pow_51 + Expression::Constant(two_pow_64) - t_p - c1_d0_prime;

            Constraints::with_selector(
                q_notecommit_fdi,
                [
                    ("fdi_check", fdi_check),
                    ("c1_d0_prime_check", c1_d0_prime_check),
                ],
            )
        });

        Self {
            q_notecommit_fdi,
            col_l,
            col_m,
            col_r,
            col_z,
        }
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        fdi: AssignedCell<pallas::Base, pallas::Base>,
        c1: AssignedCell<pallas::Base, pallas::Base>,
        d0: AssignedCell<pallas::Base, pallas::Base>,
        c1_d0_prime: AssignedCell<pallas::Base, pallas::Base>,
        z6_c1_d0_prime: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "NoteCommit input fdi",
            |mut region| {
                self.q_notecommit_fdi.enable(&mut region, 0)?;

                // Row 0
                fdi.copy_advice(|| "fdi", &mut region, self.col_l, 0)?;
                c1.copy_advice(|| "c1 (bits 0..51)", &mut region, self.col_m, 0)?;
                d0.copy_advice(|| "d0 (bits 51..64)", &mut region, self.col_r, 0)?;
                c1_d0_prime.copy_advice(|| "c1_d0_prime", &mut region, self.col_z, 0)?;

                // Row 1
                z6_c1_d0_prime.copy_advice(|| "z6_c1_d0_prime", &mut region, self.col_z, 1)?;

                Ok(())
            },
        )
    }
}

/// | A_6  | A_7 | A_8 | A_9          | q_notecommit_recp |
/// ------------------------------------------------------
/// | recp | d1  |  e  | d1_e_f0'     |        1          |
/// |      | f0  |     | z26_d1_e_f0' |        0          |
///
#[derive(Clone, Debug)]
struct RecpCanonicity {
    q_notecommit_recp: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
    col_z: Column<Advice>,
}

impl RecpCanonicity {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        col_z: Column<Advice>,
        two_pow_7: pallas::Base,
        two_pow_247: pallas::Base,
        two_pow_254: pallas::Base,
        t_p: Expression<pallas::Base>,
    ) -> Self {
        let q_notecommit_recp = meta.selector();
        // recp = bits 0..7 of (d1) || 67..127 of recp (60)  || 127..187 of recp (60)  || 187..247 of recp  (20) || 247..255 recp 8
        meta.create_gate("NoteCommit input value", |meta| {
            let q_notecommit_recp = meta.query_selector(q_notecommit_recp);
            // Row 0
            let recp = meta.query_advice(col_l, Rotation::cur());
            let d1 = meta.query_advice(col_m, Rotation::cur()); // bits 0..7 (7 bits)
            let e = meta.query_advice(col_r, Rotation::cur()); // bits 7..247 (240 bits)
            let d1_e_f0_prime = meta.query_advice(col_z, Rotation::cur());

            // Row 1
            let f0 = meta.query_advice(col_m, Rotation::next()); // bits 247..255 (8 bits)
            meta.query_advice(col_z, Rotation::next());

            // Decomposition check: recp = d1 + e * 2^7 + f0 * 2^247
            let recp_check = d1.clone() + e.clone() * two_pow_7 + f0.clone() * two_pow_247 - recp;

            // Canonicity check: d1_e_f0_prime = d1 + e * 2^7 + f0 * 2^247 + 2^254 - t_P
            let d1_e_f0_prime_check =
                d1 + e * two_pow_7 + f0 * two_pow_247 + Expression::Constant(two_pow_254)
                    - t_p
                    - d1_e_f0_prime;

            Constraints::with_selector(
                q_notecommit_recp,
                [
                    ("recp_check", recp_check),
                    ("d1_e_f0_prime_check", d1_e_f0_prime_check),
                ],
            )
        });

        Self {
            q_notecommit_recp,
            col_l,
            col_m,
            col_r,
            col_z,
        }
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        recp: AssignedCell<pallas::Base, pallas::Base>,
        d1: AssignedCell<pallas::Base, pallas::Base>, // From DecomposeD::assign
        e: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bitrange_of(recp, 7..247)
        f0: AssignedCell<pallas::Base, pallas::Base>, // From DecomposeF::assign
        d1_e_f0_prime: AssignedCell<pallas::Base, pallas::Base>,
        z26_d1_e_f0_prime: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "NoteCommit recp canonicity",
            |mut region| {
                self.q_notecommit_recp.enable(&mut region, 0)?;

                // Row 0
                recp.copy_advice(|| "recp full", &mut region, self.col_l, 0)?;
                d1.copy_advice(|| "d1 (bits 0..7)", &mut region, self.col_m, 0)?;
                region.assign_advice(|| "e (bits 7..247)", self.col_r, 0, || *e.inner())?;
                d1_e_f0_prime.copy_advice(|| "d1_e_f0_prime", &mut region, self.col_z, 0)?;

                // Row 1
                f0.copy_advice(|| "f0 (bits 247..255)", &mut region, self.col_m, 1)?;
                z26_d1_e_f0_prime.copy_advice(
                    || "z26_d1_e_f0_prime",
                    &mut region,
                    self.col_z,
                    1,
                )?;

                Ok(())
            },
        )
    }
}

/// | A_6 | A_7 | A_8 | A_9 | q_notecommit_esk |
/// -------------------------------------------
/// | esk | f1  |  g  | h0  |       1          |
#[derive(Clone, Debug)]
struct EskCanonicity {
    q_notecommit_esk: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
    col_z: Column<Advice>,
}

impl EskCanonicity {
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        col_z: Column<Advice>,
        two_pow_2: pallas::Base,
        two_pow_252: pallas::Base,
        two_pow_254: pallas::Base,
        t_p: Expression<pallas::Base>,
    ) -> Self {
        let q_notecommit_esk = meta.selector();

        meta.create_gate("NoteCommit input value", |meta| {
            let q_notecommit_esk = meta.query_selector(q_notecommit_esk);

            // Row 0
            let esk = meta.query_advice(col_l, Rotation::cur());
            let f1 = meta.query_advice(col_m, Rotation::cur()); // bits 0..2
            let g = meta.query_advice(col_r, Rotation::cur()); // bits 2..252
            let f1_g_h0_prime = meta.query_advice(col_z, Rotation::cur());

            // Row 1
            let h0 = meta.query_advice(col_m, Rotation::next());
            let z26_f1_g_h0_prime = meta.query_advice(col_z, Rotation::next());

            // Decomposition check: esk = f1 + g * 2^2 + h0 * 2^252
            let esk_check = f1.clone() + g.clone() * two_pow_2 + h0.clone() * two_pow_252 - esk;

            // Canonicity check: f1_g_h0_prime = f1 + g * 2^2 + h0 * 2^252 + 2^254 - t_P
            let f1_g_h0_prime_check =
                f1 + g * two_pow_2 + h0 * two_pow_252 + Expression::Constant(two_pow_254)
                    - t_p
                    - f1_g_h0_prime;

            Constraints::with_selector(
                q_notecommit_esk,
                [
                    ("esk_check", esk_check),
                    ("f1_g_h0_prime_check", f1_g_h0_prime_check),
                ],
            )
        });

        Self {
            q_notecommit_esk,
            col_l,
            col_m,
            col_r,
            col_z,
        }
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        esk: AssignedCell<pallas::Base, pallas::Base>,
        f1: AssignedCell<pallas::Base, pallas::Base>,
        g: RangeConstrained<pallas::Base, Value<pallas::Base>>,
        h0: AssignedCell<pallas::Base, pallas::Base>,
        f1_g_h0_prime: AssignedCell<pallas::Base, pallas::Base>,
        z26_f1_g_h0_prime: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "NoteCommit esk canonicity",
            |mut region| {
                self.q_notecommit_esk.enable(&mut region, 0)?;

                // Row 0
                esk.copy_advice(|| "esk full", &mut region, self.col_l, 0)?;
                f1.copy_advice(|| "f1 (bits 0..2)", &mut region, self.col_m, 0)?;
                region.assign_advice(|| "g (bits 2..252)", self.col_r, 0, || *g.inner())?;
                f1_g_h0_prime.copy_advice(|| "f1_g_h0_prime", &mut region, self.col_z, 0)?;

                // Row 1
                h0.copy_advice(|| "h0 (bits 252..255)", &mut region, self.col_m, 1)?;
                z26_f1_g_h0_prime.copy_advice(
                    || "z26_f1_g_h0_prime",
                    &mut region,
                    self.col_z,
                    1,
                )?;

                Ok(())
            },
        )
    }
}

/// | A_6 | A_7 | A_8 | A_9          | q_notecommit_rho |
/// -----------------------------------------------------
/// | rho | h1  |  i  | h1_i_j0'     |        1         |
/// |     | j0  |     | z26_h1_i_j0' |        0         |
///
#[derive(Clone, Debug)]
struct RhoCanonicity {
    q_notecommit_rho: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
    col_z: Column<Advice>,
}

impl RhoCanonicity {
    #[allow(clippy::too_many_arguments)]
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        col_z: Column<Advice>,
        two_pow_7: pallas::Base,
        two_pow_247: pallas::Base,
        two_pow_254: pallas::Base,
        t_p: Expression<pallas::Base>,
    ) -> Self {
        let q_notecommit_rho = meta.selector();

        meta.create_gate("NoteCommit input rho", |meta| {
            let q_notecommit_rho = meta.query_selector(q_notecommit_rho);

            let rho = meta.query_advice(col_l, Rotation::cur()); // full 255-bit rho
            let h1 = meta.query_advice(col_m, Rotation::cur()); // bits 0..7 (7 bits, from DecomposeH)
            let i = meta.query_advice(col_r, Rotation::cur()); // bits 7..247 (240 bits, assigned as witness)

            let h1_i_j0_prime = meta.query_advice(col_z, Rotation::cur());
            let j0 = meta.query_advice(col_m, Rotation::next());

            // Row 1
            let z26_h1_i_j0_prime = meta.query_advice(col_z, Rotation::next());

            // Decomposition check: rho = h1 + i * 2^7 + j0 * 2^247
            let rho_check = h1.clone() + i.clone() * two_pow_7 + j0.clone() * two_pow_247 - rho;

            // Canonicity check: h1_i_j0_prime = h1 + i * 2^7 + j0 * 2^247 + 2^254 - t_P
            let h1_i_j0_prime_check =
                h1 + i * two_pow_7 + j0 * two_pow_247 + Expression::Constant(two_pow_254)
                    - t_p
                    - h1_i_j0_prime;

            Constraints::with_selector(
                q_notecommit_rho,
                [
                    ("rho_check", rho_check),
                    ("h1_i_j0_prime_check", h1_i_j0_prime_check),
                ],
            )
        });

        Self {
            q_notecommit_rho,
            col_l,
            col_m,
            col_r,
            col_z,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        rho: AssignedCell<pallas::Base, pallas::Base>,
        h1: AssignedCell<pallas::Base, pallas::Base>, // From DecomposeH::assign (h[1])
        i: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bitrange_of(rho, 7..247)
        j0: AssignedCell<pallas::Base, pallas::Base>, // From DecomposeJ::assign (j[0])
        h1_i_j0_prime: AssignedCell<pallas::Base, pallas::Base>,
        z26_h1_i_j0_prime: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "NoteCommit rho canonicity",
            |mut region| {
                self.q_notecommit_rho.enable(&mut region, 0);
                // Row 0
                rho.copy_advice(|| "rho full", &mut region, self.col_l, 0)?;
                h1.copy_advice(|| "h1 (bits 0..7)", &mut region, self.col_m, 0)?;
                region.assign_advice(|| "i (bits 7..247)", self.col_r, 0, || *i.inner())?;
                h1_i_j0_prime.copy_advice(|| "h1_i_j0_prime", &mut region, self.col_z, 0)?;

                // Row 1
                j0.copy_advice(|| "j0 (bits 247..255)", &mut region, self.col_m, 1)?;
                z26_h1_i_j0_prime.copy_advice(
                    || "z26_h1_i_j0_prime",
                    &mut region,
                    self.col_z,
                    1,
                )?;
                Ok(())
            },
        )
    }
}

/// | A_6 | A_7 | A_8 | A_9 | q_notecommit_psi |
/// -------------------------------------------
/// | psi | i1  | i2  | z13_i|        1         |
/// |     | i3  |     |      |        0         |
///
#[derive(Clone, Debug)]
struct PsiCanonicity {
    q_notecommit_psi: Selector,
    col_l: Column<Advice>,
    col_m: Column<Advice>,
    col_r: Column<Advice>,
    col_z: Column<Advice>,
}

impl PsiCanonicity {
    #[allow(clippy::too_many_arguments)]
    fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        col_l: Column<Advice>,
        col_m: Column<Advice>,
        col_r: Column<Advice>,
        col_z: Column<Advice>,
        two_pow_2: pallas::Base,
        two_pow_252: pallas::Base,
        two_pow_254: pallas::Base,
        t_p: Expression<pallas::Base>,
    ) -> Self {
        let q_notecommit_psi = meta.selector();

        meta.create_gate("NoteCommit input psi", |meta| {
            let q_notecommit_psi = meta.query_selector(q_notecommit_psi);

            let psi = meta.query_advice(col_l, Rotation::cur()); // full 255-bit psi
            let j1 = meta.query_advice(col_m, Rotation::cur()); // bits 0..2 (2 bits, from DecomposeJ)
            let k = meta.query_advice(col_r, Rotation::cur()); // bits 2..252 (250 bits, assigned as witness)
            let j1_k_l0_prime = meta.query_advice(col_z, Rotation::cur());

            let l0 = meta.query_advice(col_m, Rotation::next()); // bits 252..255 (3 bits, from DecomposeL)
            let z26_j1_k_l0_prime = meta.query_advice(col_z, Rotation::next());

            // Decomposition check: psi = j1 + k * 2^2 + l0 * 2^252
            let psi_check = j1.clone() + k.clone() * two_pow_2 + l0.clone() * two_pow_252 - psi;

            // Canonicity check: j1_k_l0_prime = j1 + k * 2^2 + l0 * 2^252 + 2^254 - t_P
            let j1_k_l0_prime_check =
                j1 + k * two_pow_2 + l0 * two_pow_252 + Expression::Constant(two_pow_254)
                    - t_p
                    - j1_k_l0_prime;

            Constraints::with_selector(
                q_notecommit_psi,
                [
                    ("psi_check", psi_check),
                    ("j1_k_l0_prime_check", j1_k_l0_prime_check),
                ],
            )
        });

        Self {
            q_notecommit_psi,
            col_l,
            col_m,
            col_r,
            col_z,
        }
    }

    fn assign(
        &self,
        lo: &mut impl Layouter<pallas::Base>,
        psi: AssignedCell<pallas::Base, pallas::Base>,
        j1: AssignedCell<pallas::Base, pallas::Base>, // From DecomposeJ::assign (j[1])
        k: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bitrange_of(psi, 2..252)
        l0: AssignedCell<pallas::Base, pallas::Base>, // From DecomposeL::assign (l[0])
        j1_k_l0_prime: AssignedCell<pallas::Base, pallas::Base>,
        z26_j1_k_l0_prime: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<(), Error> {
        lo.assign_region(
            || "NoteCommit psi canonicity",
            |mut region| {
                self.q_notecommit_psi.enable(&mut region, 0)?;
                // Row 0
                psi.copy_advice(|| "psi full", &mut region, self.col_l, 0)?;
                j1.copy_advice(|| "j1 (bits 0..2)", &mut region, self.col_m, 0)?;
                region.assign_advice(|| "k (bits 2..252)", self.col_r, 0, || *k.inner())?;
                j1_k_l0_prime.copy_advice(|| "j1_k_l0_prime", &mut region, self.col_z, 0)?;

                // Row 1
                l0.copy_advice(|| "l0 (bits 252..255)", &mut region, self.col_m, 1)?;
                z26_j1_k_l0_prime.copy_advice(
                    || "z26_j1_k_l0_prime",
                    &mut region,
                    self.col_z,
                    1,
                )?;

                Ok(())
            },
        )
    }
}

#[allow(non_snake_case)]
#[derive(Clone, Debug)]
pub struct NoteCommitConfig {
    b: DecomposeB,
    c: DecomposeC,
    d: DecomposeD,
    f: DecomposeF,
    h: DecomposeH,
    j: DecomposeJ,
    l: DecomposeL,
    nd: NdCanonicity,
    v: ValueCanonicity,
    fdi: FdiCanonicity,
    recp: RecpCanonicity,
    esk: EskCanonicity,
    rho: RhoCanonicity,
    psi: PsiCanonicity,
    advices: [Column<Advice>; 10],
    sinsemilla_config:
        SinsemillaConfig<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
}

#[derive(Clone, Debug)]
pub struct NoteCommitChip {
    config: NoteCommitConfig,
}

impl NoteCommitChip {
    #[allow(non_snake_case)]
    #[allow(clippy::many_single_char_names)]
    pub(in crate::circuit) fn configure(
        meta: &mut ConstraintSystem<pallas::Base>,
        advices: [Column<Advice>; 10],
        sinsemilla_config: SinsemillaConfig<
            OrchardHashDomains,
            OrchardCommitDomains,
            OrchardFixedBases,
        >,
    ) -> NoteCommitConfig {
        // Useful constants
        let two = pallas::Base::from(2);
        let two_pow_2 = pallas::Base::from(1 << 2);
        let two_pow_3: pasta_curves::Fp = two_pow_2 * two;
        let two_pow_4 = two_pow_2.square();
        let two_pow_5 = two_pow_4 * two;
        let two_pow_8 = two_pow_4.square();
        let two_pow_7 = pallas::Base::from(1 << 7);
        let two_pow_9 = two_pow_8 * two;
        let two_pow_13 = pallas::Base::from(1 << 13);
        // let two_pow_10 = two_pow_9 * two;
        // let two_pow_13 = pallas::Base::from(1 << 13);
        let two_pow_51 = pallas::Base::from(1 << 51);
        let two_pow_55 = pallas::Base::from(1 << 55);
        let two_pow_60 = pallas::Base::from(1 << 60);
        let two_pow_64 = two_pow_60 * two_pow_4;
        // let two_pow_58 = pallas::Base::from(1 << 58);
        // let two_pow_117 = two_pow_57.square() * two_pow_3;
        let two_pow_120 = two_pow_60.square();
        let two_pow_240 = two_pow_120.square();
        // let two_pow_177 = two_pow_117 * two_pow_60;
        // let two_pow_237 = two_pow_177 * two_pow_60;
        let two_pow_247 = two_pow_240 * two_pow_7;
        let two_pow_249 = pallas::Base::from_u128(1 << 124).square() * two;
        let two_pow_250 = two_pow_249 * two;
        let two_pow_252 = two_pow_250 * two_pow_2;
        let two_pow_254 = two_pow_252 * two_pow_2;

        let t_p = Expression::Constant(pallas::Base::from_u128(T_P));

        // Columns used for MessagePiece and message input gates.
        let col_l = advices[6];
        let col_m = advices[7];
        let col_r = advices[8];
        let col_z = advices[9];

        let b = DecomposeB::configure(meta, col_l, col_m, col_r, two_pow_5);
        let c = DecomposeC::configure(meta, col_l, col_m, col_r, two_pow_9);
        let d = DecomposeD::configure(meta, col_l, col_m, col_r, two_pow_13);
        let f = DecomposeF::configure(meta, col_l, col_m, col_r, two_pow_8);
        let h = DecomposeH::configure(meta, col_l, col_m, col_r, two_pow_3);
        let j = DecomposeJ::configure(meta, col_l, col_m, col_r, two_pow_8);
        let l = DecomposeL::configure(meta, col_l, col_m, col_r, two_pow_3);

        let nd = NdCanonicity::configure(
            meta,
            col_l,
            col_m,
            col_r,
            col_z,
            two_pow_250.clone(),
            t_p.clone(),
        );

        let v = ValueCanonicity::configure(
            meta,
            col_l,
            col_m,
            col_r,
            col_z,
            two_pow_55,
            two_pow_64,
            t_p.clone(),
        );
        let fdi = FdiCanonicity::configure(
            meta,
            col_l,
            col_m,
            col_r,
            col_z,
            two_pow_51,
            two_pow_64,
            t_p.clone(),
        );
        let recp = RecpCanonicity::configure(
            meta,
            col_l,
            col_m,
            col_r,
            col_z,
            two_pow_7,
            two_pow_247,
            two_pow_254,
            t_p.clone(),
        );

        let esk = EskCanonicity::configure(
            meta,
            col_l,
            col_m,
            col_r,
            col_z,
            two_pow_2,
            two_pow_252,
            two_pow_254,
            t_p.clone(),
        );

        let rho = RhoCanonicity::configure(
            meta,
            col_l,
            col_m,
            col_r,
            col_z,
            two_pow_7,
            two_pow_247,
            two_pow_254,
            t_p.clone(),
        );

        let psi = PsiCanonicity::configure(
            meta,
            col_l,
            col_m,
            col_r,
            col_z,
            two_pow_2,
            two_pow_252,
            two_pow_254,
            t_p,
        );

        NoteCommitConfig {
            b,
            c,
            d,
            f,
            h,
            j,
            l,
            nd,
            v,
            fdi,
            recp,
            esk,
            rho,
            psi,
            advices,
            sinsemilla_config,
        }
    }

    pub(in crate::circuit) fn construct(config: NoteCommitConfig) -> Self {
        Self { config }
    }
}

pub(in crate::circuit) mod gadgets {
    use halo2_gadgets::sinsemilla::HashDomain;
    use halo2_proofs::circuit::{Chip, Value};

    use super::*;

    #[allow(clippy::many_single_char_names)]
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::circuit) fn note_commit(
        mut lo: impl Layouter<pallas::Base>,
        chip: SinsemillaChip<OrchardHashDomains, OrchardCommitDomains, OrchardFixedBases>,
        ecc_chip: EccChip<OrchardFixedBases>,
        note_commit_chip: NoteCommitChip,
        nd: AssignedCell<pallas::Base, pallas::Base>,
        v: AssignedCell<NoteValue, pallas::Base>,
        fdi: AssignedCell<pallas::Base, pallas::Base>,
        recp: AssignedCell<pallas::Base, pallas::Base>,
        esk: AssignedCell<pallas::Base, pallas::Base>,
        rho: AssignedCell<pallas::Base, pallas::Base>,
        psi: AssignedCell<pallas::Base, pallas::Base>,
        rcm: ScalarFixed<pallas::Affine, EccChip<OrchardFixedBases>>,
    ) -> Result<Point<pallas::Affine, EccChip<OrchardFixedBases>>, Error> {
        // Headstash NoteCommitment Message: nd(254) || v(64) || fdi(64) || recp(254) || esk(254) || rho(254) || psi(254)
        // Total: 1398 bits
        //
        // Optimized decomposition for Sinsemilla (250 bit pieces, 10-bit limb alignment):
        let lc = chip.config().lookup_config();

        // Piece a: bits 0-249 of nd (250 bits)
        let a = MessagePiece::from_subpieces(
            chip.clone(),
            lo.namespace(|| "piece_a: nd[0..250)"),
            [RangeConstrained::bitrange_of(nd.value(), 0..250)],
        )?;
        // Piece b: bits 250..255 of nd || 0..55 of v  (5 + 55 = 60 bits)
        let (b, b0, b1) = DecomposeB::decompose(&lc, chip.clone(), &mut lo, &nd, &v)?;
        // Piece c: bits 55-64 of v || 0-51 of fdi  (9 + 51 = 60 bits)
        let (c, c0, c1) = DecomposeC::decompose(chip.clone(), &mut lo, &v, &fdi)?;
        // Piece d: bits  51-64 of fdi || 0..7 of recp (13 + 7 = 20 bits)
        let (d, d0, d1) = DecomposeD::decompose(chip.clone(), &mut lo, &fdi, &recp)?;
        // Piece e:  bits 7..247 of recp (240 bits)
        // First, create and save the value for e
        let e_value = RangeConstrained::bitrange_of(recp.value(), 7..247);
        let e = MessagePiece::from_subpieces(
            chip.clone(),
            lo.namespace(|| "piece_e: recp[7..247)"),
            [e_value],
        )?;
        // Piece f:  bits 247..255 of recp (8) || 0..2 of esk (8 + 2 = 10 bits)
        let (f, [f0, f1]) = DecomposeF::decompose(chip.clone(), &mut lo, &recp, &esk)?;
        // Piece g: bits 2..252 of esk (250 bits)
        let g_value = RangeConstrained::bitrange_of(esk.value(), 2..252);
        let g = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "g: "), [g_value])?;
        // Piece h: bits 252..255 of esk (3) || bits 0..7 of rho (7) (7+3 = 10 bits)
        let (h, [h0, h1]) = DecomposeH::decompose(chip.clone(), &mut lo, &esk, &rho)?;
        // Piece i: bits 7..247 of rho (240 bits)
        // Save the value for canonicity check
        let i_value = RangeConstrained::bitrange_of(rho.value(), 7..247);
        let i = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "g: "), [i_value])?;
        // Piece j: bits 247..255 of rho (8) || bits 0..2 of psi (8+2 = 10 bits)
        let (j, [j0, j1]) = DecomposeJ::decompose(chip.clone(), &mut lo, &rho, &psi)?;
        // Piece k: bits 2..252 psi
        let k_value = RangeConstrained::bitrange_of(psi.value(), 2..252);
        let k = MessagePiece::from_subpieces(chip.clone(), lo.namespace(|| "k"), [k_value])?;
        // Piece l: bits 252..255 of psi (3 bits) || 7 bit padding (3+7 = 10 bits)
        let (l, [l0, l1]) = DecomposeL::decompose(chip.clone(), &mut lo, &psi)?;

        // println!("Message pieces:");
        // println!("a: {:?}", a);
        // println!("b: {:?}", b);
        // println!("b0: {:?}", b0);
        // println!("b1: {:?}", b1);
        // println!("c: {:?}", c);
        // println!("c0: {:?}", c0);
        // println!("c1: {:?}", c1);
        // println!("d: {:?}", d);
        // println!("d0: {:?}", d0);
        // println!("d1: {:?}", d1);
        // println!("e: {:?}", e);
        // println!("f0: {:?}", f0);
        // println!("f1: {:?}", f1);
        // println!("f: {:?}", f);
        // println!("g: {:?}", g);
        // println!("h0: {:?}", h0);
        // println!("h1: {:?}", h1);
        // println!("h: {:?}", h);
        // println!("i: {:?}", i);
        // println!("j0: {:?}", j0);
        // println!("j1: {:?}", j1);
        // println!("j: {:?}", j);
        // println!("k: {:?}", k);
        // println!("l0: {:?}", l0);
        // println!("l1: {:?}", l1);
        // println!("l: {:?}", l);

        // cm = NoteCommit^Headstash_rcm( nd || i2lebsp_{64}(v) || i2lebsp_{64}(fdi) || recp || esk  || rho || psi )
        //
        // `cm = ⊥` is handled internally to `CommitDomain::commit`: incomplete addition
        // constraints allows ⊥ to occur, and then during synthesis it detects these edge
        // cases and raises an error (aborting proof creation).
        let (cm, zs) = {
            let message = Message::from_pieces(
                chip.clone(),
                vec![
                    a.clone(),
                    b.clone(),
                    c.clone(),
                    d.clone(),
                    e.clone(),
                    f.clone(),
                    g.clone(),
                    h.clone(),
                    i.clone(),
                    j.clone(),
                    k.clone(),
                    l.clone(),
                ],
            );
            let domain = CommitDomain::new(chip, ecc_chip, &OrchardCommitDomains::NoteCommit);
            domain.commit(lo.namespace(|| "Process NoteCommit inputs"), message, rcm)?
        };

        // `CommitDomain::commit` returns the running sum for each `MessagePiece`. Grab
        // the outputs that we will need for canonicity checks.'
        // println!("Running sums length:");
        // println!("zs:{:#?}", zs.len());
        // println!("zs[0]:{:#?}", zs[0].len());
        // println!("zs[1]:{:#?}", zs[1].len());
        // println!("zs[2]:{:#?}", zs[2].len());
        // println!("zs[3]:{:#?}", zs[3].len());
        // println!("zs[4]:{:#?}", zs[4].len());
        // println!("zs[5]:{:#?}", zs[5].len());
        // println!("zs[6]:{:#?}", zs[6].len());
        // println!("zs[7]:{:#?}", zs[7].len());
        // println!("zs[8]:{:#?}", zs[8].len());
        // println!("zs[9]:{:#?}", zs[9].len());
        // println!("zs[10]:{:#?}", zs[10].len());
        // println!("zs[11]:{:#?}", zs[11].len());

        // `CommitDomain::commit` returns the running sum for each `MessagePiece`. Grab
        // the outputs that we will need for canonicity checks.

        // Piece indices: a=0, b=1, c=2, d=3, e=4, f=5, g=6, h=7, i=8, j=9, k=10, l=11

        // ===== ND CANONICITY (piece a, b) =====
        // Piece a: nd[0..250) = 250 bits = 25 ten-bit words → zs[0] has 26 elements
        let z13_a = zs[0][13].clone(); // Midpoint check for nd's low 250 bits
        let z25_a = zs[0][24].clone(); // End of piece a
                                       // ===== VALUE CANONICITY (piece b, c) =====
                                       // Piece b: nd[250..255) || v[0..55) = 60 bits = 6 words → zs[1] has 7 elements
        let z1_b = zs[1][1].clone(); // After first word boundary in piece b
        let z6_b = zs[1][5].clone(); // End of piece b (end of v's low 55 bits)
                                     // Piece c: v[55..64) || fdi[0..51) = 60 bits = 6 words → zs[2] has 7 elements
        let z1_c = zs[2][1].clone(); // After v[55..64) completes (9 bits → in first word)
        let z6_c = zs[2][5].clone(); // End of piece c
                                     // ===== FDI CANONICITY (piece c, d) =====
                                     // Piece d: fdi[51..64) || recp[0..7) = 20 bits = 2 words → zs[3] has 3 elements
        let z1_d = zs[3][0].clone(); // After fdi[51..64) completes (13 bits → in second word)
        let z2_d = zs[3][1].clone(); // End of piece d
                                     // ===== RECP CANONICITY (piece d, e, f) =====
                                     // Piece e: recp[7..247) = 240 bits = 24 words → zs[4] has 25 elements
        let z13_e = zs[4][13].clone(); // Midpoint check for recp middle bits
        let z24_e = zs[4][23].clone(); // End of piece e
                                       // Piece f: recp[247..255) || esk[0..2) = 10 bits = 1 word → zs[5] has 2 elements
        let z1_f = zs[5][0].clone(); // End of piece f (completes recp high 8 bits)
                                     // ===== ESK CANONICITY (piece f, g, h) =====
                                     // Piece g: esk[2..247) = 245 bits = 24.5 words → zs[6] has 25 or 26 elements (check actual)
        let z13_g = zs[6][13].clone(); // Midpoint check for esk middle bits
        let z24_g = zs[6][24].clone(); // Near end of piece g
                                       // Piece h: esk[247..255) || rho[0..2) = 10 bits = 1 word → zs[7] has 2 elements
        let z1_h = zs[7][0].clone(); // End of piece h (completes esk high 8 bits)
                                     // ===== RHO CANONICITY (piece h, i, j) =====
                                     // Piece i: rho[2..252) = 250 bits = 25 words → zs[8] has 26 elements
        let z13_i = zs[8][13].clone(); // Midpoint check for rho middle bits
        let z25_i = zs[8][23].clone(); // End of piece i
                                       // Piece j: rho[252..255) || psi[0..7) = 10 bits = 1 word → zs[9] has 2 elements
        let z1_j = zs[9][0].clone(); // End of piece j (completes rho high 3 bits)
                                     // Piece k: psi[7..247) = 240 bits = 24 words → zs[10] has 25 elements
        let z13_k = zs[10][13].clone(); // Midpoint check for psi middle bits
        let z24_k = zs[10][24].clone(); // End of piece k
                                        // Piece l: psi[247..255) || padding(2) = 10 bits = 1 word → zs[11] has 2 elements
        let z1_l = zs[11][0].clone(); // End of piece l (completes psi high 8 bits)

        // println!("Running sums:");
        // // ===== ND CANONICITY =====
        // println!("z13_a: {:?}", z13_a);
        // println!("z25_a: {:?}", z25_a);
        // // ===== VALUE CANONICITY =====
        // println!("z1_b: {:?}", z1_b);
        // println!("z6_b: {:?}", z6_b);
        // println!("z1_c: {:?}", z1_c);
        // println!("z6_c: {:?}", z6_c);
        // // ===== FDI CANONICITY =====
        // println!("z1_d: {:?}", z1_d);
        // println!("z2_d: {:?}", z2_d);
        // // ===== RECP CANONICITY =====
        // println!("z13_e: {:?}", z13_e);
        // println!("z24_e: {:?}", z24_e);
        // println!("z1_f: {:?}", z1_f);
        // // ===== ESK CANONICITY =====
        // println!("z13_g: {:?}", z13_g);
        // println!("z24_g: {:?}", z24_g);
        // println!("z1_h: {:?}", z1_h);
        // // ===== RHO CANONICITY =====
        // println!("z13_i: {:?}", z13_i);
        // println!("z25_i: {:?}", z25_i);
        // println!("z1_j: {:?}", z1_j);
        // // ===== PSI CANONICITY =====
        // println!("z13_k: {:?}", z13_k);
        // println!("z24_k: {:?}", z24_k);
        // println!("z1_l: {:?}", z1_l);

        // Check decomposition of nd (a,b0)
        let (a_prime, z13_a_prime) = nd_canonicity(
            &lc,
            lo.namespace(|| "nd canonicity"),
            a.inner().cell_value(),
        )?;

        // Check decomposition of v (b1, c0)
        let (b1_c0_prime, z6_b1_c0_prime) =
            v_canonicity(&lc, lo.namespace(|| "v canonicity"), b1.clone(), c0.clone())?;

        // Check decomposition of fdi (c1,d0)
        let (c1_d0_prime, z6_c1_d0_prime) = fdi_canonicity(
            &lc,
            lo.namespace(|| "fdi canonicity"),
            c1.clone(),
            d0.clone(),
        )?;

        // Check decomposition of recp (d1,e,f0)
        let (d1_e_f0_prime, z26_d1_e_f0_prime) = recp_canonicity(
            &lc,
            lo.namespace(|| "v canonicity"),
            d1.clone(),
            e_value,
            f0.clone(),
        )?;

        // Check decomposition of esk (f1,g,h0)
        let (f1_g_h0_prime, z26_f1_g_h0_prime) = esk_canonicity(
            &lc,
            lo.namespace(|| "esk canonicity"),
            f1.clone(),
            g.inner().cell_value(),
            h0.clone(),
        )?;

        // Check decomposition of rho (h1,i,j0)
        let (h1_i_j0_prime, z26_h1_i_j0_prime) = rho_canonicity(
            &lc,
            lo.namespace(|| "rho canonicity"),
            h1.clone(),
            i.inner().cell_value(),
            j0.clone(),
        )?;

        // Check decomposition of psi (j1, k, l0)
        let (j1_k_l0_prime, z26_j1_k_l0_prime) = psi_canonicity(
            &lc,
            lo.namespace(|| "psi canonicity"),
            j1.clone(),
            k.inner().cell_value(),
            l0.clone(),
        )?;

        // Finally, assign vs to all of the NoteCommit regions.
        let cfg = note_commit_chip.config;

        let b_1 = cfg.b.assign(&mut lo, b, b0.clone(), b1)?;
        let [c0, c1] = cfg.c.assign(&mut lo, c, c0.clone(), c1)?;
        let [d0, d1] = cfg.d.assign(&mut lo, d, d0, d1, z1_d)?;
        let [f0, f1] = cfg.f.assign(&mut lo, f, [f0, f1])?;
        let [h0, h1] = cfg.h.assign(&mut lo, h, [h0, h1])?;
        let [j0, j1] = cfg.j.assign(&mut lo, j, [j0, j1])?;
        let [l0, l1] = cfg.l.assign(&mut lo, l, [l0, l1])?;

        cfg.nd
            .assign(&mut lo, &nd, a, b0, b_1, a_prime, z13_a, z13_a_prime)?;
        cfg.v
            .assign(&mut lo, v, b1, c0, b1_c0_prime, z6_b1_c0_prime)?;
        cfg.fdi
            .assign(&mut lo, fdi, c1, d0, c1_d0_prime, z6_c1_d0_prime)?;
        cfg.recp.assign(
            &mut lo,
            recp,
            d1,
            e_value,
            f0,
            d1_e_f0_prime,
            z26_d1_e_f0_prime,
        )?;
        cfg.esk.assign(
            &mut lo,
            esk,
            f1,
            g_value,
            h0,
            f1_g_h0_prime,
            z26_f1_g_h0_prime,
        )?;
        cfg.rho.assign(
            &mut lo,
            rho,
            h1,
            i_value,
            j0,
            h1_i_j0_prime,
            z26_h1_i_j0_prime,
        )?;
        cfg.psi.assign(
            &mut lo,
            psi,
            j1,
            k_value,
            l0,
            j1_k_l0_prime,
            z26_j1_k_l0_prime,
        )?;
        Ok(cm)
    }

    /// A canonicity check helper used in checking nd.
    fn nd_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        a: AssignedCell<pallas::Base, pallas::Base>,
    ) -> Result<CanonicityBounds, Error> {
        // element = a (250 bits)
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
        assert_eq!(zs.len(), 26); // [z_0, z_1, ..., z_25]
        Ok((a_prime, zs[25].clone()))
    }

    /// Check canonicity of `v` encoding.
    fn v_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        b1: RangeConstrained<pallas::Base, Value<pasta_curves::Fp>>,
        c0: RangeConstrained<pallas::Base, Value<pasta_curves::Fp>>, // Updated to take c0 directly
    ) -> Result<CanonicityBounds, Error> {
        // `v` = `b1 (55 bits) || c0 (9 bits)`
        // Canonicity check: b1 + 2^55 * c0 < t_P

        let b1_c0_prime = {
            let two_pow_55 = Value::known(pallas::Base::from(1u64 << 55));
            let two_pow_64 = two_pow_55.value() * Value::known(pallas::Base::from(1u64 << 9));
            let t_p = Value::known(pallas::Base::from_u128(T_P));
            b1.inner().value() + (two_pow_55 * c0.inner().value()) + two_pow_64 - t_p
        };
        let zs = lc.witness_check(
            lo.namespace(|| "Decompose low 64 bits of (b1 + 2^55 * c0 + 2^64 - t_P)"),
            b1_c0_prime,
            6,
            false,
        )?;
        let b1_c0_prime = zs[0].clone();
        assert_eq!(zs.len(), 7); // [z_0, z_1, ..., z_6]
        Ok((b1_c0_prime, zs[6].clone()))
    }

    fn fdi_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        c1: RangeConstrained<pallas::Base, Value<pasta_curves::Fp>>, // bits 0..51 fdi
        d0: RangeConstrained<pallas::Base, Value<pasta_curves::Fp>>, // bits 51..64 fdi
    ) -> Result<CanonicityBounds, Error> {
        // Check canonicity of `fdi` encoding.
        //
        // `fdi` = `c1 (51 bits) || d0 (13 bits)`
        let c1_d0_prime = {
            let two_pow_51 = Value::known(pallas::Base::from(1u64 << 51));
            let two_pow_64 = two_pow_51.value() * Value::known(pallas::Base::from(1u64 << 13));
            let t_p = Value::known(pallas::Base::from_u128(T_P));
            c1.inner().value() + (two_pow_51 * d0.inner().value()) + two_pow_64 - t_p
        };
        let zs = lc.witness_check(
            lo.namespace(|| "Decompose low 64 bits of (c1 + 2^51 * d0 + 2^64 - t_P)"),
            c1_d0_prime,
            6,
            false,
        )?;
        let c1_d0_prime = zs[0].clone();
        assert_eq!(zs.len(), 7); // [z_0, z_1, ..., z_6]
        Ok((c1_d0_prime, zs[6].clone()))
    }

    /// Check canonicity of `recp` encoding.
    fn recp_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        d1: RangeConstrained<pallas::Base, Value<pasta_curves::Fp>>,
        e: RangeConstrained<pallas::Base, Value<pallas::Base>>,
        f0: RangeConstrained<pallas::Base, Value<pasta_curves::Fp>>,
    ) -> Result<CanonicityBounds, Error> {
        // Compute full recp value from decomposed pieces for verification
        let d1_e_f0_prime = {
            let two_pow_7 = pallas::Base::from(1u64 << 7);
            let two_pow_240 = pallas::Base::from(1u64 << 60).square().square();
            let two_pow_247 = two_pow_240 * two_pow_7;
            let two_pow_254 = two_pow_247 * two_pow_7;
            let t_p = pallas::Base::from_u128(T_P);

            d1.inner()
                .value()
                .zip(e.inner().value())
                .zip(f0.inner().value())
                .map(|((d1_val, e_val), f0_val)| {
                    *d1_val + two_pow_7 * e_val + two_pow_247 * f0_val + two_pow_254 - t_p
                })
        };

        // For 254-bit range check: 254 bits / 10 bits per chunk = 25.4, so use 26 chunks
        let zs = lc.witness_check(
            lo.namespace(|| "Decompose (d1 + e * 2^7 + f0 * 2^247 + 2^254 - t_P)"),
            d1_e_f0_prime,
            25,
            false,
        )?;

        let d1_e_f0_prime = zs[0].clone();
        assert_eq!(zs.len(), 26); // [z_0, z_1, ..., z_26]
        Ok((d1_e_f0_prime, zs[25].clone()))
    }

    /// Check canonicity of `esk` encoding.
    fn esk_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        f1: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bits 0..7 of esk
        g: AssignedCell<pallas::Base, pallas::Base>,             // bits 7..247 of esk (240 bits)
        h0: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bits 247..255 of esk
    ) -> Result<CanonicityBounds, Error> {
        let f1_g_h0_prime = {
            let two = pallas::Base::from(1u64 << 2);
            let five = pallas::Base::from(1u64 << 5);
            let seven = pallas::Base::from(1u64 << 7);
            let sixty = pallas::Base::from(1u64 << 60);
            let two_pow_2 = Value::known(two);
            let two_pow_5 = Value::known(five);
            let two_pow_7 = Value::known(seven);
            let two_pow_240 = Value::known(sixty.square().square());
            let two_pow_247 = two_pow_240 * two_pow_7;
            let two_pow_252 = two_pow_247 * two_pow_5;
            let two_pow_254 = two_pow_247 * two_pow_7;
            let t_p = Value::known(pallas::Base::from_u128(T_P));

            f1.inner().value()
                + two_pow_2 * g.value()
                + two_pow_252 * h0.inner().value()
                + two_pow_254
                - t_p
        };

        let zs = lc.witness_check(
            lo.namespace(|| "Decompose (f1 + g * 2^2 + h0 * 2^252 + 2^254 - t_P)"),
            f1_g_h0_prime,
            25,
            false,
        )?;

        let esk_prime_cell = zs[0].clone();
        assert_eq!(zs.len(), 26);
        Ok((esk_prime_cell, zs[25].clone()))
    }

    /// Check canonicity of `rho` encoding.
    fn rho_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        h1: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bits 0..7 of rho
        i: AssignedCell<pallas::Base, pallas::Base>,             // bits 7..247 of rho (240 bits)
        j0: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bits 247..255 of rho (8 bits)
    ) -> Result<CanonicityBounds, Error> {
        let h1_i_j0_prime = {
            let two_pow_7 = Value::known(pallas::Base::from(1u64 << 7));
            let two_pow_240 = Value::known(pallas::Base::from(1u64 << 60).square().square());
            let two_pow_247 = two_pow_240 * two_pow_7;
            let two_pow_254 = two_pow_247 * two_pow_7;
            let t_p = Value::known(pallas::Base::from_u128(T_P));
            h1.inner().value()
                + two_pow_7 * i.value()
                + two_pow_247 * j0.inner().value()
                + two_pow_254
                - t_p
        };

        let zs = lc.witness_check(
            lo.namespace(|| "Decompose low 254 bits of (rho + 2^254 - t_P)"),
            h1_i_j0_prime,
            25,
            false,
        )?;

        let h1_i_j0_prime_cell = zs[0].clone();
        assert_eq!(zs.len(), 26); // [z_0, z_1, ..., z_26]
        Ok((h1_i_j0_prime_cell, zs[25].clone()))
    }

    // Check canonicity of `psi` encoding.
    fn psi_canonicity(
        lc: &LookupRangeCheckConfig<pallas::Base, 10>,
        mut lo: impl Layouter<pallas::Base>,
        j1: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bits 0..2 (2 bits)
        k: AssignedCell<pallas::Base, pallas::Base>,             // bits 2..252 (250 bits)
        l0: RangeConstrained<pallas::Base, Value<pallas::Base>>, // bits 252..255 (3 bits)
    ) -> Result<CanonicityBounds, Error> {
        let j1_k_l0_prime = {
            let two = pallas::Base::from(1u64 << 2);
            let five = pallas::Base::from(1u64 << 5);
            let seven = pallas::Base::from(1u64 << 7);
            let sixty = pallas::Base::from(1u64 << 60);
            let two_pow_2 = Value::known(two);
            let two_pow_5 = Value::known(five);
            let two_pow_7 = Value::known(seven);
            let two_pow_240 = Value::known(sixty.square().square());
            let two_pow_247 = two_pow_240 * two_pow_7;
            let two_pow_252 = two_pow_247 * two_pow_5;
            let two_pow_254 = two_pow_247 * two_pow_7;
            let t_p = Value::known(pallas::Base::from_u128(T_P));
            j1.inner().value()
                + two_pow_2 * k.value()
                + two_pow_252 * l0.inner().value()
                + two_pow_254
                - t_p
        };

        let zs = lc.witness_check(
            lo.namespace(|| "Decompose (j1 + k * 2^2 + l0 * 2^252 + 2^254 - t_P)"),
            j1_k_l0_prime,
            25,
            false,
        )?;

        let j1_k_l0_prime_cell = zs[0].clone();
        assert_eq!(zs.len(), 26); // [z_0, z_1, ..., z_26]
        Ok((j1_k_l0_prime_cell, zs[25].clone()))
    }
}

#[cfg(test)]
mod tests {
    use core::u64;

    use super::NoteCommitConfig;
    use crate::{
        circuit::{
            gadget::assign_free_advice,
            note_commit::{gadgets, NoteCommitChip},
        },
        constants::{
            fixed_bases::NOTE_COMMITMENT_PERSONALIZATION, OrchardCommitDomains, OrchardFixedBases,
            OrchardHashDomains, L_ORCHARD_BASE, L_VALUE, T_Q,
        },
        value::NoteValue,
    };
    use halo2_gadgets::{
        ecc::{
            chip::{EccChip, EccConfig},
            NonIdentityPoint, ScalarFixed,
        },
        sinsemilla::{chip::SinsemillaChip, primitives::CommitDomain},
        utilities::lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig},
    };

    use ff::{Field, PrimeField, PrimeFieldBits};
    use group::Curve;
    use halo2_proofs::{
        circuit::{Layouter, SimpleFloorPlanner, Value},
        dev::MockProver,
        plonk::{ConstraintSystem, Error},
    };
    use pasta_curves::pallas;

    use crate::os_rng;

    #[test]
    fn decomposition_values() {
        let u = u64::MAX;

        for i in &u.to_le_bytes()[0..7] {
            assert_eq!(i.to_le_bytes().len(), 1);
            // println!("H:{:#?}", i);
            // println!("H:{:#?}", u);
        }
    }

    #[test]
    fn note_commit() {
        #[derive(Default)]
        struct MyCircuit {
            nd: Value<pallas::Base>,
            v: Value<NoteValue>,
            fdi: Value<pallas::Base>,
            recp: Value<pallas::Base>,
            esk: Value<pallas::Base>,
            rho: Value<pallas::Base>,
            psi: Value<pallas::Base>,
        }

        impl halo2_proofs::plonk::Circuit<pallas::Base> for MyCircuit {
            type Config = (NoteCommitConfig, EccConfig<OrchardFixedBases>);
            type FloorPlanner = SimpleFloorPlanner;

            fn without_witnesses(&self) -> Self {
                Self::default()
            }

            fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
                let advices = [
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                    meta.advice_column(),
                ];

                // Shared fixed column for loading constants.
                let constants = meta.fixed_column();
                meta.enable_constant(constants);

                for advice in advices.iter() {
                    meta.enable_equality(*advice);
                }

                let table_idx = meta.lookup_table_column();
                let lookup = (
                    table_idx,
                    meta.lookup_table_column(),
                    meta.lookup_table_column(),
                );
                let lagrange_coeffs = [
                    meta.fixed_column(),
                    meta.fixed_column(),
                    meta.fixed_column(),
                    meta.fixed_column(),
                    meta.fixed_column(),
                    meta.fixed_column(),
                    meta.fixed_column(),
                    meta.fixed_column(),
                ];

                let range_check = LookupRangeCheckConfig::configure(meta, advices[9], table_idx);
                let sinsemilla_config = SinsemillaChip::<
                    OrchardHashDomains,
                    OrchardCommitDomains,
                    OrchardFixedBases,
                >::configure(
                    meta,
                    advices[..5].try_into().unwrap(),
                    advices[2],
                    lagrange_coeffs[0],
                    lookup,
                    range_check,
                    false,
                );
                let note_commit_config =
                    NoteCommitChip::configure(meta, advices, sinsemilla_config);

                let ecc_config = EccChip::<OrchardFixedBases>::configure(
                    meta,
                    advices,
                    lagrange_coeffs,
                    range_check,
                );

                (note_commit_config, ecc_config)
            }

            fn synthesize(
                &self,
                config: Self::Config,
                mut lo: impl Layouter<pallas::Base>,
            ) -> Result<(), Error> {
                let (note_commit_config, ecc_config) = config;

                // Load the Sinsemilla generator lookup table used by the whole circuit.
                SinsemillaChip::<
                OrchardHashDomains,
                OrchardCommitDomains,
                OrchardFixedBases,
            >::load(note_commit_config.sinsemilla_config.clone(), &mut lo)?;

                // Construct a Sinsemilla chip
                let sinsemilla_chip =
                    SinsemillaChip::construct(note_commit_config.sinsemilla_config.clone());

                // Construct an ECC chip
                let ecc_chip = EccChip::construct(
                    ecc_config,
                    halo2_gadgets::ecc::CircuitVersion::AnchoredBase,
                );

                // Construct a NoteCommit chip
                let note_commit_chip = NoteCommitChip::construct(note_commit_config.clone());

                // Witness nd.
                let nd = assign_free_advice(
                    lo.namespace(|| "witness nd"),
                    note_commit_config.advices[0],
                    self.nd,
                )?;

                // Witness a random non-negative u64 note v
                // A note v cannot be negative.
                let v = assign_free_advice(
                    lo.namespace(|| "witness v"),
                    note_commit_config.advices[0],
                    self.v,
                )?;

                // Witness fdi.
                let fdi = assign_free_advice(
                    lo.namespace(|| "witness fdi"),
                    note_commit_config.advices[0],
                    self.fdi,
                )?;

                // Witness recp.
                let recp = assign_free_advice(
                    lo.namespace(|| "witness recp"),
                    note_commit_config.advices[0],
                    self.recp,
                )?;
                // Witness esk.
                let esk = assign_free_advice(
                    lo.namespace(|| "witness esk"),
                    note_commit_config.advices[0],
                    self.esk,
                )?;

                // Witness rho
                let rho = assign_free_advice(
                    lo.namespace(|| "witness rho"),
                    note_commit_config.advices[0],
                    self.rho,
                )?;

                // Witness psi
                let psi = assign_free_advice(
                    lo.namespace(|| "witness psi"),
                    note_commit_config.advices[0],
                    self.psi,
                )?;

                let rcm = pallas::Scalar::random(&mut rand::rng());
                let rcm_gadget =
                    ScalarFixed::new(ecc_chip.clone(), lo.namespace(|| "rcm"), Value::known(rcm))?;

                let cm = gadgets::note_commit(
                    lo.namespace(|| "Hash NoteCommit pieces"),
                    sinsemilla_chip,
                    ecc_chip.clone(),
                    note_commit_chip,
                    nd,
                    v,
                    fdi,
                    recp,
                    esk,
                    rho,
                    psi,
                    rcm_gadget,
                )?;
                let expected_cm = {
                    let domain = CommitDomain::new(NOTE_COMMITMENT_PERSONALIZATION);
                    // Hash nd || i2lebsp_{64}(v) || i2lebsp_{64}(fdi) || recp || esk || rho || psi || 7 bit padding

                    // Debug: Print expected test values
                    // println!("\n=== TEST EXPECTED VALUES ===");
                    // self.nd.map(|v| println!("test nd: {:?}", v));
                    // self.v.map(|v| println!("test v: {:?}", v));
                    // self.fdi.map(|v| println!("test fdi: {:?}", v));
                    // self.recp.map(|v| println!("test recp: {:?}", v));
                    // self.esk.map(|v| println!("test esk: {:?}", v));
                    // self.rho.map(|v| println!("test rho: {:?}", v));
                    // self.psi.map(|v| println!("test psi: {:?}", v));

                    let point = self
                        .nd
                        .zip(self.v)
                        .zip(self.fdi.zip(self.recp.zip(self.esk)))
                        .zip(self.rho.zip(self.psi))
                        .map(|(((nd, v), (fdi, (recp, esk))), (rho, psi))| {
                            domain
                                .commit(
                                    nd.to_le_bits()
                                        .iter()
                                        .by_vals()
                                        .take(L_ORCHARD_BASE)
                                        .chain(v.to_le_bits().iter().by_vals().take(L_VALUE))
                                        .chain(fdi.to_le_bits().iter().by_vals().take(L_VALUE))
                                        .chain(
                                            recp.to_le_bits().iter().by_vals().take(L_ORCHARD_BASE),
                                        )
                                        .chain(
                                            esk.to_le_bits().iter().by_vals().take(L_ORCHARD_BASE),
                                        )
                                        .chain(
                                            rho.to_le_bits().iter().by_vals().take(L_ORCHARD_BASE),
                                        )
                                        .chain(
                                            psi.to_le_bits().iter().by_vals().take(L_ORCHARD_BASE),
                                        )
                                        .chain(
                                            pallas::Base::zero()
                                                .to_le_bits()
                                                .iter()
                                                .by_vals()
                                                .take(2),
                                        ),
                                    &rcm,
                                )
                                .unwrap()
                                .to_affine()
                        });
                    NonIdentityPoint::new(ecc_chip, lo.namespace(|| "witness cm"), point)?
                };
                // println!("cm: {:#?}", cm.extract_p());
                // println!("{:#?}", expected_cm.extract_p());
                // println!("cm == synth");
                cm.constrain_equal(lo.namespace(|| "cm == expected cm"), &expected_cm)
            }
        }

        let two_pow_254 = pallas::Base::from_u128(1 << 127).square();
        // Test different vs of `ak`, `nk`
        let circuits = [
            // `gd_x` = -1, `pkd_x` = -1 (these have to be x-coordinates of curve points)
            // `rho` = 0, `psi` = 0
            MyCircuit {
                nd: Value::known(pallas::Base::one()),
                v: Value::known(NoteValue::one()),
                fdi: Value::known(pallas::Base::one()),
                recp: Value::known(-pallas::Base::one()),
                esk: Value::known(pallas::Base::one()),
                rho: Value::known(pallas::Base::zero()),
                psi: Value::known(pallas::Base::zero()),
            },
            // `rho` = T_Q - 1, `psi` = T_Q - 1
            MyCircuit {
                nd: Value::known(-pallas::Base::one()),
                v: Value::known(NoteValue::one()),
                fdi: Value::known(pallas::Base::one()),
                recp: Value::known(-pallas::Base::one()),
                esk: Value::known(pallas::Base::one()),
                rho: Value::known(pallas::Base::from_u128(T_Q - 1)),
                psi: Value::known(pallas::Base::from_u128(T_Q - 1)),
            },
            // `rho` = T_Q, `psi` = T_Q
            MyCircuit {
                nd: Value::known(-pallas::Base::one()),
                v: Value::known(NoteValue::one()),
                fdi: Value::known(pallas::Base::one()),
                recp: Value::known(-pallas::Base::one()),
                esk: Value::known(pallas::Base::one()),
                rho: Value::known(pallas::Base::from_u128(T_Q)),
                psi: Value::known(pallas::Base::from_u128(T_Q)),
            },
            // `rho` = 2^127 - 1, `psi` = 2^127 - 1
            MyCircuit {
                nd: Value::known(-pallas::Base::one()),
                v: Value::known(NoteValue::one()),
                fdi: Value::known(pallas::Base::one()),
                recp: Value::known(-pallas::Base::one()),
                esk: Value::known(pallas::Base::one()),
                rho: Value::known(pallas::Base::from_u128((1 << 127) - 1)),
                psi: Value::known(pallas::Base::from_u128((1 << 127) - 1)),
            },
            // `rho` = 2^127, `psi` = 2^127
            MyCircuit {
                nd: Value::known(-pallas::Base::one()),
                v: Value::known(NoteValue::one()),
                fdi: Value::known(pallas::Base::one()),
                recp: Value::known(-pallas::Base::one()),
                esk: Value::known(pallas::Base::one()),
                rho: Value::known(pallas::Base::from_u128(1 << 127)),
                psi: Value::known(pallas::Base::from_u128(1 << 127)),
            },
            // `rho` = 2^254 - 1, `psi` = 2^254 - 1
            MyCircuit {
                rho: Value::known(two_pow_254 - pallas::Base::one()),
                psi: Value::known(two_pow_254 - pallas::Base::one()),
                nd: Value::known(-pallas::Base::one()),
                v: Value::known(NoteValue::one()),
                fdi: Value::known(pallas::Base::one()),
                recp: Value::known(-pallas::Base::one()),
                esk: Value::known(pallas::Base::one()),
            },
            // `rho` = 2^254, `psi` = 2^254
            MyCircuit {
                rho: Value::known(two_pow_254),
                psi: Value::known(two_pow_254),
                nd: Value::known(-pallas::Base::one()),
                v: Value::known(NoteValue::one()),
                fdi: Value::known(pallas::Base::one()),
                recp: Value::known(-pallas::Base::one()),
                esk: Value::known(pallas::Base::one()),
            },
        ];

        for circuit in circuits.iter() {
            let prover = MockProver::<pallas::Base>::run(11, circuit, vec![]).unwrap();
            #[cfg(feature = "dev-graph")]
            {
                use plotters::prelude::*;
                let root = BitMapBackend::new("note-commit.png", (1024, 768)).into_drawing_area();
                halo2_proofs::dev::CircuitLayout::default()
                    // You can optionally render only a section of the circuit.
                    .view_width(0..33)
                    .view_height(0..1024)
                    .render(13, circuit, &root) // 13 is the k value (number of rows)
                    .unwrap();
            }
            assert_eq!(prover.verify(), Ok(()));
        }
    }
}
