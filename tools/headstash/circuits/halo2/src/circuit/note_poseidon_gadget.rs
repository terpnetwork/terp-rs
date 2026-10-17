//! In-circuit Poseidon-v1 gadgets for **private note commitment**.
//!
//! Pure off-circuit SSOT: [`crate::note_poseidon`].
//! ADR: `docs/plans/spectrum/ADR-POSEIDON-NOTE-COMMIT.md` (option A lift).

use halo2_gadgets::{
    ecc::{
        chip::EccChip,
        FixedPointBaseField, Point,
    },
    poseidon::{
        primitives::{ConstantLength, P128Pow5T3},
        Hash as PoseidonHash, Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig,
    },
};
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, Value},
    plonk::{Advice, Column, Error},
};
use ff::PrimeField;
use pasta_curves::pallas;

use crate::constants::{OrchardFixedBases, OrchardFixedBasesBase};
use crate::note_poseidon::note_commit_tag;

/// Assign a field element and pin it. The prover cannot swap a domain tag.
fn assign_pinned(
    mut layouter: impl Layouter<pallas::Base>,
    column: Column<Advice>,
    label: &'static str,
    value: pallas::Base,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    layouter.assign_region(
        || label,
        |mut region| {
            let cell = region.assign_advice(|| label, column, 0, || Value::known(value))?;
            region.constrain_constant(cell.cell(), value)?;
            Ok(cell)
        },
    )
}

fn poseidon_hash<const N: usize>(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    label: &'static str,
    message: [AssignedCell<pallas::Base, pallas::Base>; N],
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    let poseidon_chip = PoseidonChip::construct(poseidon_config.clone());
    let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<N>, 3, 2>::init(
        poseidon_chip,
        layouter.namespace(|| label),
    )?;
    hasher.hash(layouter.namespace(|| label), message)
}

/// Bind the one-time nullifier inputs to a 32-byte note random string.
///
/// ```text
/// rho = rseed_lo
/// psi = rseed_hi
/// rcm = rseed_lo + rseed_hi
/// nk  = Poseidon(DST_HKDF, rseed_lo, rseed_hi)
/// ```
///
/// The halves are the secret. The public leaf still hides them behind
/// `rseed_com`. The eligible key is not an input: ownership is the signature.
#[allow(clippy::too_many_arguments)]
pub(in crate::circuit) fn bind_hiding_nullifier_inputs(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    advice: Column<Advice>,
    add_chip: &crate::circuit::gadget::add_chip::AddChip,
    rseed_lo: AssignedCell<pallas::Base, pallas::Base>,
    rseed_hi: AssignedCell<pallas::Base, pallas::Base>,
    rho: &AssignedCell<pallas::Base, pallas::Base>,
    psi: &AssignedCell<pallas::Base, pallas::Base>,
    rcm_base: &AssignedCell<pallas::Base, pallas::Base>,
    nk: &AssignedCell<pallas::Base, pallas::Base>,
) -> Result<(), Error> {
    use crate::circuit::gadget::AddInstruction;
    use crate::constants::DST_HKDF;
    layouter.assign_region(
        || "rho = rseed_lo",
        |mut region| region.constrain_equal(rho.cell(), rseed_lo.cell()),
    )?;
    layouter.assign_region(
        || "psi = rseed_hi",
        |mut region| region.constrain_equal(psi.cell(), rseed_hi.cell()),
    )?;
    let rcm_sum = add_chip.add(
        layouter.namespace(|| "rcm = rseed_lo + rseed_hi"),
        &rseed_lo,
        &rseed_hi,
    )?;
    layouter.assign_region(
        || "rcm bound",
        |mut region| region.constrain_equal(rcm_base.cell(), rcm_sum.cell()),
    )?;

    let nk_tag = assign_pinned(
        layouter.namespace(|| "nk dst"),
        advice,
        "dst_nk",
        pallas::Base::from_repr(DST_HKDF).expect("DST_HKDF is canonical"),
    )?;
    let nk_prf = poseidon_hash(
        layouter.namespace(|| "nk prf"),
        poseidon_config,
        "nk",
        [nk_tag, rseed_lo, rseed_hi],
    )?;
    layouter.assign_region(
        || "nk bound",
        |mut region| region.constrain_equal(nk.cell(), nk_prf.cell()),
    )
}

/// Hiding commitment of the note random string, published inside the distro leaf.
pub fn derive_rseed_commitment(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    advice: Column<Advice>,
    rseed_lo: AssignedCell<pallas::Base, pallas::Base>,
    rseed_hi: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    use crate::claim_auth::DST_RSEED;
    use crate::note_poseidon::personalization_to_fp;
    let tag = assign_pinned(
        layouter.namespace(|| "rseed dst"),
        advice,
        "dst_rseed",
        personalization_to_fp(DST_RSEED),
    )?;
    poseidon_hash(
        layouter.namespace(|| "rseed com"),
        poseidon_config,
        "rseed_com",
        [tag, rseed_lo, rseed_hi],
    )
}

/// Assign a fixed field element as an advice cell (constant witness).
fn assign_constant(
    mut layouter: impl Layouter<pallas::Base>,
    column: Column<Advice>,
    label: &str,
    value: pallas::Base,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    layouter.assign_region(
        || label,
        |mut region| region.assign_advice(|| label, column, 0, || Value::known(value)),
    )
}

/// Poseidon-v1 private note commitment digest `cmx`.
///
/// ```text
/// Poseidon^P128Pow5T3 / ConstantLength<9>(
///   tag_note_commit, nd, v, fdi, recp, esk, rho, psi, rcm_base
/// )
/// ```
///
/// Matches [`crate::note_poseidon::poseidon_note_cmx`]. Tag is assigned as a fixed
/// constant (prover cannot choose the domain).
#[allow(clippy::too_many_arguments)]
pub fn derive_note_cmx_poseidon(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    advice: Column<Advice>,
    nd: AssignedCell<pallas::Base, pallas::Base>,
    v: AssignedCell<pallas::Base, pallas::Base>,
    fdi: AssignedCell<pallas::Base, pallas::Base>,
    recp: AssignedCell<pallas::Base, pallas::Base>,
    esk: AssignedCell<pallas::Base, pallas::Base>,
    rho: AssignedCell<pallas::Base, pallas::Base>,
    psi: AssignedCell<pallas::Base, pallas::Base>,
    rcm_base: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    let tag = assign_constant(
        layouter.namespace(|| "note-commit personalization tag"),
        advice,
        "tag_note_commit",
        note_commit_tag(),
    )?;

    let poseidon_chip = PoseidonChip::construct(poseidon_config.clone());
    let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<9>, 3, 2>::init(
        poseidon_chip,
        layouter.namespace(|| "Poseidon note-commit init"),
    )?;
    hasher.hash(
        layouter.namespace(|| "Poseidon note cmx"),
        [tag, nd, v, fdi, recp, esk, rho, psi, rcm_base],
    )
}

/// Lift `cmx` to `cm_point = [cmx] · NoteCommitR` (ADR option A).
pub fn lift_note_cmx_gadget(
    mut layouter: impl Layouter<pallas::Base>,
    ecc_chip: EccChip<OrchardFixedBases>,
    cmx: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<Point<pallas::Affine, EccChip<OrchardFixedBases>>, Error> {
    let note_commit_r =
        FixedPointBaseField::from_inner(ecc_chip, OrchardFixedBasesBase::NoteCommitR);
    note_commit_r.mul(
        layouter.namespace(|| "[cmx] NoteCommitR"),
        cmx,
    )
}

/// Full private note commitment: Poseidon `cmx` + lift to point.
///
/// Returns `(cm_point, cmx_cell)`.
#[allow(clippy::too_many_arguments)]
pub fn note_commit_poseidon(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    ecc_chip: EccChip<OrchardFixedBases>,
    advice: Column<Advice>,
    nd: AssignedCell<pallas::Base, pallas::Base>,
    v: AssignedCell<pallas::Base, pallas::Base>,
    fdi: AssignedCell<pallas::Base, pallas::Base>,
    recp: AssignedCell<pallas::Base, pallas::Base>,
    esk: AssignedCell<pallas::Base, pallas::Base>,
    rho: AssignedCell<pallas::Base, pallas::Base>,
    psi: AssignedCell<pallas::Base, pallas::Base>,
    rcm_base: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<
    (
        Point<pallas::Affine, EccChip<OrchardFixedBases>>,
        AssignedCell<pallas::Base, pallas::Base>,
    ),
    Error,
> {
    let cmx = derive_note_cmx_poseidon(
        layouter.namespace(|| "Poseidon note cmx"),
        poseidon_config,
        advice,
        nd,
        v,
        fdi,
        recp,
        esk,
        rho,
        psi,
        rcm_base,
    )?;
    let cm_point = lift_note_cmx_gadget(
        layouter.namespace(|| "lift cmx to NoteCommitR"),
        ecc_chip,
        cmx.clone(),
    )?;
    Ok((cm_point, cmx))
}

/// Pure/circuit consistency tests for Poseidon note `cmx` (no ECC lift tables).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::note_poseidon::{poseidon_note_cmx, rcm_to_base};
    use halo2_proofs::{
        circuit::{SimpleFloorPlanner, Value},
        dev::MockProver,
        plonk::{Circuit, ConstraintSystem},
    };

    #[derive(Default)]
    struct NoteCmxCircuit {
        nd: Value<pallas::Base>,
        v: Value<pallas::Base>,
        fdi: Value<pallas::Base>,
        recp: Value<pallas::Base>,
        esk: Value<pallas::Base>,
        rho: Value<pallas::Base>,
        psi: Value<pallas::Base>,
        rcm_base: Value<pallas::Base>,
    }

    #[derive(Clone)]
    struct NoteCmxConfig {
        advice: [Column<Advice>; 10],
        poseidon: PoseidonConfig<pallas::Base, 3, 2>,
        instance: halo2_proofs::plonk::Column<halo2_proofs::plonk::Instance>,
    }

    impl Circuit<pallas::Base> for NoteCmxCircuit {
        type Config = NoteCmxConfig;
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            Self::default()
        }

        fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
            let instance = meta.instance_column();
            meta.enable_equality(instance);
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
            for a in &advices {
                meta.enable_equality(*a);
            }
            let rc_a = [
                meta.fixed_column(),
                meta.fixed_column(),
                meta.fixed_column(),
            ];
            let rc_b = [
                meta.fixed_column(),
                meta.fixed_column(),
                meta.fixed_column(),
            ];
            meta.enable_constant(rc_b[0]);
            let poseidon = PoseidonChip::configure::<P128Pow5T3>(
                meta,
                advices[0..3].try_into().unwrap(),
                advices[3],
                rc_a,
                rc_b,
            );
            NoteCmxConfig {
                advice: advices,
                poseidon,
                instance,
            }
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<pallas::Base>,
        ) -> Result<(), Error> {
            let mut load = |col: Column<Advice>, val: Value<pallas::Base>| {
                layouter.assign_region(
                    || "load",
                    |mut region| region.assign_advice(|| "v", col, 0, || val),
                )
            };
            let nd = load(config.advice[0], self.nd)?;
            let v = load(config.advice[1], self.v)?;
            let fdi = load(config.advice[2], self.fdi)?;
            let recp = load(config.advice[3], self.recp)?;
            let esk = load(config.advice[4], self.esk)?;
            let rho = load(config.advice[5], self.rho)?;
            let psi = load(config.advice[6], self.psi)?;
            let rcm_base = load(config.advice[7], self.rcm_base)?;

            let cmx = derive_note_cmx_poseidon(
                layouter.namespace(|| "note cmx"),
                &config.poseidon,
                config.advice[0],
                nd,
                v,
                fdi,
                recp,
                esk,
                rho,
                psi,
                rcm_base,
            )?;
            layouter.constrain_instance(cmx.cell(), config.instance, 0)?;
            Ok(())
        }
    }

    #[test]
    fn poseidon_note_cmx_gadget_matches_pure() {
        let nd = pallas::Base::from(1u64);
        let v = pallas::Base::from(2u64);
        let fdi = pallas::Base::from(3u64);
        let recp = pallas::Base::from(4u64);
        let esk = pallas::Base::from(5u64);
        let rho = pallas::Base::from(6u64);
        let psi = pallas::Base::from(7u64);
        let rcm_base = rcm_to_base(pasta_curves::pallas::Scalar::from(8u64));
        let expected = poseidon_note_cmx(nd, v, fdi, recp, esk, rho, psi, rcm_base);

        let circuit = NoteCmxCircuit {
            nd: Value::known(nd),
            v: Value::known(v),
            fdi: Value::known(fdi),
            recp: Value::known(recp),
            esk: Value::known(esk),
            rho: Value::known(rho),
            psi: Value::known(psi),
            rcm_base: Value::known(rcm_base),
        };
        // k enough for a single ConstantLength<9> Poseidon
        let prover = MockProver::run(11, &circuit, vec![vec![expected]]).unwrap();
        prover.assert_satisfied();
    }
}
