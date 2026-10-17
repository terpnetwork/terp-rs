//! The Orchard Action circuit implementation.
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
    string::ToString,
};

use alloc::vec::Vec;

use cosmwasm_std::Checksum;
#[cfg(feature = "interface")]
use cw_orch::anyhow::{self, anyhow};
// Re-export types needed for test circuits
pub use gadget::assign_free_advice;
pub use gadget::secp256k1_chip::{
    CrtInteger, FpChip, FpConfig, FpInstructions, Secp256k1Chip, Secp256k1Config, Secp256k1Fp,
    Secp256k1FpChip, Secp256k1Fq, Secp256k1FqChip,
};

use group::Curve;
use halo2_proofs::{
    circuit::{floor_planner, Layouter, Value},
    plonk::{
        self, Advice, BatchVerifier, Column, Instance as InstanceColumn, SingleVerifier,
        TableColumn,
    },
    transcript::{Blake2bRead, Blake2bWrite},
};

use pasta_curves::{pallas, vesta};
use rand_core::RngCore;
use tracing::info;

use self::gadget::add_chip::{AddChip, AddConfig};
use crate::{
    address::RecpAddr,
    builder::SpendInfo,
    constants::{OrchardFixedBases, MERKLE_DEPTH_ORCHARD},
    keys::NullifierDerivingKey,
    note::{
        commitment::{NoteCommitTrapdoor, NoteCommitment},
        nullifier::Nullifier,
        ExtractedNoteCommitment, Note, Rho,
    },
    tree::{Anchor, MerkleHashOrchard},
    value::{NoteDenom, NoteValue},
};
use ff::{Field, PrimeField};
use halo2_gadgets::{
    ecc::{
        chip::{EccChip, EccConfig},
        Point,
    },
    poseidon::{primitives as poseidon, Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig},
    utilities::{
        cond_swap::{CondSwapChip, CondSwapConfig},
        lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig},
    },
};

mod commit_ivk;
pub mod distro_poseidon_gadget;
pub mod gadget;
pub mod headstash_merkle_tree;
mod note_commit;
pub mod note_poseidon_gadget;
#[cfg(test)]
mod note_commit_bit_tests;

pub use crate::Proof;

/// Row budget. One GLV mul fit in K=18. Personal-sign verify is two
/// (fixed base and variable base), which does not.
pub(crate) const K: u32 = 19;

// Absolute offsets for public inputs.
const ANCHOR: usize = 0;
const HS_ND: usize = 1;
const HS_V: usize = 2;
const RECP: usize = 3;
const NF_OLD: usize = 4;
const CMX: usize = 5;
/// Low and high 128-bit halves of the public EIP-191 challenge scalar.
const E_LO: usize = 6;
const E_HI: usize = 7;
// const RK_X: usize = 4;
// const RK_Y: usize = 5;
// const ENABLE_SPEND: usize = 7;
// const ENABLE_OUTPUT: usize = 8;

/// Configuration needed to use the Orchard Action circuit.
#[derive(Clone, Debug)]
pub struct Config {
    primary: Column<InstanceColumn>,
    advices: [Column<Advice>; 10],
    add_config: AddConfig,
    ecc_config: EccConfig<OrchardFixedBases>,
    secp256k1: Secp256k1Config,
    poseidon_config: PoseidonConfig<pallas::Base, 3, 2>,
    /// Conditional swap for Poseidon-v1 distro Merkle path ordering.
    cond_swap_config: CondSwapConfig,
    /// Shared 10-bit lookup table for ECC / secp256k1 range checks (not Sinsemilla).
    range_table: TableColumn,
}

/// The Orchard Action circuit.
#[derive(Clone, Debug, Default)]
pub struct Circuit {
    pub(crate) path: Value<[MerkleHashOrchard; MERKLE_DEPTH_ORCHARD]>,
    pub(crate) pos: Value<u32>,
    /// EIP-191 challenge, and the `(r, s)` of the personal_sign over it.
    pub(crate) sig_e: Value<Secp256k1Fq>,
    pub(crate) sig_r: Value<Secp256k1Fq>,
    pub(crate) sig_s: Value<Secp256k1Fq>,
    pub(crate) epkx: Value<Secp256k1Fp>,
    pub(crate) epky: Value<Secp256k1Fp>,
    pub(crate) nk: Value<NullifierDerivingKey>,
    /// Low and high halves of the note's one-time 32-byte random string.
    pub(crate) rseed_lo: Value<pallas::Base>,
    pub(crate) rseed_hi: Value<pallas::Base>,
    pub(crate) fdi: Value<pallas::Base>,
    pub(crate) v: Value<NoteValue>,
    pub(crate) nd: Value<pallas::Base>,
    pub(crate) recp: Value<pallas::Base>,
    pub(crate) rho_old: Value<Rho>,
    pub(crate) psi_old: Value<pallas::Base>,
    pub(crate) rcm_old: Value<NoteCommitTrapdoor>,
    pub(crate) cm_old: Value<NoteCommitment>,
    // pub(crate) alpha: Value<pallas::Scalar>,
    // pub(crate) ak: Value<SpendValidatingKey>
    // pub(crate) rivk: Value<CommitIvkRandomness>,
    // pub(crate) rcv: Value<ValueCommitTrapdoor>,
}

impl From<Circuit> for zk_cosmwasm::CosmwasmCircuit<Circuit> {
    fn from(c: Circuit) -> Self {
        zk_cosmwasm::CosmwasmCircuit::new(c)
    }
}


/// Load [0, 2^{10}) into the shared lookup column used by ECC / secp256k1 range checks.
/// Product A no longer configures Sinsemilla, so this replaces `SinsemillaChip::load`.
fn load_kbit_range_table(
    table_idx: TableColumn,
    layouter: &mut impl Layouter<pallas::Base>,
) -> Result<(), plonk::Error> {
    const KBITS: usize = 10;
    layouter.assign_table(
        || "k-bit range table",
        |mut table| {
            for index in 0..(1 << KBITS) {
                table.assign_cell(
                    || "table_idx",
                    table_idx,
                    index,
                    || Value::known(pallas::Base::from(index as u64)),
                )?;
            }
            Ok(())
        },
    )
}

impl Circuit {
    /// This constructor is public to enable creation of custom builders.
    /// If you are not creating a custom builder, use [`Builder`] to compose
    /// and authorize a transaction.
    ///
    /// Constructs a `Circuit` from the following components:
    /// - `spend`: [`SpendInfo`] of the note spent in scope of the action
    /// - `output_note`: a note created in scope of the action
    /// - `alpha`: a scalar used for randomization of the action spend validating key
    /// - `rcv`: trapdoor for the action value commitment
    ///
    /// Returns `None` if the `rho` of the `output_note` is not equal
    /// to the nullifier of the spent note.
    ///
    /// [`SpendInfo`]: crate::builder::SpendInfo
    /// [`Builder`]: crate::builder::Builder
    pub fn from_action_context(
        spend: SpendInfo,
        output_note: Note,
        // alpha: pallas::Scalar,
        // rcv: ValueCommitTrapdoor,
    ) -> Option<Circuit> {
        (Rho::from_nf_old(spend.note.nullifier()) == output_note.rho())
            .then(|| Self::from_action_context_unchecked(spend, output_note))
    }
    /// This from_action_context_unchecked is from action without validation.
    pub fn from_action_context_unchecked(
        spend: SpendInfo,
        _output_note: Note,
        // alpha: pallas::Scalar,
        // rcv: ValueCommitTrapdoor,
    ) -> Circuit {
        let bound = spend.note.hiding_binding();
        let rho_old = bound.rho;
        let psi_old = bound.psi;
        let rcm_old = bound.rcm;
        let fdi = spend.note.fdi();
        let (epkx, epky) = spend.note.elig_sk().epk().xy();
        let recp = spend.note.recipient();
        let nd = spend.note.nd();

        // nk is the DST_HKDF PRF of the note string, not a free witness.
        let nk = bound.nk;

        Circuit {
            path: Value::known(spend.merkle_path.auth_path()),
            pos: Value::known(spend.merkle_path.position()),
            v: Value::known(spend.note.value()),
            rho_old: Value::known(rho_old),
            psi_old: Value::known(psi_old),
            rcm_old: Value::known(rcm_old),
            cm_old: Value::known(spend.note.commitment()),
            nk: Value::known(nk),
            rseed_lo: Value::known(bound.rseed_lo),
            rseed_hi: Value::known(bound.rseed_hi),
            sig_e: Value::unknown(),
            sig_r: Value::unknown(),
            sig_s: Value::unknown(),
            epkx: Value::known(gadget::secp256k1_chip::secp_fp_from_coord_be(&epkx)),
            epky: Value::known(gadget::secp256k1_chip::secp_fp_from_coord_be(&epky)),
            fdi: Value::known(fdi.into()),
            nd: Value::known(nd.to_fp()),
            recp: Value::known(recp.to_fp()),
        }
    }

    /// Fill `(e, r, s)` from a personal_sign of the instance prefix.
    ///
    /// The signed body is `keccak256` of the 168-byte instance with the
    /// challenge bytes still zero. `instance.e` becomes the reduced scalar.
    pub fn attach_personal_sign(&mut self, instance: &mut Instance, secret_be: &[u8; 32]) {
        use crate::claim_auth::{keccak256, sign_personal_claim};
        use crate::circuit::gadget::secp256k1_chip::secp_fq_from_secret_be;

        let prefix = instance.to_bytes();
        let body = keccak256(&prefix[..168]);
        let (e_be, r_be, s_be) = sign_personal_claim(secret_be, &body);
        instance.e = e_be;
        self.sig_e = Value::known(secp_fq_from_secret_be(&e_be));
        self.sig_r = Value::known(secp_fq_from_secret_be(&r_be));
        self.sig_s = Value::known(secp_fq_from_secret_be(&s_be));
    }
}

impl plonk::Circuit<pallas::Base> for Circuit {
    type Config = Config;
    type FloorPlanner = floor_planner::V1;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut plonk::ConstraintSystem<pallas::Base>) -> Self::Config {
        // Advice columns used in the Orchard circuit.
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

        // Orchard action gate (v_old/v_new/enable_spend/enable_output) is not
        // synthesized. Dropping its selector keeps those polynomials out of the CS.

        // Addition of two field elements.
        let add_config = AddChip::configure(meta, advices[7], advices[8], advices[6]);

        // Single 10-bit lookup column for ECC / secp256k1 range checks.
        let table_idx = meta.lookup_table_column();

        // Instance column used for public inputs
        let primary = meta.instance_column();
        meta.enable_equality(primary);

        // Permutation over all advice columns.
        for advice in advices.iter() {
            meta.enable_equality(*advice);
        }

        // Poseidon requires four advice columns, while ECC incomplete addition requires
        // six, so we could choose to configure them in parallel. However, we only use a
        // single Poseidon invocation, and we have the rows to accommodate it serially.
        // Instead, we reduce the proof size by sharing fixed columns between the ECC and
        // Poseidon chips.
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
        let rc_a = lagrange_coeffs[2..5].try_into().unwrap();
        let rc_b = lagrange_coeffs[5..8].try_into().unwrap();

        // Also use the first Lagrange coefficient column for loading global constants.
        // It's free real estate :)
        meta.enable_constant(lagrange_coeffs[0]);

        // We have a lot of free space in the right-most advice columns; use one of them
        // for all of our range checks.
        let range_check = LookupRangeCheckConfig::configure(meta, advices[9], table_idx);

        // Configuration for curve point operations.
        // This uses 10 advice columns and spans the whole circuit.
        let ecc_config =
            EccChip::<OrchardFixedBases>::configure(meta, advices, lagrange_coeffs, range_check);

        // Configuration for the Poseidon hash.
        let poseidon_config = PoseidonChip::configure::<poseidon::P128Pow5T3>(
            meta,
            // We place the state columns after the partial_sbox column so that the
            // pad-and-add region can be laid out more efficiently.
            advices[6..9].try_into().unwrap(),
            advices[5],
            rc_a,
            rc_b,
        );

        // Secp256k1 foreign-field arithmetic config (9 shared advice columns)
        let secp_advices: [halo2_proofs::plonk::Column<halo2_proofs::plonk::Advice>; 9] = [
            advices[0], advices[1], advices[2], advices[3], advices[4], advices[5], advices[6],
            advices[7], advices[8],
        ];
        let secp256k1 = Secp256k1Config::configure(meta, secp_advices, range_check.clone());

        // CondSwap for Poseidon-v1 distro Merkle path (node/sibling ordering).
        let cond_swap_config =
            CondSwapChip::configure(meta, advices[0..5].try_into().unwrap());

        Config {
            primary,
            advices,
            add_config,
            ecc_config,
            secp256k1,
            poseidon_config,
            cond_swap_config,
            range_table: table_idx,
        }
    }

    #[allow(non_snake_case)]
    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), plonk::Error> {
        // Load [0, 2^10) for ECC / secp256k1 lookup range checks.
        load_kbit_range_table(config.range_table, &mut layouter)?;

        // Construct the ECC chip.
        let ecc_chip = config.ecc_chip();

        // 1. Eligible key signed the public claim challenge. esk is not a witness.
        let secp256k1_chip = Secp256k1Chip::construct(config.secp256k1.clone());
        let (epk_x, epk_y, sig_e) = secp256k1_chip.prove_ecdsa_verify(
            layouter.namespace(|| "personal_sign"),
            self.sig_e,
            self.sig_r,
            self.sig_s,
            self.epkx,
            self.epky,
        )?;
        let e_lo = secp256k1_chip.pack_u128(
            layouter.namespace(|| "challenge lo"),
            &sig_e.truncation.limbs[0],
            &sig_e.truncation.limbs[1],
        )?;
        let e_hi = secp256k1_chip.pack_u128(
            layouter.namespace(|| "challenge hi"),
            &sig_e.truncation.limbs[2],
            &sig_e.truncation.limbs[3],
        )?;
        layouter.constrain_instance(e_lo.cell(), config.primary, E_LO)?;
        layouter.constrain_instance(e_hi.cell(), config.primary, E_HI)?;

        // Witness private inputs that are used across multiple checks.
        let (nd, v, fdi, recp, psi_old, rho_old, nk) = {
            // Witness nd.
            let nd = assign_free_advice(
                layouter.namespace(|| "witness nd"),
                config.advices[0],
                self.nd,
            )?;

            // Witness v.
            let v = assign_free_advice(
                layouter.namespace(|| "witness v"),
                config.advices[0],
                self.v,
            )?;

            // Witness fdi.
            let fdi = assign_free_advice(
                layouter.namespace(|| "witness fdi"),
                config.advices[0],
                self.fdi,
            )?;

            // Witness recp.
            let recp = assign_free_advice(
                layouter.namespace(|| "witness recp"),
                config.advices[0],
                self.recp,
            )?;
            // Witness psi_old
            let psi_old = assign_free_advice(
                layouter.namespace(|| "witness psi_old"),
                config.advices[0],
                self.psi_old,
            )?;

            // Witness rho_old
            let rho_old = assign_free_advice(
                layouter.namespace(|| "witness rho_old"),
                config.advices[0],
                self.rho_old.map(|rho| rho.into_inner()),
            )?;

            // Witness nk (Headstash: derived from esk+rho off-circuit; witnessed here).
            let nk = assign_free_advice(
                layouter.namespace(|| "witness nk"),
                config.advices[0],
                self.nk.map(|nk| nk.inner()),
            )?;

            (nd, v, fdi, recp, psi_old, rho_old, nk)
        };

        // Public instances must match the same cells used in Poseidon note + distro leaf.
        // Distro tree is public (unlike Orchard Sinsemilla cmx-in-tree); binding nd/v/recp
        // here is what makes a clearnet claim mint the allocation the proof opened.
        layouter.constrain_instance(nd.cell(), config.primary, HS_ND)?;
        layouter.constrain_instance(recp.cell(), config.primary, RECP)?;

        // --- Note commit + nullifier FIRST (Poseidon-v1 cmx + lift), before distro path.
        // Running 32 Poseidon CRH layers first was associated with normalize/Fixed
        // permutation failures in the composite circuit; keep Orchard-shaped order.
        let rcm_base = assign_free_advice(
            layouter.namespace(|| "witness rcm_base"),
            config.advices[0],
            self.rcm_old
                .as_ref()
                .map(|rcm_old| crate::note_poseidon::rcm_to_base(rcm_old.inner())),
        )?;

        // v is witnessed as NoteValue; poseidon path needs Base (u64).
        let v_for_cm = assign_free_advice(
            layouter.namespace(|| "witness v as Base for note commit"),
            config.advices[0],
            self.v.map(|nv| pallas::Base::from(nv.inner())),
        )?;
        layouter.assign_region(
            || "constrain v == v_for_cm (note commit)",
            |mut region| region.constrain_equal(v.cell(), v_for_cm.cell()),
        )?;
        layouter.constrain_instance(v_for_cm.cell(), config.primary, HS_V)?;

        let rseed_lo = assign_free_advice(
            layouter.namespace(|| "witness note random lo"),
            config.advices[0],
            self.rseed_lo,
        )?;
        let rseed_hi = assign_free_advice(
            layouter.namespace(|| "witness note random hi"),
            config.advices[0],
            self.rseed_hi,
        )?;
        // rho and psi are the note-random halves. rcm is their sum.
        // nk is a PRF of that string. rseed_com hides the halves in the leaf.
        let rseed_com = note_poseidon_gadget::derive_rseed_commitment(
            layouter.namespace(|| "rseed commitment"),
            &config.poseidon_config,
            config.advices[0],
            rseed_lo.clone(),
            rseed_hi.clone(),
        )?;
        note_poseidon_gadget::bind_hiding_nullifier_inputs(
            layouter.namespace(|| "bind hiding nullifier"),
            &config.poseidon_config,
            config.advices[0],
            &config.add_chip(),
            rseed_lo,
            rseed_hi,
            &rho_old,
            &psi_old,
            &rcm_base,
            &nk,
        )?;

        let esk_zero = layouter.assign_region(
            || "note commit does not take esk",
            |mut region| {
                let cell = region.assign_advice(
                    || "zero",
                    config.advices[0],
                    0,
                    || Value::known(pallas::Base::ZERO),
                )?;
                region.constrain_constant(cell.cell(), pallas::Base::ZERO)?;
                Ok(cell)
            },
        )?;
        let (cm_old, cmx_cell) = note_poseidon_gadget::note_commit_poseidon(
            layouter.namespace(|| "derive note commitment Poseidon-v1"),
            &config.poseidon_config,
            ecc_chip.clone(),
            config.advices[0],
            nd.clone(),
            v_for_cm,
            fdi.clone(),
            recp.clone(),
            esk_zero,
            rho_old.clone(),
            psi_old.clone(),
            rcm_base,
        )?;

        // Public CMX = Poseidon cmx (not extract_p of the lift).
        layouter.constrain_instance(cmx_cell.cell(), config.primary, CMX)?;

        let _nf_old = {
            let nf_old = gadget::derive_nullifier(
                layouter.namespace(|| "nf_old = DeriveNullifier_nk(rho_old, psi_old, cm)"),
                config.poseidon_chip(),
                config.add_chip(),
                ecc_chip.clone(),
                rho_old.clone(),
                &psi_old,
                &cm_old,
                nk.clone(),
            )?;
            layouter.constrain_instance(nf_old.inner().cell(), config.primary, NF_OLD)?;
            nf_old
        };

        // --- Genesis Poseidon-v1 inclusion (public distro tree) ---
        let v_base = assign_free_advice(
            layouter.namespace(|| "witness v as Base for distro leaf"),
            config.advices[0],
            self.v.map(|nv| pallas::Base::from(nv.inner())),
        )?;
        layouter.assign_region(
            || "constrain v == v_base",
            |mut region| region.constrain_equal(v.cell(), v_base.cell()),
        )?;

        let genesis_leaf = distro_poseidon_gadget::derive_leaf_poseidon(
            layouter.namespace(|| "derive genesis leaf Poseidon-v1"),
            &config.poseidon_config,
            config.advices[0],
            epk_x,
            epk_y,
            nd,
            v_base,
            fdi,
            rseed_com,
        )?;

        let path_vals = self
            .path
            .map(|typed_path| typed_path.map(|node| node.inner()));
        let genesis_root = distro_poseidon_gadget::calculate_distro_root_poseidon(
            layouter.namespace(|| "Genesis Poseidon-v1 merkle path"),
            &config.poseidon_config,
            CondSwapChip::construct(config.cond_swap_config.clone()),
            config.advices[0],
            genesis_leaf,
            self.pos,
            path_vals,
        )?;
        layouter.constrain_instance(genesis_root.cell(), config.primary, ANCHOR)?;

        Ok(())
    }
}

/// The verifying key for the Orchard Action circuit.
#[derive(Debug)]
pub struct VerifyingKey {
    /// params
    pub params: halo2_proofs::poly::commitment::Params<vesta::Affine>,
    /// vk
    pub vk: plonk::VerifyingKey<vesta::Affine>,
}

impl From<&ProvingKey> for VerifyingKey {
    fn from(pk: &ProvingKey) -> Self {
        Self {
            params: pk.params(),
            vk: pk.pk.get_vk().clone(),
        }
    }
}

impl VerifyingKey {
    /// Builds the verifying key.
    pub fn build() -> Self {
        let params = halo2_proofs::poly::commitment::Params::new(K);
        let circuit: Circuit = Default::default();
        let vk = plonk::keygen_vk(&params, &circuit).unwrap();
        VerifyingKey { params, vk }
    }
}

/// The proving key for the Orchard Action circuit.
#[derive(Debug)]
pub struct ProvingKey {
    params: halo2_proofs::poly::commitment::Params<vesta::Affine>,
    pk: plonk::ProvingKey<vesta::Affine>,
}

impl ProvingKey {
    /// Builds existing proving key
    pub fn new(
        pk: plonk::ProvingKey<vesta::Affine>,
        params: halo2_proofs::poly::commitment::Params<vesta::Affine>,
    ) -> Self {
        ProvingKey { params, pk }
    }

    pub fn build() -> Self {
        let params = halo2_proofs::poly::commitment::Params::new(K);
        let circuit: Circuit = Default::default();
        let vk = plonk::keygen_vk(&params, &circuit).expect("keygen_vk");
        let pk = plonk::keygen_pk(&params, vk, &circuit).expect("keygen_pk");
        Self::new(pk, params)
    }

    /// build and write the provingkey and verifying key, as defined by the terp-ADR that specifies how we serialize our proving keys for on-chain compatibility.
    /// path - the path to the artifacts directory.
    #[cfg(feature = "interface")]
    pub fn build_and_write(vkpath: PathBuf) -> cw_orch::anyhow::Result<Self> {
        let path = File::create(&vkpath).map_err(|e| cw_orch::anyhow::anyhow!(e))?;
        let mut vkw = BufWriter::new(path);

        let pk = Self::build();

        let mut buf = Vec::new();
        pk.params().write(&mut buf).expect("params serialization");
        let paramlen = buf.len();
        // keygen's CS is post-synthesis. A fresh configure() CS has fewer
        // fixed columns, and the VM rejects that blob.
        pk.vk().vk.cs().write(&mut buf)?;
        let cslen = buf.len() - paramlen;
        pk.vk().vk.write(&mut buf)?;
        let vklen = buf.len() - cslen - paramlen;

        // Separate checksums for the two independently-stored components.
        // The body layout is: [params][cs][vk] — param bytes are buf[0..paramlen],
        // vk+cs bytes are buf[paramlen..paramlen+cslen+vklen].
        let param_bytes = &buf[..paramlen];
        let vk_body_bytes = &buf[paramlen..paramlen + cslen + vklen];
        let param_hash: [u8; 32] = {
            let mut h = [0u8; 32];
            h.copy_from_slice(Checksum::generate(param_bytes).as_slice());
            h
        };
        let vk_hash: [u8; 32] = {
            let mut h = [0u8; 32];
            h.copy_from_slice(Checksum::generate(vk_body_bytes).as_slice());
            h
        };

        buf.extend_from_slice(
            &zk_cosmwasm::CircuitFooter::new(
                zk_cosmwasm::CircuitType::Plonkish,
                zk_cosmwasm::curves::CurveType::Pasta,
                K as u8,
                8,   // i — anchor, nd, v, recp, nf, cmx, challenge lo, challenge hi
                paramlen as u32,
                cslen as u32,
                vklen as u32,
                param_hash,
                vk_hash,
            )
            .to_bytes(),
        );

        vkw.write_all(&buf)?;
        vkw.flush()?;

        Ok(pk)
    }

    /// retrieve a clone of the vk
    pub fn vk(&self) -> VerifyingKey {
        VerifyingKey::from(self)
    }
    /// retrieve a clone of the params
    pub fn params(&self) -> halo2_proofs::poly::commitment::Params<vesta::Affine> {
        self.params.clone()
    }
}

/// Public inputs to the Headstash Action circuit.
#[derive(Clone, Debug)]
pub struct Instance {
    pub(crate) anchor: Anchor,
    pub(crate) nd: NoteDenom,
    pub(crate) v: NoteValue,
    pub(crate) nf: Nullifier,
    pub(crate) recp: RecpAddr,
    pub(crate) cmx: ExtractedNoteCommitment,
    /// Reduced EIP-191 challenge, 32 big-endian bytes. Not part of the signed prefix.
    pub e: [u8; 32],
}

impl Instance {
    /// Constructs an [`Instance`] from its constituent parts.
    ///
    /// This API can be used in combination with [`Proof::verify`] to build verification
    /// pipelines for many proofs, where you don't want to pass around the full bundle.
    /// Use [`Bundle::verify_proof`] instead if you have the full bundle.
    ///
    /// [`Bundle::verify_proof`]: crate::Bundle::verify_proof
    pub fn from_parts(
        anchor: Anchor,
        nd: NoteDenom,
        v: NoteValue,
        recp: RecpAddr,
        nf: Nullifier,
        cmx: ExtractedNoteCommitment,
        // rk: VerificationKey<SpendAuth>,
        // enable_spend: bool,
        // enable_output: bool,
    ) -> Self {
        Instance {
            anchor,
            nd,
            v,
            recp,
            nf,
            cmx,
            e: [0u8; 32],
            // rk,
            // enable_spend,
            // enable_output,
        }
    }

    /// Constructs an [`Instance`] from its constituent parts in bytes. Expected to have already been formatted for in-circuit use
    /// Anchor: 32 bytes
    /// Note Denom: 32 bytes
    /// Value: 8 bytes
    /// Nullifier: 32 bytes
    /// Recipient: 32 bytes
    /// Note Commitment: 32 bytes
    /// Challenge: 32 bytes
    /// Total: 200 bytes. The challenge is not part of the signed prefix.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        const THREETWO: usize = 32;
        const EIGHT: usize = 8;
        const TOTAL_SIZE: usize = (6 * THREETWO) + EIGHT;
        assert_eq!(bytes.len(), TOTAL_SIZE);
        let mut offset = 0;

        let anchor: &[u8; 32] = &bytes[offset..offset + THREETWO].try_into().expect("anchor");
        offset += THREETWO;

        let nd: &[u8; 32] = &bytes[offset..offset + THREETWO].try_into().expect("nd");
        offset += THREETWO;

        let v_bytes = &bytes[offset..offset + EIGHT];
        let v = u64::from_le_bytes(v_bytes.try_into().expect("v"));
        offset += EIGHT;

        let nf: &[u8; 32] = &bytes[offset..offset + THREETWO].try_into().expect("nf");
        offset += THREETWO;

        let recp = &bytes[offset..offset + THREETWO];
        offset += THREETWO;
        let cmx: &[u8; 32] = &bytes[offset..offset + THREETWO].try_into().expect("cmx");
        offset += THREETWO;
        let e: [u8; 32] = bytes[offset..offset + THREETWO].try_into().expect("e");

        Instance {
            anchor: Anchor::from_bytes(*anchor).expect("anchor"),
            nd: NoteDenom::from(*nd),
            v: NoteValue::from(v),
            nf: Nullifier::from_bytes(nf).expect("msg"),
            recp: RecpAddr::try_from(recp).expect(""),
            cmx: ExtractedNoteCommitment::from_bytes(cmx).expect(""),
            e,
        }
    }

    /// Constructs an  [Vec<u8>]  from an instance for serialization/deserialization.
    /// NOTE: we store ALL values as their out-of-circuit specs, NOT applying in circuit serialization for field comatibility.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(200);

        bytes.extend_from_slice(&self.anchor.to_bytes());
        bytes.extend_from_slice(&self.nd.to_fp().to_repr());
        bytes.extend_from_slice(&self.v.inner().to_le_bytes());
        bytes.extend_from_slice(&self.nf.to_bytes());
        bytes.extend_from_slice(&self.recp.to_canonical_bytes());
        bytes.extend_from_slice(&self.cmx.to_bytes());
        bytes.extend_from_slice(&self.e);

        bytes
    }

    /// 8x32-byte little-endian scalars for wasmvm `proof_instance_verify`.
    /// Same field elements as `to_halo2_instance` rows 0..8.
    pub fn to_vm_public_inputs(&self) -> Vec<u8> {
        crate::vm_instance::encode_vm_public_inputs(
            self.anchor.inner(),
            self.nd.to_fp(),
            self.v.inner(),
            self.recp.to_fp(),
            self.nf.0,
            self.cmx.inner(),
            self.e,
        )
    }
    /// Constructs the `[[vesta::Scalar; 9]; 1]` array representation from an instance for serialization/deserialization.
    pub fn to_halo2_instance(&self) -> [[vesta::Scalar; 9]; 1] {
        let mut instance = [vesta::Scalar::zero(); 9];
        instance[ANCHOR] = self.anchor.inner();
        instance[HS_ND] = self.nd.to_fp();
        instance[HS_V] = self.v.inner().into();
        instance[RECP] = self.recp.to_fp();
        instance[NF_OLD] = self.nf.0;
        instance[CMX] = self.cmx.inner();
        let e_lo = u128::from_be_bytes(self.e[16..32].try_into().expect("e lo"));
        let e_hi = u128::from_be_bytes(self.e[0..16].try_into().expect("e hi"));
        instance[E_LO] = pallas::Base::from_u128(e_lo);
        instance[E_HI] = pallas::Base::from_u128(e_hi);

        [instance]
    }
}

impl Proof {
    /// Creates a proof for the given circuits and instances.
    pub fn create(
        pk: &ProvingKey,
        circuits: &[Circuit],
        instances: &[Instance],
        mut rng: impl RngCore,
    ) -> Result<Self, plonk::Error> {
        let instances: Vec<_> = instances.iter().map(|i| i.to_halo2_instance()).collect();
        let instances: Vec<Vec<_>> = instances
            .iter()
            .map(|i| i.iter().map(|c| &c[..]).collect())
            .collect();
        let instances: Vec<_> = instances.iter().map(|i| &i[..]).collect();

        let mut transcript = Blake2bWrite::<_, vesta::Affine, _>::init(vec![]);
        plonk::create_proof(
            &pk.params,
            &pk.pk,
            circuits,
            &instances,
            &mut rng,
            &mut transcript,
        )?;
        Ok(Proof(transcript.finalize()))
    }

    /// Verifies this proof with the given instances.
    pub fn verify(&self, vk: &VerifyingKey, instances: &[Instance]) -> Result<(), plonk::Error> {
        let instances: Vec<_> = instances.iter().map(|i| i.to_halo2_instance()).collect();
        let instances: Vec<Vec<_>> = instances
            .iter()
            .map(|i| i.iter().map(|c| &c[..]).collect())
            .collect();
        let instances: Vec<_> = instances.iter().map(|i| &i[..]).collect();

        let strategy = SingleVerifier::new(&vk.params);
        let mut transcript = Blake2bRead::init(&self.0[..]);
        plonk::verify_proof(&vk.params, &vk.vk, strategy, &instances, &mut transcript)
    }

    /// Adds this proof to the given batch for verification with the given instances.
    ///
    /// Use this API if you want more control over how proof batches are processed. If you
    /// just want to batch-validate Orchard bundles, use [`bundle::BatchValidator`].
    ///
    /// [`bundle::BatchValidator`]: crate::bundle::BatchValidator
    pub fn add_to_batch(&self, batch: &mut BatchVerifier<vesta::Affine>, instances: Vec<Instance>) {
        let instances = instances
            .iter()
            .map(|i| {
                i.to_halo2_instance()
                    .into_iter()
                    .map(|c| c.into_iter().collect())
                    .collect()
            })
            .collect();

        batch.add_proof(instances, self.0.clone());
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::iter;

    use ff::PrimeField;
    use halo2_proofs::{circuit::Value, dev::MockProver};
    use pasta_curves::pallas;
    use rand_core::RngCore;
    use crate::os_rng;

    use super::{Circuit, Instance, Proof, ProvingKey, VerifyingKey, K};
    use crate::{
        circuit::gadget::secp256k1_chip::{secp_coord_be_to_pallas_base, secp_fp_from_coord_be},
        distro_poseidon::poseidon_distro_leaf,
        note::Note,
        tree::MerklePath,
    };

    /// Product A claim fixture: Poseidon-v1 distro leaf + path root, Poseidon note `cmx`.
    ///
    /// Matches synthesize: [`distro_poseidon_gadget::derive_leaf_poseidon`] +
    /// [`distro_poseidon_gadget::calculate_distro_root_poseidon`] and
    /// [`note_poseidon_gadget::note_commit_poseidon`].
    ///
    /// Sinsemilla leaf (`LEAF_PERSONALIZATION`) is recovery-only — not used here.
    fn generate_circuit_instance<R: RngCore>(mut rng: R) -> (Circuit, Instance) {
        let (_sk, _fvk, esk, spent_note) = Note::dummy(&mut rng, None);
        let (epkx_be, epky_be) = esk.epk().xy();
        // BE host encodings → LE halo2curves fields (matches prove_key_pairing / G load).
        let (epkx, epky) = (
            secp_fp_from_coord_be(&epkx_be),
            secp_fp_from_coord_be(&epky_be),
        );
        let epk_x_native: pallas::Base = secp_coord_be_to_pallas_base(&epkx_be);
        let epk_y_native: pallas::Base = secp_coord_be_to_pallas_base(&epky_be);
        let recp = spent_note.recipient();

        let bound = spent_note.hiding_binding();
        let nk = bound.nk;
        let nf = spent_note.nullifier();
        // Private note cmx: Poseidon CL9 (NoteCommitment::derive / note_poseidon SSOT).
        let cmx = spent_note.commitment().into();
        let nd = spent_note.nd();
        let v = spent_note.value();
        let path = MerklePath::dummy(&mut rng);
        let nd_pallas: pallas::Base = spent_note.nd().to_fp();
        let v_pallas: pallas::Base = pallas::Base::from(spent_note.value().inner());
        let fdi_pallas: pallas::Base = pallas::Base::from(spent_note.fdi());

        // Public eligibility leaf: Poseidon-v1 (full epk_y; empty pad is ZERO in tree helpers).
        // Distinct from private cmx — do not feed note commit into distro path.
        let distro_leaf = poseidon_distro_leaf(
            epk_x_native,
            epk_y_native,
            nd_pallas,
            v_pallas,
            fdi_pallas,
            crate::claim_auth::claim_rseed_com(bound.rseed_lo, bound.rseed_hi),
        );
        // Auth path is random siblings; root via Poseidon CRH so anchor matches
        // calculate_distro_root_poseidon in synthesize.
        let anchor = path.root_from_leaf(distro_leaf);

        let mut circuit = Circuit {
            path: Value::known(path.auth_path()),
            pos: Value::known(path.position()),
            nk: Value::known(nk),
            rseed_lo: Value::known(bound.rseed_lo),
            rseed_hi: Value::known(bound.rseed_hi),
            nd: Value::known(spent_note.nd().to_fp()),
            v: Value::known(spent_note.value()),
            fdi: Value::known(pallas::Base::from(spent_note.fdi())),
            recp: Value::known(recp.to_fp()),
            sig_e: Value::unknown(),
            sig_r: Value::unknown(),
            sig_s: Value::unknown(),
            epkx: Value::known(epkx),
            epky: Value::known(epky),
            rho_old: Value::known(bound.rho),
            psi_old: Value::known(bound.psi),
            rcm_old: Value::known(bound.rcm),
            cm_old: Value::known(spent_note.commitment()),
        };
        let mut instance = Instance {
            anchor,
            nd,
            v,
            recp,
            nf,
            cmx,
            e: [0u8; 32],
        };
        circuit.attach_personal_sign(&mut instance, &esk.secret_bytes());
        (circuit, instance)
    }

    // TODO: recast as a proptest
    #[test]
    fn round_trip() {
        let mut rng = os_rng();

        let (circuits, instances): (Vec<_>, Vec<_>) = iter::once(())
            .map(|()| generate_circuit_instance(&mut rng))
            .unzip();

        let vk = VerifyingKey::build();

        // Test that the pinned verification key (representing the circuit)
        // is as expected.
        {
            // panic!("{:#?}", vk.vk.pinned());
            // assert_eq!(
            //     format!("{:#?}\n", vk.vk.pinned()),
            //     // include_str!("circuit_description").replace("\r\n", "\n")
            // );
        }

        // Test that the proof size is as expected (Product A Poseidon distro path).
        let expected_proof_size = {
            let circuit_cost =
                halo2_proofs::dev::CircuitCost::<pasta_curves::vesta::Point, _>::measure(
                    K,
                    &circuits[0],
                );
            // Layout after unused Sinsemilla configure removal.
            assert_eq!(usize::from(circuit_cost.proof_size(1)), 4704);
            assert_eq!(usize::from(circuit_cost.proof_size(2)), 7840);
            usize::from(circuit_cost.proof_size(instances.len()))
        };

        for (circuit, instance) in circuits.iter().zip(instances.iter()) {
            assert_eq!(
                MockProver::run(
                    K,
                    circuit,
                    instance
                        .to_halo2_instance()
                        .iter()
                        .map(|p| p.to_vec())
                        .collect()
                )
                .unwrap()
                .verify(),
                Ok(())
            );
        }

        // Real proving keygen at K=18 is multi-minute; gate for CI / local opt-in.
        // Product A correctness gate is MockProver above.
        if std::env::var_os("HEADSTASH_CIRCUIT_FULL_PROVE").is_some() {
            let pk = ProvingKey::build();
            let proof = Proof::create(&pk, &circuits, &instances, &mut rng).unwrap();
            assert!(proof.verify(&vk, &instances).is_ok());
            assert_eq!(proof.0.len(), expected_proof_size);
        } else {
            let _ = (vk, expected_proof_size);
        }
    }

    // #[test]
    // fn serialized_proof_test_case() {
    //     use std::io::{Read, Write};

    //     let vk = VerifyingKey::build();

    //     fn write_test_case<W: Write>(
    //         mut w: W,
    //         instance: &Instance,
    //         proof: &Proof,
    //     ) -> std::io::Result<()> {
    //         w.write_all(&instance.anchor.to_bytes())?;
    //         w.write_all(&instance.nd.as_bytes())?;
    //         w.write_all(&instance.v.to_bytes())?;
    //         w.write_all(&instance.recp.to_canonical_bytes())?;
    //         w.write_all(&instance.nf.to_bytes())?;
    //         w.write_all(&instance.cmx.to_bytes())?;
    //         w.write_all(proof.as_ref())?;
    //         Ok(())
    //     }

    //     fn read_test_case<R: Read>(mut r: R) -> std::io::Result<(Instance, Proof)> {
    //         let read_8_bytes = |r: &mut R| {
    //             let mut ret = [0u8; 8];
    //             r.read_exact(&mut ret).unwrap();
    //             ret
    //         };
    //         let read_32_bytes = |r: &mut R| {
    //             let mut ret = [0u8; 32];
    //             r.read_exact(&mut ret).unwrap();
    //             ret
    //         };
    //         let _read_bool = |r: &mut R| {
    //             let mut byte = [0u8; 1];
    //             r.read_exact(&mut byte).unwrap();
    //             match byte {
    //                 [0] => false,
    //                 [1] => true,
    //                 _ => panic!("Unexpected non-boolean byte"),
    //             }
    //         };

    //         let anchor = crate::Anchor::from_bytes(read_32_bytes(&mut r)).unwrap();
    //         let v = NoteValue::from_bytes(read_8_bytes(&mut r));
    //         let nd = NoteDenom::new_for_proof(&hex::encode(&read_32_bytes(&mut r)));
    //         let nf_old = crate::note::Nullifier::from_bytes(&read_32_bytes(&mut r)).unwrap();
    //         let recp = RecpAddr::new(read_32_bytes(&mut r));
    //         let cmx =
    //             crate::note::ExtractedNoteCommitment::from_bytes(&read_32_bytes(&mut r)).unwrap();
    //         let instance = Instance::from_parts(anchor, nd, v, recp, nf_old, cmx);

    //         let mut proof_bytes = vec![];
    //         r.read_to_end(&mut proof_bytes)?;
    //         let proof = Proof::new(proof_bytes);

    //         Ok((instance, proof))
    //     }

    //     if std::env::var_os("ORCHARD_CIRCUIT_TEST_GENERATE_NEW_PROOF").is_some() {
    //         let create_proof = || -> std::io::Result<()> {
    //             let mut rng = os_rng();

    //             let (circuit, instance) = generate_circuit_instance(rng);
    //             let instances = &[instance.clone()];

    //             let pk = ProvingKey::build();
    //             let proof = Proof::create(&pk, &[circuit], instances, &mut rng).unwrap();
    //             assert!(proof.verify(&vk, instances).is_ok());

    //             let file = std::fs::File::create("circuit_proof_test_case.bin")?;
    //             write_test_case(file, &instance, &proof)
    //         };
    //         create_proof().expect("should be able to write new proof");
    //     }

    //     // Parse the hardcoded proof test case.
    //     let (instance, proof) = {
    //         let test_case_bytes = include_bytes!("circuit_proof_test_case.bin");
    //         read_test_case(&test_case_bytes[..]).expect("proof must be valid")
    //     };
    //     assert_eq!(proof.0.len(), 4992);

    //     assert!(proof.verify(&vk, &[instance]).is_ok());
    // }

    /// Integration test for genesis merkle tree inclusion proof
    /// Verifies that a participant can prove their inclusion in the genesis distribution
    #[test]
    fn test_genesis_merkle_inclusion() {
        let mut rng = os_rng();

        let (circuits, instances): (Vec<_>, Vec<_>) = iter::once(())
            .map(|()| generate_circuit_instance(&mut rng))
            .unzip();

        for (circuit, instance) in circuits.iter().zip(instances.iter()) {
            // Test MockProver passes with correct instance
            let result = MockProver::run(
                K,
                circuit,
                instance
                    .to_halo2_instance()
                    .iter()
                    .map(|p| p.to_vec())
                    .collect(),
            );

            assert!(
                result.is_ok(),
                "Genesis merkle inclusion proof should create valid MockProver"
            );
            assert_eq!(
                result.unwrap().verify(),
                Ok(()),
                "Genesis merkle inclusion proof should verify correctly"
            );
        }
    }

    #[test]
    fn test_genesis_merkle_path_with_various_positions() {
        // Test genesis merkle path verification with different leaf positions
        // This validates that the merkle path proof works correctly at any tree position
        let mut rng = os_rng();

        // Test multiple instances with potentially different merkle paths
        for test_num in 0..3 {
            std::println!("Testing genesis merkle path with instance {}", test_num);
            let (circuit, instance) = generate_circuit_instance(&mut rng);

            let result = MockProver::run(
                K,
                &circuit,
                instance
                    .to_halo2_instance()
                    .iter()
                    .map(|p| p.to_vec())
                    .collect(),
            );

            assert!(
                result.is_ok(),
                "MockProver creation failed for test {}",
                test_num
            );
            assert_eq!(
                result.unwrap().verify(),
                Ok(()),
                "Genesis merkle path verification failed for test {}",
                test_num
            );
        }
    }

    #[test]
    #[cfg(feature = "interface")]
    fn test_genesis_merkle_tree_with_generated_data() {
        /// Product A integration: suite multi-leaf Poseidon tree + matching claim note.
        /// Uses [`HeadstashProofBuilder::suite_backed_claim_pair`] so eligibility leaf,
        /// depth-32 path/anchor, and private cmx/nf are consistent with synthesize.
        use crate::suite::suite::{HeadstashProofBuilder, MerkleTestDataBuilder};
        use crate::suite::HeadstashCircuitSuite;
        use cw_orch::mock::Mock;

        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let num_participants = 8;
        let selected_index = 3;

        let test_data = suite
            .generate_circuit_test_data(num_participants, selected_index)
            .expect("Should generate test data successfully");

        assert!(
            suite.verify_merkle_path(&test_data.leaf_hash, &test_data.auth_path, &test_data.root),
            "Generated merkle path should be valid (shallow suite root)"
        );
        // Circuit anchor is depth-32 Poseidon path root (may differ from shallow root).
        assert_ne!(
            test_data.circuit_anchor.to_bytes(),
            [0u8; 32],
            "circuit anchor must be non-zero for multi-leaf tree"
        );

        let (circuit, instance, anchor, partial) = suite
            .suite_backed_claim_pair(num_participants, selected_index)
            .expect("suite-backed claim pair");

        assert_eq!(instance.anchor.to_bytes(), anchor.to_bytes());
        assert_eq!(partial.value, instance.v.inner());
        // Same leaf index family: suite test data and claim pair both select selected_index.
        assert_eq!(test_data.auth_path.leaf_index, selected_index);

        let cost =
            halo2_proofs::dev::CircuitCost::<pasta_curves::vesta::Point, _>::measure(K, &circuit);
        let proof_size = usize::from(cost.proof_size(1));
        assert!(proof_size > 0, "Proof size should be non-zero");
        std::println!(
            "suite multi-leaf claim: n={}, idx={}, proof_size={}",
            num_participants,
            selected_index,
            proof_size
        );

        let result = MockProver::run(
            K,
            &circuit,
            instance
                .to_halo2_instance()
                .iter()
                .map(|p| p.to_vec())
                .collect(),
        );
        let prover = result.unwrap_or_else(|e| {
            panic!("MockProver creation failed: {:?}", e);
        });
        assert_eq!(
            prover.verify(),
            Ok(()),
            "suite-backed multi-leaf Product A claim must MockProver-verify"
        );
    }

    #[test]
    #[cfg(feature = "interface")]
    fn test_genesis_merkle_tree_various_sizes() {
        /// Test genesis merkle tree generation with various tree sizes
        use crate::suite::HeadstashCircuitSuite;
        use crate::suite::suite::MerkleTestDataBuilder;
        use cw_orch::mock::Mock;

        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));

        // Test different tree sizes
        for num_leaves in [2, 4, 8, 16, 32] {
            std::println!("Testing merkle tree with {} leaves", num_leaves);

            // Test claiming from different positions
            for selected_index in [0, num_leaves / 2, num_leaves - 1] {
                let test_data = suite
                    .generate_circuit_test_data(num_leaves, selected_index)
                    .expect("Should generate test data");

                // Verify the path
                assert!(
                    suite.verify_merkle_path(
                        &test_data.leaf_hash,
                        &test_data.auth_path,
                        &test_data.root
                    ),
                    "Path should verify for tree size {} at index {}",
                    num_leaves,
                    selected_index
                );

                // Verify position encoding
                assert_eq!(
                    test_data.auth_path.leaf_index, selected_index,
                    "Leaf index should match selected index"
                );

                std::println!(
                    "  - Index {}: depth={}, root={:?}",
                    selected_index,
                    test_data.tree_depth,
                    test_data.root
                );
            }
        }
    }

    #[cfg(feature = "dev-graph")]
    #[test]
    fn print_action_circuit() {
        use plotters::prelude::*;

        let root = BitMapBackend::new("action-circuit-layout.png", (1024, 768)).into_drawing_area();
        root.fill(&WHITE).unwrap();
        let root = root
            .titled("Headstash Action Circuit", ("sans-serif", 60))
            .unwrap();

        let circuit = Circuit {
            path: Value::unknown(),
            pos: Value::unknown(),
            sig_e: Value::unknown(),
            sig_r: Value::unknown(),
            sig_s: Value::unknown(),
            epkx: Value::unknown(),
            epky: Value::unknown(),
            rho_old: Value::unknown(),
            psi_old: Value::unknown(),
            rcm_old: Value::unknown(),
            cm_old: Value::unknown(),
            fdi: Value::unknown(),
            nd: Value::unknown(),
            recp: Value::unknown(),
            v: Value::unknown(),
            // alpha: Value::unknown(),
            // ak: Value::unknown(),
            nk: Value::unknown(),
            rseed_lo: Value::unknown(),
            rseed_hi: Value::unknown(),
            // rivk: Value::unknown(),
            // g_d_new: Value::unknown(),
            // pk_d_new: Value::unknown(),
            // v_new: Value::unknown(),
            // psi_new: Value::unknown(),
            // rcm_new: Value::unknown(),
            rcv: Value::unknown(),
        };
        halo2_proofs::dev::CircuitLayout::default()
            .show_labels(false)
            .view_height(0..(1 << 11))
            .render(K, &circuit, &root)
            .unwrap();
    }
}
