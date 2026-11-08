//! Two tiny Pasta circuits for the registry test.
//!
//! Same transcript as the epoch prover (Blake2b, IPA, one instance column).
//! K and the constraint system differ, so each blob is a different key.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use ff::{Field, PrimeField};
use halo2_proofs::{
    circuit::{floor_planner::V1, Layouter, Value},
    plonk::{self, Advice, Circuit, Column, ConstraintSystem, Expression, Instance, Selector},
    poly::{commitment::Params, Rotation},
    transcript::Blake2bWrite,
};
use pasta_curves::{pallas, vesta};

use crate::circuit::pasta_circuit_blob;

/// Blob, proof, and the 32-byte public limb the host must see.
#[derive(Clone)]
pub struct ToyHostProof {
    pub blob: Vec<u8>,
    pub proof: Vec<u8>,
    pub instances: Vec<u8>,
}

impl fmt::Debug for ToyHostProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToyHostProof")
            .field("blob_len", &self.blob.len())
            .field("proof_len", &self.proof.len())
            .field("instances_len", &self.instances.len())
            .finish()
    }
}

#[derive(Clone, Debug)]
struct EqConfig {
    advice: Column<Advice>,
    instance: Column<Instance>,
}

#[derive(Clone, Debug)]
struct EqCircuit {
    secret: Value<pallas::Base>,
}

impl Default for EqCircuit {
    fn default() -> Self {
        Self {
            secret: Value::unknown(),
        }
    }
}

impl Circuit<pallas::Base> for EqCircuit {
    type Config = EqConfig;
    type FloorPlanner = V1;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        let advice = meta.advice_column();
        let instance = meta.instance_column();
        meta.enable_equality(advice);
        meta.enable_equality(instance);
        EqConfig { advice, instance }
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), plonk::Error> {
        let cell = layouter.assign_region(
            || "eq",
            |mut region| region.assign_advice(|| "s", config.advice, 0, || self.secret),
        )?;
        layouter.constrain_instance(cell.cell(), config.instance, 0)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct OffConfig {
    advice: Column<Advice>,
    instance: Column<Instance>,
    selector: Selector,
}

#[derive(Clone, Debug)]
struct OffCircuit {
    secret: Value<pallas::Base>,
}

impl Default for OffCircuit {
    fn default() -> Self {
        Self {
            secret: Value::unknown(),
        }
    }
}

impl Circuit<pallas::Base> for OffCircuit {
    type Config = OffConfig;
    type FloorPlanner = V1;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        let advice = meta.advice_column();
        let instance = meta.instance_column();
        let selector = meta.selector();
        meta.enable_equality(advice);
        meta.enable_equality(instance);
        meta.create_gate("off", |meta| {
            let q = meta.query_selector(selector);
            let cur = meta.query_advice(advice, Rotation::cur());
            let nxt = meta.query_advice(advice, Rotation::next());
            vec![q * (nxt - cur - Expression::Constant(pallas::Base::ONE))]
        });
        OffConfig {
            advice,
            instance,
            selector,
        }
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), plonk::Error> {
        let plus = self.secret.map(|s| s + pallas::Base::ONE);
        let cell = layouter.assign_region(
            || "off",
            |mut region| {
                config.selector.enable(&mut region, 0)?;
                region.assign_advice(|| "s", config.advice, 0, || self.secret)?;
                region.assign_advice(|| "p", config.advice, 1, || plus)
            },
        )?;
        layouter.constrain_instance(cell.cell(), config.instance, 0)?;
        Ok(())
    }
}

fn prove_one<C>(k: u32, circuit: C, empty: C, public: pallas::Base) -> Result<ToyHostProof, String>
where
    C: Circuit<pallas::Base> + Clone + Sync,
    C::Config: Send,
{
    let params = Params::<vesta::Affine>::new(k);
    let vk = plonk::keygen_vk(&params, &empty).map_err(|_| "keygen")?;
    let pk = plonk::keygen_pk(&params, vk, &empty).map_err(|_| "keygen")?;
    let blob = pasta_circuit_blob(&params, pk.get_vk(), k as u8, 1)?;
    let values = [public];
    let columns: [&[pallas::Base]; 1] = [&values];
    let per_circuit: [&[&[pallas::Base]]; 1] = [&columns];
    let mut transcript = Blake2bWrite::<_, vesta::Affine, _>::init(Vec::new());
    plonk::create_proof(
        &params,
        &pk,
        &[circuit],
        &per_circuit,
        &mut crate::os_rng(),
        &mut transcript,
    )
    .map_err(|_| "prove")?;
    let proof = transcript.finalize();
    let mut instances = Vec::with_capacity(32);
    instances.extend_from_slice(public.to_repr().as_ref());
    Ok(ToyHostProof {
        blob,
        proof,
        instances,
    })
}

/// Equality circuit at `k`. The public limb is `secret`.
pub fn toy_vk_proof(k: u32, secret: u64) -> Result<ToyHostProof, String> {
    if !(8..=16).contains(&k) {
        return Err("k".into());
    }
    let secret_fp = pallas::Base::from(secret);
    prove_one(
        k,
        EqCircuit {
            secret: Value::known(secret_fp),
        },
        EqCircuit::default(),
        secret_fp,
    )
}

/// Offset circuit at `k`. The public limb is `secret + 1`.
pub fn toy_vk_proof_offset(k: u32, secret: u64) -> Result<ToyHostProof, String> {
    if !(8..=16).contains(&k) {
        return Err("k".into());
    }
    let secret_fp = pallas::Base::from(secret);
    let public = secret_fp + pallas::Base::ONE;
    prove_one(
        k,
        OffCircuit {
            secret: Value::known(secret_fp),
        },
        OffCircuit::default(),
        public,
    )
}
