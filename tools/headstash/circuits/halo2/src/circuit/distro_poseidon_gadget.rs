//! In-circuit Poseidon-v1 gadgets for the **public inclusion / distro** Merkle set.
//!
//! Replaces Sinsemilla leaf + MerkleCRH for genesis eligibility (ADR-POSEIDON-DISTRO-TREE).
//! Pure off-circuit SSOT: [`crate::distro_poseidon`].
//!
//! Private note commitment: [`crate::circuit::note_poseidon_gadget`] (Poseidon-v1).

use halo2_gadgets::{
    poseidon::{
        primitives::{ConstantLength, P128Pow5T3},
        Hash as PoseidonHash, Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig,
    },
    utilities::cond_swap::{CondSwapChip, CondSwapInstructions},
};
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, Value},
    plonk::{Advice, Column, Error},
};
use pasta_curves::pallas;

use crate::constants::MERKLE_DEPTH_ORCHARD;
use crate::distro_poseidon::{distro_crh_tag, distro_leaf_tag};

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

/// Poseidon-v1 public inclusion **leaf**.
///
/// ```text
/// Poseidon^P128Pow5T3 / ConstantLength<7>(
///   tag_leaf, epk_x, epk_y, nd, v, fdi, rseed_com
/// )
/// ```
///
/// Matches [`crate::distro_poseidon::poseidon_distro_leaf`]. Full field elements
/// (including full `epk_y`), not Sinsemilla bit packing.
#[allow(clippy::too_many_arguments)]
pub fn derive_leaf_poseidon(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    advice: Column<Advice>,
    epk_x: AssignedCell<pallas::Base, pallas::Base>,
    epk_y: AssignedCell<pallas::Base, pallas::Base>,
    nd: AssignedCell<pallas::Base, pallas::Base>,
    v: AssignedCell<pallas::Base, pallas::Base>,
    fdi: AssignedCell<pallas::Base, pallas::Base>,
    rseed_com: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    let tag = assign_constant(
        layouter.namespace(|| "leaf personalization tag"),
        advice,
        "tag_leaf",
        distro_leaf_tag(),
    )?;

    let poseidon_chip = PoseidonChip::construct(poseidon_config.clone());
    let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<7>, 3, 2>::init(
        poseidon_chip,
        layouter.namespace(|| "Poseidon leaf init"),
    )?;
    hasher.hash(
        layouter.namespace(|| "Poseidon distro leaf"),
        [tag, epk_x, epk_y, nd, v, fdi, rseed_com],
    )
}

/// One Poseidon-v1 Merkle CRH step.
pub fn poseidon_merkle_crh(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    advice: Column<Advice>,
    layer: u32,
    left: AssignedCell<pallas::Base, pallas::Base>,
    right: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    let tag = assign_constant(
        layouter.namespace(|| format!("crh tag layer {layer}")),
        advice,
        "tag_crh",
        distro_crh_tag(),
    )?;
    let layer_cell = assign_constant(
        layouter.namespace(|| format!("layer {layer}")),
        advice,
        "layer",
        pallas::Base::from(u64::from(layer)),
    )?;

    let poseidon_chip = PoseidonChip::construct(poseidon_config.clone());
    let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<4>, 3, 2>::init(
        poseidon_chip,
        layouter.namespace(|| format!("Poseidon CRH init layer {layer}")),
    )?;
    hasher.hash(
        layouter.namespace(|| format!("Poseidon distro CRH layer {layer}")),
        [tag, layer_cell, left, right],
    )
}

/// Calculate the Poseidon-v1 distro root from leaf + auth path (depth 32).
///
/// Path ordering and position bits match Orchard / suite: index `0` at leaves;
/// bit `l` of `position` set means the current node is the **right** child.
pub fn calculate_distro_root_poseidon(
    mut layouter: impl Layouter<pallas::Base>,
    poseidon_config: &PoseidonConfig<pallas::Base, 3, 2>,
    cond_swap: CondSwapChip<pallas::Base>,
    advice: Column<Advice>,
    leaf: AssignedCell<pallas::Base, pallas::Base>,
    position: Value<u32>,
    path: Value<[pallas::Base; MERKLE_DEPTH_ORCHARD]>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    let mut node = leaf;

    for l in 0..MERKLE_DEPTH_ORCHARD {
        let sibling_val = path.map(|p| p[l]);
        let swap_bit = position.map(|pos| (pos >> l) & 1 == 1);

        // swap=false → (node, sibling); swap=true → (sibling, node)
        let (left, right) = cond_swap.swap(
            layouter.namespace(|| format!("order children layer {l}")),
            (node, sibling_val),
            swap_bit,
        )?;

        node = poseidon_merkle_crh(
            layouter.namespace(|| format!("Poseidon CRH layer {l}")),
            poseidon_config,
            advice,
            l as u32,
            left,
            right,
        )?;
    }

    Ok(node)
}

/// Pure/circuit consistency test helpers (used from unit tests).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::distro_poseidon::{
        poseidon_distro_crh, poseidon_distro_leaf, poseidon_distro_path_root,
    };
    use halo2_gadgets::poseidon::Pow5Config as PoseidonConfig;
    use halo2_proofs::{
        circuit::{SimpleFloorPlanner, Value},
        dev::MockProver,
        plonk::{Circuit, ConstraintSystem},
    };

    #[derive(Default)]
    struct LeafCircuit {
        epk_x: Value<pallas::Base>,
        epk_y: Value<pallas::Base>,
        nd: Value<pallas::Base>,
        v: Value<pallas::Base>,
        fdi: Value<pallas::Base>,
        rseed_com: Value<pallas::Base>,
    }

    #[derive(Clone)]
    struct LeafConfig {
        advice: [Column<Advice>; 6],
        poseidon: PoseidonConfig<pallas::Base, 3, 2>,
        instance: halo2_proofs::plonk::Column<halo2_proofs::plonk::Instance>,
    }

    impl Circuit<pallas::Base> for LeafCircuit {
        type Config = LeafConfig;
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
            LeafConfig {
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
            let epk_x = load(config.advice[0], self.epk_x)?;
            let epk_y = load(config.advice[1], self.epk_y)?;
            let nd = load(config.advice[2], self.nd)?;
            let v = load(config.advice[4], self.v)?;
            let fdi = load(config.advice[5], self.fdi)?;
            let rseed_com = load(config.advice[0], self.rseed_com)?;
            let leaf = derive_leaf_poseidon(
                layouter.namespace(|| "leaf"),
                &config.poseidon,
                config.advice[0],
                epk_x,
                epk_y,
                nd,
                v,
                fdi,
                rseed_com,
            )?;
            layouter.constrain_instance(leaf.cell(), config.instance, 0)?;
            Ok(())
        }
    }

    #[test]
    fn poseidon_leaf_gadget_matches_pure() {
        let epk_x = pallas::Base::from(11u64);
        let epk_y = pallas::Base::from(22u64);
        let nd = pallas::Base::from(33u64);
        let v = pallas::Base::from(44u64);
        let fdi = pallas::Base::from(55u64);
        let rseed_com = pallas::Base::from(66u64);
        let expected = poseidon_distro_leaf(epk_x, epk_y, nd, v, fdi, rseed_com);

        let circuit = LeafCircuit {
            epk_x: Value::known(epk_x),
            epk_y: Value::known(epk_y),
            nd: Value::known(nd),
            v: Value::known(v),
            fdi: Value::known(fdi),
            rseed_com: Value::known(rseed_com),
        };
        // k=11 is enough for a single ConstantLength<6> Poseidon
        let prover = MockProver::run(11, &circuit, vec![vec![expected]]).unwrap();
        prover.assert_satisfied();
    }

    #[test]
    fn poseidon_path_root_pure_order() {
        let leaf = pallas::Base::from(7u64);
        let sib0 = pallas::Base::from(8u64);
        let pos = 0u32; // left child
        let root = poseidon_distro_path_root(leaf, pos, &[sib0]);
        assert_eq!(root, poseidon_distro_crh(0, leaf, sib0));
        let root_r = poseidon_distro_path_root(leaf, 1u32, &[sib0]);
        assert_eq!(root_r, poseidon_distro_crh(0, sib0, leaf));
        assert_ne!(root, root_r);
    }
}
