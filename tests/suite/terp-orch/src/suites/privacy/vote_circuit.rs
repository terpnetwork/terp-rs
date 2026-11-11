//! Stored-circuit keys for the `vote_proof` cast slot, written with the cw-orch
//! `circuit_interface` derive in the layout the chain verifies:
//! `params || CS || VK || 80-byte footer` (prover 0, curve 2 VoteCommitment, k 11, i 11).
//! Spec: `~/grok-bot/privacy-products/zk-voting/SPEC.md` ("cast circuit keys").
//!
//! [`VoteProofCircuit`] is valargroup's real ZKP #2 (`voting-circuits` 0.10.0, as pinned by
//! vote-sdk) ported to the zakura Halo2 fork (`crates/voting-circuits-zakura`), so its CS
//! and VK serialize. [`VoteProofDevCircuit`] is the earlier public-input *shape* circuit,
//! kept only as a fast test fallback ([`VoteShapeCircuit`] proves nothing about ballots).
use anyhow::{Context, Result, anyhow, ensure};
use cosmwasm_std::Checksum;
use cw_orch::circuit_interface;
use cw_orch::core::circuits::circuit_interface_traits::{
    CircuitFooterSpec, CircuitSummary, CircuitSummaryLoader, CircuitUploadable,
};
use cw_orch::core::environment::ChainState;
use cw_orch::mock::Mock;
use cw_vote_ceremony::zk::{CAST_CURVE_ID, CAST_INSTANCES, CAST_K, CAST_PROVER_ID, cast_appstate_prefix};
use halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner, Value},
    pasta::{EqAffine, Fp, group::ff::PrimeField},
    plonk::{self, Advice, Circuit, Column, ConstraintSystem, Instance, ProvingKey, VerifyingKey},
    poly::commitment::Params,
    transcript::{Blake2bWrite, Challenge255},
};
use std::time::{Duration, Instant};
use zk_cosmwasm::{AnyInstance, AnyVerifyingKey, Proof};

/// Real valar `vote_proof` (`artifacts/vote_proof_vk.bin`, summary JSON beside it).
#[circuit_interface(id = "vote_proof", artifacts_dir = "artifacts")]
pub struct VoteProofCircuit;

impl<Chain: ChainState> CircuitFooterSpec for VoteProofCircuit<Chain> {
    fn summary_json_path() -> &'static str {
        concat!(env!("CARGO_MANIFEST_DIR"), "/artifacts/vote_proof_summary.json")
    }
}

/// Test fallback: the 11-input shape circuit (`artifacts/vote_proof_dev_vk.bin`).
#[circuit_interface(id = "vote_proof_dev", artifacts_dir = "artifacts")]
pub struct VoteProofDevCircuit;

impl<Chain: ChainState> CircuitFooterSpec for VoteProofDevCircuit<Chain> {
    fn summary_json_path() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/artifacts/vote_proof_dev_summary.json"
        )
    }
}

/// `cosmwasm_std::testing::register_test_circuit` verifier.
pub type HostVerifier =
    Box<dyn Fn(&[u8], &[u8]) -> Result<bool, cosmwasm_std::VerificationError>>;

/// A stored-circuit blob in the chain format plus how long its keygen took.
pub struct StoredCircuit {
    pub name: String,
    pub blob: Vec<u8>,
    pub keygen: Duration,
}

impl StoredCircuit {
    /// Write summary JSON and blob for `H` through the circuit interface
    /// (`generate_footer`, `vk_path`, `validate_combined_binary`). Vote keys keep CS and
    /// VK as one body (`cs_len` = body, `vk_len` = 0), as `VoteVerifyingKey` reads them.
    fn write<H>(
        params: &Params<EqAffine>,
        vk: &VerifyingKey<EqAffine>,
        lookups: bool,
        keygen: Duration,
    ) -> Result<Self>
    where
        H: CircuitUploadable + CircuitSummaryLoader,
    {
        ensure!(params.k() == CAST_K as u32, "params k {} != CAST_K {CAST_K}", params.k());
        let mut blob = vec![];
        params.write(&mut blob)?;
        let param_len = blob.len();
        vk.cs().write(&mut blob)?;
        vk.write(&mut blob)?;
        let sha = |b: &[u8]| -> [u8; 32] { *Checksum::generate(b).as_ref() };
        let cs = vk.cs();
        // Column/selector counts are informational; the footer binds k, i, lengths, hashes.
        let summary = CircuitSummary {
            circuit_name: H::circuit_name(),
            k: CAST_K,
            instance_count: CAST_INSTANCES as u8,
            curve_id: CAST_CURVE_ID,
            prover_id: CAST_PROVER_ID,
            num_fixed_columns: cs.num_fixed_columns() as u8,
            num_advice_columns: cs.num_advice_columns() as u8,
            num_instance_columns: cs.num_instance_columns() as u8,
            degree: cs.degree() as u8,
            has_lookups: lookups,
            param_len: param_len as u32,
            vk_len: 0,
            cs_len: (blob.len() - param_len) as u32,
            num_selectors: cs.num_selectors() as u32,
            num_gates: 0,
            param_checksum: sha(&blob[..param_len]),
            vk_checksum: sha(&blob[param_len..]),
        };
        std::fs::write(H::summary_json_path(), serde_json::to_vec_pretty(&summary)?)?;
        let footer =
            H::generate_footer(&H::load_summary().map_err(|e| anyhow!(e))?).map_err(|e| anyhow!(e))?;
        blob.extend_from_slice(&footer.to_bytes());
        let path = H::vk_path();
        std::fs::write(&path, &blob).with_context(|| format!("write {}", path.display()))?;
        H::validate_combined_binary(&path, &summary).map_err(|e| anyhow!(e))?;
        let c = Self { name: H::circuit_name(), blob, keygen };
        ensure!(
            c.circuit_key()?[..4] == cast_appstate_prefix(),
            "footer prefix is not the cast prefix"
        );
        Ok(c)
    }

    /// Real valar `vote_proof`: params + keygen from the port's process cache (the same
    /// keys `build_vote_proof_from_delegation` proves with), then the stored blob.
    pub fn vote_proof() -> Result<Self> {
        let t = Instant::now();
        let (params, _pk, vk) = voting_circuits::vote_proof::vote_proof_cached_keys()
            .map_err(|e| anyhow!("vote_proof keygen: {e}"))?;
        ensure!(voting_circuits::vote_proof::K == CAST_K as u32, "valar K != CAST_K");
        Self::write::<VoteProofCircuit<Mock>>(params, vk, true, t.elapsed())
    }

    /// 72-byte `param_key || vk_key` the contract pins (`cast_circuit_key`).
    pub fn circuit_key(&self) -> Result<[u8; 72]> {
        Ok(AnyVerifyingKey::try_from(self.blob.as_slice())
            .map_err(|e| anyhow!("blob: {e}"))?
            .circuit_key())
    }

    /// The verifier the in-process host runs for this zkid: the same Path A dispatch as
    /// wasmvm (`AnyVerifyingKey::try_from(blob)` + `AnyInstance` for its curve).
    pub fn host_verifier(&self) -> Result<HostVerifier> {
        let vk = AnyVerifyingKey::try_from(self.blob.as_slice()).map_err(|e| anyhow!("{e}"))?;
        Ok(Box::new(move |proof: &[u8], inst: &[u8]| {
            Ok(AnyInstance::try_from_bytes(vk.curve_id(), inst)
                .and_then(|i| vk.verify(&Proof::new(proof.to_vec()), &[i]))
                .is_ok())
        }))
    }
}

/// 11 public inputs exposed by copy constraints (test fallback circuit).
#[derive(Clone, Default)]
pub struct VoteShapeCircuit(pub Option<[Fp; CAST_INSTANCES]>);

impl Circuit<Fp> for VoteShapeCircuit {
    type Config = (Column<Advice>, Column<Instance>);
    type FloorPlanner = SimpleFloorPlanner;
    fn without_witnesses(&self) -> Self {
        Self(None)
    }
    fn configure(meta: &mut ConstraintSystem<Fp>) -> Self::Config {
        let (a, i) = (meta.advice_column(), meta.instance_column());
        meta.enable_equality(a);
        meta.enable_equality(i);
        (a, i)
    }
    fn synthesize(
        &self,
        (a, i): Self::Config,
        mut layouter: impl Layouter<Fp>,
    ) -> Result<(), plonk::Error> {
        let cells = layouter.assign_region(
            || "public inputs",
            |mut region| {
                (0..CAST_INSTANCES)
                    .map(|r| {
                        let v = self.0.map_or(Value::unknown(), |x| Value::known(x[r]));
                        region.assign_advice(|| "pi", a, r, || v)
                    })
                    .collect::<Result<Vec<_>, _>>()
            },
        )?;
        for (r, c) in cells.iter().enumerate() {
            layouter.constrain_instance(c.cell(), i, r)?;
        }
        Ok(())
    }
}

/// Fallback keys: shape-circuit proving key + its stored blob.
pub struct DevVoteKeys {
    pub params: Params<EqAffine>,
    pub pk: ProvingKey<EqAffine>,
    pub stored: StoredCircuit,
}

impl DevVoteKeys {
    /// Keygen at `CAST_K`. Params are reused from an existing blob.
    pub fn load_or_generate() -> Result<Self> {
        type H = VoteProofDevCircuit<Mock>;
        let t = Instant::now();
        let params = std::fs::read(H::vk_path())
            .ok()
            .and_then(|old| Params::<EqAffine>::read(&mut old.as_slice()).ok())
            .filter(|p| p.k() == CAST_K as u32)
            .unwrap_or_else(|| Params::new(CAST_K as u32));
        let empty = VoteShapeCircuit::default();
        let vk = plonk::keygen_vk(&params, &empty).map_err(|e| anyhow!("keygen_vk: {e:?}"))?;
        let pk = plonk::keygen_pk(&params, vk, &empty).map_err(|e| anyhow!("keygen_pk: {e:?}"))?;
        let stored = StoredCircuit::write::<H>(&params, pk.get_vk(), false, t.elapsed())?;
        Ok(Self { params, pk, stored })
    }

    /// Halo2 proof over 11 32-byte canonical Fp encodings (host order).
    pub fn prove(&self, instances: &[u8]) -> Result<Vec<u8>> {
        let x = scalars(instances)?;
        let mut t = Blake2bWrite::<_, EqAffine, Challenge255<EqAffine>>::init(vec![]);
        let rng = <rand::rngs::StdRng as rand::SeedableRng>::try_from_rng(&mut rand::rngs::SysRng)
            .map_err(|e| anyhow!("rng: {e}"))?;
        plonk::create_proof(
            &self.params,
            &self.pk,
            &[VoteShapeCircuit(Some(x))],
            &[&[&x[..]]],
            rng,
            &mut t,
        )
        .map_err(|e| anyhow!("create_proof: {e:?}"))?;
        Ok(t.finalize())
    }
}

fn scalars(instances: &[u8]) -> Result<[Fp; CAST_INSTANCES]> {
    ensure!(instances.len() == 32 * CAST_INSTANCES, "need 11 x 32 bytes");
    let v = instances
        .chunks_exact(32)
        .map(|c| Option::from(Fp::from_repr(c.try_into().expect("32"))).ok_or_else(|| anyhow!("non-canonical Fp")))
        .collect::<Result<Vec<_>>>()?;
    v.try_into().map_err(|_| anyhow!("instance count"))
}
