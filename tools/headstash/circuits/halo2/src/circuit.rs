//! The Orchard Action circuit implementation.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[cfg(feature = "interface")]
use cosmwasm_std::Checksum;
#[cfg(feature = "interface")]
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};
// Re-export types needed for test circuits
pub use gadget::assign_free_advice;
pub use gadget::secp256k1_chip::{
    CrtInteger, FpChip, FpConfig, FpInstructions, Secp256k1Chip, Secp256k1Config, Secp256k1Fp,
    Secp256k1FpChip, Secp256k1Fq, Secp256k1FqChip,
};

use halo2_proofs::{
    circuit::{Layouter, Value, floor_planner},
    plonk::{
        self, Advice, BatchVerifier, Column, Instance as InstanceColumn, SingleVerifier,
        TableColumn,
    },
    transcript::{Blake2bRead, Blake2bWrite},
};

use pasta_curves::{pallas, vesta};
use rand_core::Rng;

use self::gadget::add_chip::{AddChip, AddConfig};
use crate::{
    address::RecpAddr,
    builder::SpendInfo,
    constants::{MERKLE_DEPTH_ORCHARD, OrchardFixedBases},
    keys::NullifierDerivingKey,
    note::{
        ExtractedNoteCommitment, Note, Rho,
        commitment::{NoteCommitTrapdoor, NoteCommitment},
        nullifier::Nullifier,
    },
    tree::{Anchor, MerkleHashOrchard},
    value::{NoteDenom, NoteValue},
};
use ff::{Field, PrimeField};
use halo2_gadgets::{
    ecc::chip::{EccChip, EccConfig},
    poseidon::{Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig, primitives as poseidon},
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
#[cfg(test)]
mod note_commit_bit_tests;
pub mod note_poseidon_gadget;

pub use crate::Proof;

/// Row budget. One GLV mul fit in K=18. Personal-sign verify is two
/// (fixed base and variable base), which does not.
pub(crate) const K: u32 = 19;

// Absolute offsets for public inputs.
// Slot 1 was the denom and slot 5 was the note commitment. Both stay
// witnesses. The sidecar statement is the canonical claim body.
const ANCHOR: usize = 0;
const CTX_LO: usize = 1;
const HS_V: usize = 2;
const RECP: usize = 3;
const NF_OLD: usize = 4;
const CTX_HI: usize = 5;
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
    /// Campaign context limbs. Public. Not the eligible key.
    pub(crate) ctx_lo: Value<pallas::Base>,
    pub(crate) ctx_hi: Value<pallas::Base>,
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

        // Legacy constructor has no campaign. The scoped key of the zero
        // context is what synthesize checks. A real epoch claim overwrites
        // both the limbs and this key before proving.
        let ctx = [0u8; 32];
        let (ctx_lo, ctx_hi) = crate::claim_auth::ctx_limbs(&ctx);
        let nk = crate::keys::NullifierDerivingKey::from_prf(crate::claim_auth::claim_nk_scoped(
            bound.rseed_lo,
            bound.rseed_hi,
            &ctx,
        ));

        Circuit {
            path: Value::known(spend.merkle_path.auth_path()),
            pos: Value::known(spend.merkle_path.position()),
            v: Value::known(spend.note.value()),
            rho_old: Value::known(rho_old),
            psi_old: Value::known(psi_old),
            rcm_old: Value::known(rcm_old),
            cm_old: Value::known(spend.note.commitment()),
            nk: Value::known(nk),
            ctx_lo: Value::known(ctx_lo),
            ctx_hi: Value::known(ctx_hi),
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

    /// Fill `(e, r, s)` from a personal_sign of the canonical claim body.
    ///
    /// The body is `headstash-claim-v1:{ctx}:{root}:{nullifier}:{destination}:{amount}`.
    /// `instance.e` becomes the reduced EIP-191 keccak of that string.
    /// The circuit checks the signature under the note's key. It does not
    /// hash the string again: the public `e` is that challenge.
    pub fn attach_personal_sign(&mut self, instance: &mut Instance, secret_be: &[u8; 32]) {
        use crate::claim_auth::{canonical_claim_body, ctx_limbs, sign_canonical_body};
        use crate::circuit::gadget::secp256k1_chip::secp_fq_from_secret_be;
        use crate::claim_auth::{keccak256, sign_personal_claim};

        let body = canonical_claim_body(
            &instance.ctx,
            &instance.anchor.to_bytes(),
            &instance.nf.to_bytes(),
            &instance.destination,
            instance.v.inner(),
        );
        let (e_be, r_be, s_be) = sign_canonical_body(secret_be, &body);
        instance.e = e_be;
        let (ctx_lo, ctx_hi) = ctx_limbs(&instance.ctx);
        self.ctx_lo = Value::known(ctx_lo);
        self.ctx_hi = Value::known(ctx_hi);
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
        let secp256k1 = Secp256k1Config::configure(meta, secp_advices, range_check);

        // CondSwap for Poseidon-v1 distro Merkle path (node/sibling ordering).
        let cond_swap_config = CondSwapChip::configure(meta, advices[0..5].try_into().unwrap());

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

        // The denom stays a leaf witness. The public statement publishes the
        // campaign context, the amount, and the payment address.
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
        let ctx_lo = assign_free_advice(
            layouter.namespace(|| "witness ctx lo"),
            config.advices[0],
            self.ctx_lo,
        )?;
        let ctx_hi = assign_free_advice(
            layouter.namespace(|| "witness ctx hi"),
            config.advices[0],
            self.ctx_hi,
        )?;
        layouter.constrain_instance(ctx_lo.cell(), config.primary, CTX_LO)?;
        layouter.constrain_instance(ctx_hi.cell(), config.primary, CTX_HI)?;
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
            ctx_lo,
            ctx_hi,
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

        // The note commitment stays inside DeriveNullifier. It is not a public word.
        let _cmx_private = cmx_cell;

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
                8, // i — anchor, nd, v, recp, nf, cmx, challenge lo, challenge hi
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
    /// Note commitment. Kept for the retained on-chain mapping. It is not a
    /// public instance word and it is not on the sidecar wire.
    pub(crate) cmx: ExtractedNoteCommitment,
    /// Campaign context, 32 bytes. Public.
    pub ctx: [u8; 32],
    /// Payment address in the canonical body. Public. Not the eligible key.
    pub destination: alloc::string::String,
    /// Reduced EIP-191 challenge of the canonical body, 32 big-endian bytes.
    pub e: [u8; 32],
}

/// What a halo2 sidecar is allowed to know about one claim.
#[derive(Clone, Debug)]
pub struct EpochStatement {
    pub ctx: [u8; 32],
    pub root: [u8; 32],
    pub nullifier: [u8; 32],
    pub destination: String,
    pub amount: u64,
    /// 20-byte bech32 payload, right-padded with zeros.
    pub recipient: [u8; 32],
}

#[derive(Debug)]
pub enum EpochVerifyError {
    Public,
    Proof,
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
            ctx: [0u8; 32],
            destination: String::new(),
            e: [0u8; 32],
            // rk,
            // enable_spend,
            // enable_output,
        }
    }

    /// Sidecar wire. The note commitment and the denom are absent.
    ///
    /// Anchor 32, context 32, value 8, nullifier 32, recipient 32, challenge 32.
    /// Total 168 bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        const THREETWO: usize = 32;
        const EIGHT: usize = 8;
        const TOTAL_SIZE: usize = (5 * THREETWO) + EIGHT;
        assert_eq!(bytes.len(), TOTAL_SIZE);
        let mut offset = 0;

        let anchor: &[u8; 32] = &bytes[offset..offset + THREETWO].try_into().expect("anchor");
        offset += THREETWO;

        let ctx: [u8; 32] = bytes[offset..offset + THREETWO].try_into().expect("ctx");
        offset += THREETWO;

        let v_bytes = &bytes[offset..offset + EIGHT];
        let v = u64::from_le_bytes(v_bytes.try_into().expect("v"));
        offset += EIGHT;

        let nf: &[u8; 32] = &bytes[offset..offset + THREETWO].try_into().expect("nf");
        offset += THREETWO;

        let recp = &bytes[offset..offset + THREETWO];
        offset += THREETWO;
        let e: [u8; 32] = bytes[offset..offset + THREETWO].try_into().expect("e");

        Instance {
            anchor: Anchor::from_bytes(*anchor).expect("anchor"),
            nd: NoteDenom::from([0u8; 32]),
            v: NoteValue::from(v),
            nf: Nullifier::from_bytes(nf).expect("msg"),
            recp: RecpAddr::try_from(recp).expect(""),
            cmx: ExtractedNoteCommitment::from_bytes(&[0u8; 32]).expect(""),
            ctx,
            destination: String::new(),
            e,
        }
    }

    /// Sidecar wire. See [`Self::from_bytes`].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(168);

        bytes.extend_from_slice(&self.anchor.to_bytes());
        bytes.extend_from_slice(&self.ctx);
        bytes.extend_from_slice(&self.v.inner().to_le_bytes());
        bytes.extend_from_slice(&self.nf.to_bytes());
        bytes.extend_from_slice(&self.recp.to_canonical_bytes());
        bytes.extend_from_slice(&self.e);

        bytes
    }

    /// 8x32-byte little-endian scalars for wasmvm `proof_instance_verify`.
    /// Same field elements as `to_halo2_instance` rows 0..8.
    pub fn to_vm_public_inputs(&self) -> Vec<u8> {
        let (ctx_lo, ctx_hi) = crate::claim_auth::ctx_limbs(&self.ctx);
        let mut out = Vec::with_capacity(8 * 32);
        let e_lo = u128::from_be_bytes(self.e[16..32].try_into().expect("e lo"));
        let e_hi = u128::from_be_bytes(self.e[0..16].try_into().expect("e hi"));
        let scalars = [
            self.anchor.inner(),
            ctx_lo,
            pallas::Base::from(self.v.inner()),
            self.recp.to_fp(),
            self.nf.0,
            ctx_hi,
            pallas::Base::from_u128(e_lo),
            pallas::Base::from_u128(e_hi),
        ];
        for scalar in scalars {
            out.extend_from_slice(scalar.to_repr().as_ref());
        }
        out
    }
    /// Public instance. Slot 8 stays zero. The note commitment is not a row.
    pub fn to_halo2_instance(&self) -> [[vesta::Scalar; 9]; 1] {
        let mut instance = [vesta::Scalar::zero(); 9];
        let (ctx_lo, ctx_hi) = crate::claim_auth::ctx_limbs(&self.ctx);
        instance[ANCHOR] = self.anchor.inner();
        instance[CTX_LO] = ctx_lo;
        instance[HS_V] = self.v.inner().into();
        instance[RECP] = self.recp.to_fp();
        instance[NF_OLD] = self.nf.0;
        instance[CTX_HI] = ctx_hi;
        let e_lo = u128::from_be_bytes(self.e[16..32].try_into().expect("e lo"));
        let e_hi = u128::from_be_bytes(self.e[0..16].try_into().expect("e hi"));
        instance[E_LO] = pallas::Base::from_u128(e_lo);
        instance[E_HI] = pallas::Base::from_u128(e_hi);

        [instance]
    }
}

fn field_opt(bytes: [u8; 32]) -> Option<pallas::Base> {
    Option::from(pallas::Base::from_repr(bytes))
}

impl Instance {
    /// Public instance for one epoch claim. `e` is the canonical challenge.
    pub fn from_epoch_statement(statement: &EpochStatement) -> Result<Self, EpochVerifyError> {
        use crate::claim_auth::{canonical_challenge, canonical_claim_body};

        let anchor_fp = field_opt(statement.root).ok_or(EpochVerifyError::Public)?;
        let nf_fp = field_opt(statement.nullifier).ok_or(EpochVerifyError::Public)?;
        let anchor = Anchor::from_bytes(statement.root)
            .into_option()
            .ok_or(EpochVerifyError::Public)?;
        let nf = Nullifier::from_bytes(&statement.nullifier)
            .into_option()
            .ok_or(EpochVerifyError::Public)?;
        let _ = (anchor_fp, nf_fp);
        let body = canonical_claim_body(
            &statement.ctx,
            &statement.root,
            &statement.nullifier,
            &statement.destination,
            statement.amount,
        );
        Ok(Instance {
            anchor,
            nd: NoteDenom::from([0u8; 32]),
            v: NoteValue::from(statement.amount),
            recp: RecpAddr::new(statement.recipient),
            nf,
            cmx: ExtractedNoteCommitment::from_bytes(&[0u8; 32])
                .into_option()
                .ok_or(EpochVerifyError::Public)?,
            ctx: statement.ctx,
            destination: statement.destination.clone(),
            e: canonical_challenge(&body),
        })
    }
}

fn epoch_verifying_key() -> &'static std::sync::OnceLock<VerifyingKey> {
    use std::sync::OnceLock;
    static VK: OnceLock<VerifyingKey> = OnceLock::new();
    &VK
}

/// Build the halo2 verifying key once for this process.
///
/// The prover self-check uses this cache. Claim checks do not.
pub fn prepare_halo2_verifier() {
    let cell = epoch_verifying_key();
    if cell.get().is_some() {
        return;
    }
    let started = std::time::Instant::now();
    let _vk = cell.get_or_init(VerifyingKey::build);
    std::eprintln!(
        "epoch-claim vk_ready_secs {:.3}",
        started.elapsed().as_secs_f64()
    );
}

/// Verify a halo2 claim. The proof is checked against the canonical statement.
/// The eligible key, the signature scalars, and the note commitment are not inputs.
pub fn verify_epoch_statement(
    proof: &[u8],
    statement: &EpochStatement,
) -> Result<(), EpochVerifyError> {
    let instance = Instance::from_epoch_statement(statement)?;
    let vk = epoch_verifying_key().get_or_init(VerifyingKey::build);
    Proof::new(proof.to_vec())
        .verify(vk, &[instance])
        .map_err(|_| EpochVerifyError::Proof)
}


fn checksum32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(Checksum::generate(bytes).as_slice());
    out
}

/// Params, post-synthesis constraint system, verifying key, and an 80-byte footer.
///
/// `instances` is the footer public-input count. The epoch prover hashes nine
/// limbs, so that blob uses 9. The campaign hash is SHA-256 of this whole buffer.
pub fn pasta_circuit_blob(
    params: &halo2_proofs::poly::commitment::Params<vesta::Affine>,
    vk: &plonk::VerifyingKey<vesta::Affine>,
    k: u8,
    instances: u8,
) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    params.write(&mut buf).map_err(|_| "circuit")?;
    let param_len = buf.len();
    vk.cs().write(&mut buf).map_err(|_| "circuit")?;
    let cs_len = buf.len() - param_len;
    vk.write(&mut buf).map_err(|_| "circuit")?;
    let vk_len = buf.len() - cs_len - param_len;
    let param_hash = checksum32(&buf[..param_len]);
    let vk_hash = checksum32(&buf[param_len..]);
    buf.extend_from_slice(
        &zk_cosmwasm::CircuitFooter::new(
            zk_cosmwasm::CircuitType::Plonkish,
            zk_cosmwasm::curves::CurveType::Pasta,
            k,
            instances,
            param_len as u32,
            cs_len as u32,
            vk_len as u32,
            param_hash,
            vk_hash,
        )
        .to_bytes(),
    );
    Ok(buf)
}

/// Epoch blob for vk id 1. Built once from the prover's cached verifying key.
pub fn epoch_circuit_blob() -> &'static [u8] {
    use std::sync::OnceLock;
    static BLOB: OnceLock<Vec<u8>> = OnceLock::new();
    BLOB.get_or_init(|| {
        prepare_halo2_verifier();
        let vk = epoch_verifying_key().get().expect("vk");
        pasta_circuit_blob(&vk.params, &vk.vk, K as u8, 9).expect("circuit")
    })
    .as_slice()
}

/// SHA-256 of the whole epoch blob, including the footer.
pub fn epoch_vk_hash() -> [u8; 32] {
    checksum32(epoch_circuit_blob())
}

/// Nine 32-byte little-endian limbs. Rows 0..8 match the circuit. Row 8 is zero
/// because the prover transcript includes that trailing instance.
pub fn host_public_inputs(statement: &EpochStatement) -> Result<Vec<u8>, EpochVerifyError> {
    let instance = Instance::from_epoch_statement(statement)?;
    let mut bytes = instance.to_vm_public_inputs();
    bytes.extend_from_slice(&[0u8; 32]);
    Ok(bytes)
}

/// Leaf 0 of a 32,769-note Poseidon-v1 suite tree.
///
/// Suite depth is 16. The circuit path is that real sibling path,
/// padded with zero siblings to [`MERKLE_DEPTH_ORCHARD`]. The anchor
/// is the padded root. The payment address is not part of the leaf.
#[cfg(all(feature = "host-crypto", feature = "interface"))]
fn depth16_member(
    recipient: [u8; 32],
    amount: u64,
    rng: &mut rand010::rngs::ThreadRng,
) -> Result<(Note, crate::tree::MerklePath, Anchor), String> {
    use crate::note::RandomSeed;
    use crate::suite::builders::{
        poseidon_distro_leaf_from_note, HeadstashBitwiseInstance, HeadstashCircuitSuite,
        MerkleTestDataBuilder, PartialClaimNote,
    };
    use cw_orch::mock::Mock;

    const SUITE_NOTES: usize = 32_769;
    const MEMBER: usize = 0;

    let started = std::time::Instant::now();
    let suite = HeadstashCircuitSuite::new(Mock::new("epoch-claim"));
    let mut leaves = suite.generate_test_leaves(SUITE_NOTES);
    if leaves.len() != SUITE_NOTES {
        return Err("suite notes".into());
    }
    {
        let member = &mut leaves[MEMBER];
        member.raw_v = amount;
        member.raw_token = String::from("uterp");
        member.raw_fdi = 1;
        member.v = pallas::Base::from(amount);
        member.fdi = pallas::Base::from(1u64);
        let nd_bytes =
            <HeadstashCircuitSuite<Mock> as HeadstashBitwiseInstance>::derive_nd("uterp");
        let nd = pallas::Base::from_repr(nd_bytes);
        if !bool::from(nd.is_some()) {
            return Err("nd".into());
        }
        member.nd = nd.unwrap();
    }

    let mut tries = 0u32;
    let (note, leaf_fp) = loop {
        tries += 1;
        if tries > 64 {
            return Err("member note".into());
        }
        let rho = Rho::from_nf_old(Nullifier::dummy(rng));
        let trial = RandomSeed::from_bytes(leaves[MEMBER].rseed, &rho);
        if !bool::from(trial.is_some()) {
            rand_core::Rng::fill_bytes(rng, &mut leaves[MEMBER].rseed);
            continue;
        }
        let rseed = trial.unwrap();
        let mut partial = PartialClaimNote::from_test_leaf(&leaves[MEMBER]);
        partial.recipient = recipient;
        match partial.to_note(rho, rseed) {
            Ok(note) => {
                if note.value().inner() != amount {
                    return Err("amount".into());
                }
                let leaf_fp = poseidon_distro_leaf_from_note(&note);
                let expected = suite.compute_leaf_from_data_poseidon_v1(&leaves[MEMBER]);
                if leaf_fp != expected {
                    return Err("claim leaf is not the suite member".into());
                }
                break (note, leaf_fp);
            }
            Err(_) => {
                rand_core::Rng::fill_bytes(rng, &mut leaves[MEMBER].rseed);
            }
        }
    };

    let leaf_hashes: Vec<_> = leaves
        .iter()
        .map(|leaf| suite.compute_leaf_from_data_poseidon_v1(leaf))
        .collect();
    if leaf_hashes[MEMBER] != leaf_fp {
        return Err("tree leaf".into());
    }
    let tree = suite.generate_full_merkle_tree_poseidon_v1(leaf_hashes);
    if tree.depth != 16 {
        return Err(format!("suite depth {}", tree.depth));
    }
    let auth = suite.compute_merkle_path(&tree, MEMBER);
    if auth.siblings.len() != tree.depth || auth.siblings[0] == pallas::Base::ZERO {
        return Err("suite path".into());
    }
    if !suite.verify_merkle_path_poseidon_v1(&leaf_fp, &auth, &tree.root()) {
        return Err("suite inclusion".into());
    }
    let (path, anchor) = auth.to_circuit_path_and_root_poseidon_v1(leaf_fp);
    if path.root_from_leaf(leaf_fp).to_bytes() != anchor.to_bytes() {
        return Err("circuit anchor".into());
    }
    if anchor.to_bytes() == tree.root().to_repr() {
        return Err("anchor is the shallow suite root".into());
    }
    std::eprintln!(
        "epoch-claim tree notes {} suite_depth {} index {} tree_secs {:.3}",
        SUITE_NOTES,
        tree.depth,
        MEMBER,
        started.elapsed().as_secs_f64()
    );
    Ok((note, path, anchor))
}

#[cfg(all(feature = "host-crypto", not(feature = "interface")))]
fn depth16_member(
    _recipient: [u8; 32],
    _amount: u64,
    _rng: &mut rand010::rngs::ThreadRng,
) -> Result<(Note, crate::tree::MerklePath, Anchor), String> {
    Err("epoch claim tree needs the interface feature".into())
}

/// Two proofs of one epoch claim. The second transcript is a fresh
/// digest of the same nullifier. The note is leaf 0 of a 32,769-note
/// Poseidon-v1 suite tree (suite depth 16). Circuit depth stays
/// [`MERKLE_DEPTH_ORCHARD`].
#[cfg(feature = "host-crypto")]
#[derive(Clone, Debug)]
pub struct EpochClaimBundle {
    pub proof: Vec<u8>,
    pub replay_proof: Vec<u8>,
    pub ctx: [u8; 32],
    pub root: [u8; 32],
    pub nullifier: [u8; 32],
    pub destination: String,
    pub amount: u64,
    pub message: String,
}

/// Prove a canonical epoch claim for `destination`.
///
/// `recipient` is the 20-byte bech32 payload, right-padded with zeros.
/// The eligible key stays a witness. [`prepare_halo2_verifier`] caches
/// the verifying key for this self-check. The sidecar claim path does not
/// call [`verify_epoch_statement`].
#[cfg(feature = "host-crypto")]
pub fn prove_epoch_claim(
    destination: &str,
    recipient: [u8; 32],
    amount: u64,
    ctx: [u8; 32],
) -> Result<EpochClaimBundle, String> {
    use std::time::Instant;

    use crate::circuit::gadget::secp256k1_chip::secp_fp_from_coord_be;
    use crate::claim_auth::{canonical_claim_body, claim_nk_scoped};
    use halo2_proofs::circuit::Value;

    if destination.is_empty() || destination.len() > 128 || destination.contains(':') {
        return Err("destination".into());
    }
    if amount == 0 {
        return Err("amount".into());
    }

    let mut rng = crate::os_rng();
    let (note, path, anchor) = depth16_member(recipient, amount, &mut rng)?;
    let esk = note.elig_sk();
    let (epkx_be, epky_be) = esk.epk().xy();
    let epkx = secp_fp_from_coord_be(&epkx_be);
    let epky = secp_fp_from_coord_be(&epky_be);
    let bound = note.hiding_binding();
    let (ctx_lo, ctx_hi) = crate::claim_auth::ctx_limbs(&ctx);
    let nk = NullifierDerivingKey::from_prf(claim_nk_scoped(bound.rseed_lo, bound.rseed_hi, &ctx));
    let nf = note.nullifier_for_ctx(&ctx);
    let mut circuit = Circuit {
        path: Value::known(path.auth_path()),
        pos: Value::known(path.position()),
        nk: Value::known(nk),
        ctx_lo: Value::known(ctx_lo),
        ctx_hi: Value::known(ctx_hi),
        rseed_lo: Value::known(bound.rseed_lo),
        rseed_hi: Value::known(bound.rseed_hi),
        nd: Value::known(note.nd().to_fp()),
        v: Value::known(note.value()),
        fdi: Value::known(pallas::Base::from(note.fdi())),
        recp: Value::known(note.recipient().to_fp()),
        sig_e: Value::unknown(),
        sig_r: Value::unknown(),
        sig_s: Value::unknown(),
        epkx: Value::known(epkx),
        epky: Value::known(epky),
        rho_old: Value::known(bound.rho),
        psi_old: Value::known(bound.psi),
        rcm_old: Value::known(bound.rcm),
        cm_old: Value::known(note.commitment()),
    };
    let mut instance = Instance {
        anchor,
        nd: note.nd(),
        v: note.value(),
        recp: note.recipient(),
        nf,
        cmx: note.commitment().into(),
        ctx,
        destination: destination.to_string(),
        e: [0u8; 32],
    };
    circuit.attach_personal_sign(&mut instance, &esk.secret_bytes());

    let root = instance.anchor.to_bytes();
    let nullifier = instance.nf.to_bytes();
    let body = canonical_claim_body(&ctx, &root, &nullifier, destination, amount);
    let statement = EpochStatement {
        ctx,
        root,
        nullifier,
        destination: destination.to_string(),
        amount,
        recipient,
    };
    let rebuilt =
        Instance::from_epoch_statement(&statement).map_err(|_| "statement".to_string())?;
    if rebuilt.to_halo2_instance() != instance.to_halo2_instance() {
        return Err("public instance does not match the epoch statement".into());
    }

    std::eprintln!("epoch-claim keygen");
    let started = Instant::now();
    let pk = ProvingKey::build();
    std::eprintln!(
        "epoch-claim keygen_secs {:.3}",
        started.elapsed().as_secs_f64()
    );

    std::eprintln!("epoch-claim prove");
    let started = Instant::now();
    let proof = Proof::create(&pk, &[circuit.clone()], &[instance.clone()], &mut rng)
        .map_err(|err| format!("prove {err:?}"))?;
    std::eprintln!(
        "epoch-claim prove_secs {:.3}",
        started.elapsed().as_secs_f64()
    );

    std::eprintln!("epoch-claim replay");
    let started = Instant::now();
    let replay = Proof::create(&pk, &[circuit], &[instance], &mut rng)
        .map_err(|err| format!("replay prove {err:?}"))?;
    std::eprintln!(
        "epoch-claim replay_secs {:.3}",
        started.elapsed().as_secs_f64()
    );
    if proof.as_ref() == replay.as_ref() {
        return Err("replay proof was identical".into());
    }

    std::eprintln!("epoch-claim self-check");
    let started = Instant::now();
    verify_epoch_statement(proof.as_ref(), &statement).map_err(|err| format!("verify {err:?}"))?;
    verify_epoch_statement(replay.as_ref(), &statement)
        .map_err(|err| format!("replay verify {err:?}"))?;
    std::eprintln!(
        "epoch-claim verify_secs {:.3}",
        started.elapsed().as_secs_f64()
    );
    std::eprintln!("epoch-claim proof_bytes {}", proof.as_ref().len());

    Ok(EpochClaimBundle {
        proof: proof.as_ref().to_vec(),
        replay_proof: replay.as_ref().to_vec(),
        ctx,
        root,
        nullifier,
        destination: destination.to_string(),
        amount,
        message: String::from_utf8(body).map_err(|_| "message".to_string())?,
    })
}

impl Proof {
    /// Creates a proof for the given circuits and instances.
    pub fn create(
        pk: &ProvingKey,
        circuits: &[Circuit],
        instances: &[Instance],
        mut rng: impl Rng,
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

    use crate::os_rng;
    use ff::{Field, PrimeField};
    use halo2_proofs::{circuit::Value, dev::MockProver};
    use pasta_curves::pallas;
    use rand_core::Rng;

    use super::{Circuit, Instance, K, Proof, ProvingKey, VerifyingKey};
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
    fn generate_circuit_instance<R: Rng>(mut rng: R) -> (Circuit, Instance) {
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
        let ctx = [0x44u8; 32];
        let (ctx_lo, ctx_hi) = crate::claim_auth::ctx_limbs(&ctx);
        let nk = crate::keys::NullifierDerivingKey::from_prf(crate::claim_auth::claim_nk_scoped(
            bound.rseed_lo,
            bound.rseed_hi,
            &ctx,
        ));
        let nf = spent_note.nullifier_for_ctx(&ctx);
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
            ctx_lo: Value::known(ctx_lo),
            ctx_hi: Value::known(ctx_hi),
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
            ctx,
            destination: "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq0g3g2k".into(),
            e: [0u8; 32],
        };
        circuit.attach_personal_sign(&mut instance, &esk.secret_bytes());
        (circuit, instance)
    }

    #[test]
    fn epoch_claim_public_statement_verifies() {
        use crate::claim_auth::{canonical_challenge, canonical_claim_body};

        let mut rng = os_rng();
        let (circuit, instance) = generate_circuit_instance(&mut rng);
        let body = canonical_claim_body(
            &instance.ctx,
            &instance.anchor.to_bytes(),
            &instance.nf.to_bytes(),
            &instance.destination,
            instance.v.inner(),
        );
        assert_eq!(instance.e, canonical_challenge(&body));
        assert_eq!(instance.to_bytes().len(), 168);
        let cmx = instance.cmx.to_bytes();
        let wire = instance.to_bytes();
        assert!(
            !wire.windows(32).any(|window| window == cmx),
            "the note commitment is not on the sidecar wire"
        );
        let public = instance.to_halo2_instance()[0];
        assert!(public.iter().all(|word| *word != instance.cmx.inner()));
        assert!(bool::from(public[8].is_zero()));
        let columns: Vec<Vec<pallas::Base>> = instance
            .to_halo2_instance()
            .iter()
            .map(|row| row.to_vec())
            .collect();
        assert_eq!(
            MockProver::run(K, &circuit, columns).unwrap().verify(),
            Ok(())
        );

        let mut moved = instance.clone();
        moved.ctx[0] ^= 1;
        let bad: Vec<Vec<pallas::Base>> = moved
            .to_halo2_instance()
            .iter()
            .map(|row| row.to_vec())
            .collect();
        assert!(
            MockProver::run(K, &circuit, bad).unwrap().verify().is_err(),
            "a different campaign context must not verify"
        );
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
            assert_eq!(usize::from(circuit_cost.proof_size(1)), 4800);
            assert_eq!(usize::from(circuit_cost.proof_size(2)), 6560);
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
        use crate::suite::HeadstashCircuitSuite;
        /// Product A integration: suite multi-leaf Poseidon tree + matching claim note.
        /// Uses [`HeadstashProofBuilder::suite_backed_claim_pair`] so eligibility leaf,
        /// depth-32 path/anchor, and private cmx/nf are consistent with synthesize.
        use crate::suite::builders::{HeadstashProofBuilder, MerkleTestDataBuilder};
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
        use crate::suite::builders::MerkleTestDataBuilder;
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
            ctx_lo: Value::unknown(),
            ctx_hi: Value::unknown(),
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

    /// Release only. Debug verify of this K=19 circuit is not the bar.
    #[cfg(all(not(debug_assertions), feature = "host-crypto", feature = "interface"))]
    #[test]
    fn warm_verify_stays_under_one_second() {
        use std::time::Instant;

        super::prepare_halo2_verifier();
        let destination = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
        let recipient = [0u8; 32];
        let ctx = [9u8; 32];
        let amount = 15u64;
        let bundle = super::prove_epoch_claim(destination, recipient, amount, ctx).expect("prove");
        let statement = super::EpochStatement {
            ctx: bundle.ctx,
            root: bundle.root,
            nullifier: bundle.nullifier,
            destination: bundle.destination.clone(),
            amount: bundle.amount,
            recipient,
        };
        let started = Instant::now();
        super::verify_epoch_statement(bundle.proof.as_slice(), &statement).expect("warm verify");
        let secs = started.elapsed().as_secs_f64();
        std::eprintln!("epoch-claim warm_verify_secs {secs:.3}");
        assert!(secs < 1.0, "warm verify {secs:.3}s");
    }
}
