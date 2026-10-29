//! k19 lab: registration-free blind claim.
//!
//! Proves, with no public registration step:
//! - knowledge of a secp256k1 secret key `sk` with `pk = sk·G` (GLV fixed-base mul);
//! - `leaf = Poseidon(TAG_NOTE_LEAF, Poseidon(TAG_PKCOM, pk.x, pk.y), amount)` sits in the
//!   depth-17 eligibility tree (the whole eligibility list is the anonymity set);
//! - `nullifier = Poseidon(TAG_NULLIFIER, sk, ctx)` (deterministic per key and campaign,
//!   unlinkable to pk without sk);
//! - ctx, destination (lo/hi) and amount are public inputs (vk3 layout:
//!   ctx, root, nullifier, dest_lo, dest_hi, amount).
//!
//! Why not ECDSA verify: an ECDSA signature is not unique per key (any nonce gives a valid
//! signature), so a nullifier derived from a signature lets one key claim many times, and a
//! nullifier derived from pk is computable by anyone who knows pk (it deanonymizes). The
//! only sound and blind nullifier with plain ECDSA keys is one from `sk` itself, which needs
//! `sk·G = pk` in circuit. ECDSA verify (`circuit.rs`) is kept for comparison only.
use std::{format, string::String, vec::Vec};

use ff::{Field, PrimeField};
use halo2_gadgets::{
    poseidon::{
        primitives::{self as poseidon_p, ConstantLength, P128Pow5T3},
        Hash as PoseidonHash, Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig,
    },
    utilities::{
        cond_swap::{CondSwapChip, CondSwapInstructions},
        lookup_range_check::{LookupRangeCheck, LookupRangeCheckConfig},
    },
};
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, SimpleFloorPlanner, Value},
    plonk::{self, Advice, Circuit, Column, ConstraintSystem, Instance, TableColumn},
};
use pasta_curves::pallas;

use crate::circuit::{Secp256k1Chip, Secp256k1Config, Secp256k1Fp, Secp256k1Fq};
use crate::distro_poseidon::{distro_crh_tag, personalization_to_fp};
use crate::note_membership::{note_leaf, note_root, NOTE_DEPTH};

pub const BLIND_K: u32 = 19;
const TAG_PKCOM: &str = "terp-hs-pkcom-v1";
const TAG_NULLIFIER: &str = "terp-hs-nf-v1";
const TAG_NOTE_LEAF: &str = "terp-hs-note-leaf-v2";
const TAG_REG_NF: &str = "terp-hs-regnf-v1";
const TAG_KEYCOM: &str = "terp-hs-keycom-v1";

/// Lab switch: synthesize the step-1 REGISTRATION statement of the blind two-step design.
/// Reg mode reuses the witnesses: `destination_hi` = private registration secret (NOT public),
/// `destination_lo` = public registrant/relayer binding. Public: [ctx, root, reg_nf, relayer, reg_leaf]
/// with reg_leaf = note_leaf(key_commitment(secret), amount), i.e. exactly a vk3 (K=11) leaf.
pub static REG_MODE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct BlindConfig {
    advice: [Column<Advice>; 9],
    instance: Column<Instance>,
    secp: Secp256k1Config,
    poseidon: PoseidonConfig<pallas::Base, 3, 2>,
    swap: CondSwapChip<pallas::Base>,
    table: TableColumn,
}

#[derive(Clone, Debug, Default)]
pub struct BlindClaimCircuit {
    pub sk: Value<Secp256k1Fq>,
    pub pkx: Value<Secp256k1Fp>,
    pub pky: Value<Secp256k1Fp>,
    pub ctx: Value<pallas::Base>,
    pub amount: Value<u64>,
    pub position: Value<u32>,
    pub path: Value<[pallas::Base; NOTE_DEPTH]>,
    pub destination_lo: Value<pallas::Base>,
    pub destination_hi: Value<pallas::Base>,
}

impl Circuit<pallas::Base> for BlindClaimCircuit {
    type Config = BlindConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        let instance = meta.instance_column();
        meta.enable_equality(instance);
        let advice: [Column<Advice>; 9] = core::array::from_fn(|_| meta.advice_column());
        for c in &advice {
            meta.enable_equality(*c);
        }
        let table = meta.lookup_table_column();
        let range_check = LookupRangeCheckConfig::configure(meta, advice[0], table);
        let secp = Secp256k1Config::configure_with_fixed_base(meta, advice, range_check);
        let rc_a = [meta.fixed_column(), meta.fixed_column(), meta.fixed_column()];
        let rc_b = [meta.fixed_column(), meta.fixed_column(), meta.fixed_column()];
        meta.enable_constant(rc_b[0]);
        let poseidon = PoseidonChip::configure::<P128Pow5T3>(
            meta,
            advice[0..3].try_into().unwrap(),
            advice[3],
            rc_a,
            rc_b,
        );
        let swap = CondSwapChip::construct(CondSwapChip::configure(meta, advice[0..5].try_into().unwrap()));
        BlindConfig { advice, instance, secp, poseidon, swap, table }
    }

    fn synthesize(&self, config: Self::Config, mut layouter: impl Layouter<pallas::Base>) -> Result<(), plonk::Error> {
        layouter.assign_table(
            || "10-bit range table",
            |mut t| {
                for i in 0..(1usize << 10) {
                    t.assign_cell(|| "idx", config.table, i, || Value::known(pallas::Base::from(i as u64)))?;
                }
                Ok(())
            },
        )?;
        let chip = Secp256k1Chip::construct(config.secp.clone());
        chip.load_fixed_base_table(layouter.namespace(|| "fb table"))?;
        let (sk, (px, py)) = chip.prove_key_pairing(layouter.namespace(|| "sk·G = pk"), self.sk, self.pkx, self.pky)?;
        let ctx = load(&mut layouter, config.advice[1], "ctx", self.ctx)?;
        let amount = load(&mut layouter, config.advice[2], "amount", self.amount.map(pallas::Base::from))?;
        let dest_lo = load(&mut layouter, config.advice[3], "dest_lo", self.destination_lo)?;
        let dest_hi = load(&mut layouter, config.advice[4], "dest_hi", self.destination_hi)?;
        let pk_tag = pinned(&mut layouter, config.advice[5], "pk-tag", personalization_to_fp(TAG_PKCOM))?;
        let pk_com = poseidon(&mut layouter, &config, "pk_com", [pk_tag, px.native.clone(), py.native.clone()])?;
        let reg = REG_MODE.load(core::sync::atomic::Ordering::SeqCst);
        let nf_tag = pinned(&mut layouter, config.advice[5], "nf-tag", personalization_to_fp(if reg { TAG_REG_NF } else { TAG_NULLIFIER }))?;
        let nullifier = poseidon(&mut layouter, &config, "nullifier", [nf_tag, sk.native.clone(), ctx.clone()])?;
        let leaf_tag = pinned(&mut layouter, config.advice[5], "leaf-tag", personalization_to_fp(TAG_NOTE_LEAF))?;
        let mut node = poseidon(&mut layouter, &config, "leaf", [leaf_tag.clone(), pk_com, amount.clone()])?;
        for layer in 0..NOTE_DEPTH {
            let sibling = self.path.map(|w| w[layer]);
            let bit = self.position.map(|p| (p >> layer) & 1 == 1);
            let (l, r) = config.swap.swap(layouter.namespace(|| "order"), (node, sibling), bit)?;
            let tag = pinned(&mut layouter, config.advice[5], "crh-tag", distro_crh_tag())?;
            let lc = pinned(&mut layouter, config.advice[4], "layer", pallas::Base::from(layer as u64))?;
            node = poseidon(&mut layouter, &config, "crh", [tag, lc, l, r])?;
        }
        if reg {
            let kc_tag = pinned(&mut layouter, config.advice[5], "kc-tag", personalization_to_fp(TAG_KEYCOM))?;
            let key_com = poseidon(&mut layouter, &config, "key_com", [kc_tag, dest_hi])?;
            let reg_leaf = poseidon(&mut layouter, &config, "reg_leaf", [leaf_tag, key_com, amount])?;
            for (i, cell) in [ctx, node, nullifier, dest_lo, reg_leaf].iter().enumerate() {
                layouter.constrain_instance(cell.cell(), config.instance, i)?;
            }
            return Ok(());
        }
        for (i, cell) in [ctx, node, nullifier, dest_lo, dest_hi, amount].iter().enumerate() {
            layouter.constrain_instance(cell.cell(), config.instance, i)?;
        }
        Ok(())
    }
}

fn load(
    layouter: &mut impl Layouter<pallas::Base>,
    column: Column<Advice>,
    label: &'static str,
    value: Value<pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    layouter.assign_region(|| label, |mut region| region.assign_advice(|| label, column, 0, || value))
}

fn pinned(
    layouter: &mut impl Layouter<pallas::Base>,
    column: Column<Advice>,
    label: &'static str,
    value: pallas::Base,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    layouter.assign_region(
        || label,
        |mut region| {
            let cell = region.assign_advice(|| label, column, 0, || Value::known(value))?;
            region.constrain_constant(cell.cell(), value)?;
            Ok(cell)
        },
    )
}

fn poseidon<const L: usize>(
    layouter: &mut impl Layouter<pallas::Base>,
    config: &BlindConfig,
    label: &'static str,
    inputs: [AssignedCell<pallas::Base, pallas::Base>; L],
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    let chip = PoseidonChip::construct(config.poseidon.clone());
    let h = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<L>, 3, 2>::init(chip, layouter.namespace(|| label))?;
    h.hash(layouter.namespace(|| label), inputs)
}

fn host_poseidon<const L: usize>(x: [pallas::Base; L]) -> pallas::Base {
    poseidon_p::Hash::<_, P128Pow5T3, ConstantLength<L>, 3, 2>::init().hash(x)
}

/// Host helpers. `sk_be` is a test or wallet secret; never logged.
pub fn blind_pk_com(pkx_native: pallas::Base, pky_native: pallas::Base) -> pallas::Base {
    host_poseidon([personalization_to_fp(TAG_PKCOM), pkx_native, pky_native])
}

pub fn blind_nullifier(sk_native: pallas::Base, ctx: pallas::Base) -> pallas::Base {
    host_poseidon([personalization_to_fp(TAG_NULLIFIER), sk_native, ctx])
}

/// Step-1 registration fixture (requires REG_MODE at synthesis).
pub fn reg_fixture(
    sk_be: [u8; 32],
    ctx: pallas::Base,
    amount: u64,
    position: u32,
    path: [pallas::Base; NOTE_DEPTH],
    relayer: pallas::Base,
    secret: pallas::Base,
    any_dest: &str,
) -> Result<BlindFixture, String> {
    use crate::circuit::gadget::secp256k1_chip::secp_secret_be_to_pallas_base;
    let mut f = blind_fixture(sk_be, ctx, amount, position, path, any_dest)?;
    f.circuit.destination_lo = Value::known(relayer);
    f.circuit.destination_hi = Value::known(secret);
    let reg_nf = host_poseidon([personalization_to_fp(TAG_REG_NF), secp_secret_be_to_pallas_base(&sk_be), ctx]);
    let reg_leaf = crate::note_membership::note_leaf(crate::no_ecdsa::key_commitment(secret), amount);
    f.public = vec_of([ctx, f.public[1], reg_nf, relayer, reg_leaf]);
    Ok(f)
}

#[derive(Debug)]
pub struct BlindFixture {
    pub circuit: BlindClaimCircuit,
    pub public: Vec<pallas::Base>,
}

/// Build a fixture from a secret key, a tree position and siblings.
pub fn blind_fixture(
    sk_be: [u8; 32],
    ctx: pallas::Base,
    amount: u64,
    position: u32,
    path: [pallas::Base; NOTE_DEPTH],
    destination: &str,
) -> Result<BlindFixture, String> {
    use crate::circuit::gadget::secp256k1_chip::{
        secp_coord_be_to_pallas_base, secp_fp_from_coord_be, secp_fq_from_secret_be, secp_secret_be_to_pallas_base,
    };
    let secp = secp256k1::Secp256k1::new();
    let sk = secp256k1::SecretKey::from_byte_array(sk_be).map_err(|e| format!("sk {e}"))?;
    let raw = secp256k1::PublicKey::from_secret_key(&secp, &sk).serialize_uncompressed();
    let x: [u8; 32] = raw[1..33].try_into().unwrap();
    let y: [u8; 32] = raw[33..65].try_into().unwrap();
    let pk_com = blind_pk_com(secp_coord_be_to_pallas_base(&x), secp_coord_be_to_pallas_base(&y));
    let root = note_root(note_leaf_blind(pk_com, amount), position, &path);
    let nf = blind_nullifier(secp_secret_be_to_pallas_base(&sk_be), ctx);
    let (lo, hi) = crate::no_ecdsa::pack_destination(destination)?;
    let public = vec_of([ctx, root, nf, lo, hi, pallas::Base::from(amount)]);
    let circuit = BlindClaimCircuit {
        sk: Value::known(secp_fq_from_secret_be(&sk_be)),
        pkx: Value::known(secp_fp_from_coord_be(&x)),
        pky: Value::known(secp_fp_from_coord_be(&y)),
        ctx: Value::known(ctx),
        amount: Value::known(amount),
        position: Value::known(position),
        path: Value::known(path),
        destination_lo: Value::known(lo),
        destination_hi: Value::known(hi),
    };
    Ok(BlindFixture { circuit, public })
}

fn vec_of<const N: usize>(a: [pallas::Base; N]) -> Vec<pallas::Base> {
    a.to_vec()
}

/// Same leaf as vk3 (`note_leaf` takes a key commitment); here the commitment is over pk.
pub fn note_leaf_blind(pk_com: pallas::Base, amount: u64) -> pallas::Base {
    note_leaf(pk_com, amount)
}

#[allow(dead_code)]
fn _unused(_: pallas::Base) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use halo2_proofs::dev::MockProver;

    const SK: [u8; 32] = [0x11; 32];
    const DEST: &str = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";

    fn fx() -> BlindFixture {
        let path = core::array::from_fn(|i| pallas::Base::from(1000 + i as u64));
        blind_fixture(SK, pallas::Base::from(0x11u64), crate::note_membership::DENOMS_UTERP[0], 5, path, DEST).unwrap()
    }

    fn k() -> u32 {
        std::env::var("BLIND_K").ok().and_then(|s| s.parse().ok()).unwrap_or(BLIND_K)
    }

    fn ok(c: &BlindClaimCircuit, p: &[pallas::Base]) -> bool {
        MockProver::run(k(), c, vec![p.to_vec()]).unwrap().verify().is_ok()
    }

    /// MockProver verdict under a forge mode; a synthesis panic is reported as `None`.
    fn ok_forged(mode: u8, c: &BlindClaimCircuit, p: &[pallas::Base]) -> Option<bool> {
        use crate::circuit::gadget::secp256k1_chip::forge;
        forge::set(mode);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ok(c, p)));
        forge::set(0);
        r.ok()
    }

    /// ADVERSARIAL (formerly `blind_forgery_accepted_unsound_sub`): the old chip let a prover
    /// witness `a - b = 0` inside `enforce_equal` for unequal a, b. The sound sub gate binds the
    /// output, so the same forgery must now be REJECTED.
    #[test]
    #[ignore = "K=19 MockProver"]
    fn blind_forgery_rejected() {
        use crate::circuit::gadget::secp256k1_chip::forge;
        let f = fx();
        let mut c = f.circuit.clone();
        c.sk = Value::known(crate::circuit::gadget::secp256k1_chip::secp_fq_from_secret_be(&[0x22; 32]));
        let honest_reject = !ok(&c, &f.public);
        let mut forged_pub = f.public.clone();
        let fake_sk_native = {
            use ff::PrimeField;
            pallas::Base::from_repr([0x22u8; 32]).unwrap()
        };
        forged_pub[2] = blind_nullifier(fake_sk_native, pallas::Base::from(0x11u64));
        let forged = ok_forged(forge::EQ_SUB, &c, &forged_pub);
        std::println!("FORGERY honest_wrong_sk_rejected={honest_reject} forged_accepted={forged:?}");
        assert!(honest_reject);
        assert_eq!(forged, Some(false), "eq-sub forgery must be rejected by constraints");
    }

    /// Each forge mode tampers one witness of the HONEST claim; every one must be rejected by
    /// the constraint system (not by a synthesis panic).
    #[test]
    #[ignore = "K=19 MockProver"]
    fn blind_negative_suite() {
        use crate::circuit::gadget::secp256k1_chip::forge;
        let f = fx();
        assert!(ok(&f.circuit, &f.public), "honest claim");
        let cases = [
            (forge::ADD_OUT, "forged add output (GLV path)"),
            (forge::SUB_OUT, "forged sub output"),
            (forge::MUL_OUT, "forged mul output"),
            (forge::MUL_Q_RANGE, "mul quotient out of range"),
            (forge::MUL_R_RANGE, "mul remainder limb out of range"),
            (forge::CONST, "unpinned constant"),
            (forge::SK_PLUS_N, "sk >= n (sk + n)"),
            (forge::FB_DIGIT, "fixed-base digit != sk limb"),
            (forge::FB_POINT, "fixed-base point not from table"),
        ];
        let mut bad = vec![];
        for (m, name) in cases {
            // fp.add only occurs in doublings; the fixed-base path has none, so exercise the
            // add gate through the GLV path (same chip, same gate).
            let glv = m == forge::ADD_OUT;
            crate::circuit::gadget::secp256k1_chip::KEYPAIR_GLV.store(glv, core::sync::atomic::Ordering::SeqCst);
            let r = ok_forged(m, &f.circuit, &f.public);
            crate::circuit::gadget::secp256k1_chip::KEYPAIR_GLV.store(false, core::sync::atomic::Ordering::SeqCst);
            std::println!("NEG {name}: accepted={r:?}");
            if r != Some(false) { bad.push(name); }
        }
        assert!(ok(&f.circuit, &f.public), "honest claim after forges");
        assert!(bad.is_empty(), "not rejected by constraints: {bad:?}");
    }

    #[test]
    #[ignore = "K=19 MockProver"]
    fn blind_soundness_suite() {
        let f = fx();
        assert!(ok(&f.circuit, &f.public), "honest claim");
        // wrong key: a different sk for the same pk
        let mut c = f.circuit.clone();
        c.sk = Value::known(crate::circuit::gadget::secp256k1_chip::secp_fq_from_secret_be(&[0x22; 32]));
        assert!(!ok(&c, &f.public), "wrong key must fail");
        // wrong Merkle path
        let mut c = f.circuit.clone();
        c.path = c.path.map(|mut p| { p[3] += pallas::Base::ONE; p });
        assert!(!ok(&c, &f.public), "wrong path must fail");
        // wrong position
        let mut c = f.circuit.clone();
        c.position = Value::known(6);
        assert!(!ok(&c, &f.public), "wrong position must fail");
        // public nullifier, ctx, destination, amount are bound
        for slot in [0usize, 2, 3, 4, 5] {
            let mut p = f.public.clone();
            p[slot] += pallas::Base::ONE;
            assert!(!ok(&f.circuit, &p), "changed public slot {slot} must fail");
        }
        std::println!("blind soundness suite: all 9 negative cases rejected, honest accepted (K={})", k());
    }

    #[test]
    #[ignore = "K=19 bench"]
    fn lab_bench_blind_ladder() {
        use crate::circuit::gadget::secp256k1_chip::KEYPAIR_LADDER;
        KEYPAIR_LADDER.store(true, core::sync::atomic::Ordering::SeqCst);
        let f = fx();
        let mut bad = f.circuit.clone();
        bad.sk = Value::known(crate::circuit::gadget::secp256k1_chip::secp_fq_from_secret_be(&[0x22; 32]));
        std::println!("LADDER honest_ok={} wrong_sk_rejected={}", ok(&f.circuit, &f.public), !ok(&bad, &f.public));
        crate::lab_bench::measure("k19_blind_keypair_ladder", k(), f.circuit, f.public, crate::lab_bench::runs_pub());
    }

    fn rfx() -> BlindFixture {
        let path = core::array::from_fn(|i| pallas::Base::from(1000 + i as u64));
        reg_fixture(SK, pallas::Base::from(0x77u64), crate::note_membership::DENOMS_UTERP[0], 5, path,
            pallas::Base::from(0xabcdu64), pallas::Base::from(0x5eed0001u64), DEST).unwrap()
    }

    #[test]
    #[ignore = "K=19 bench"]
    fn lab_bench_reg() {
        REG_MODE.store(true, core::sync::atomic::Ordering::SeqCst);
        let f = rfx();
        let mut bad = f.circuit.clone();
        bad.sk = Value::known(crate::circuit::gadget::secp256k1_chip::secp_fq_from_secret_be(&[0x22; 32]));
        let mut badpath = f.circuit.clone();
        badpath.position = Value::known(6);
        let mut badsec = f.circuit.clone();
        badsec.destination_hi = Value::known(pallas::Base::from(9u64));
        let mut relay = f.public.clone();
        relay[3] = pallas::Base::from(1u64);
        std::println!("REG honest_ok={} wrong_sk_rej={} wrong_pos_rej={} wrong_secret_rej={} relayer_swap_rej={}",
            ok(&f.circuit, &f.public), !ok(&bad, &f.public), !ok(&badpath, &f.public),
            !ok(&badsec, &f.public), !ok(&f.circuit, &relay));
        crate::lab_bench::measure("k19_reg_step1_glv", k(), f.circuit, f.public, crate::lab_bench::runs_pub());
    }

    #[test]
    #[ignore = "lab bench"]
    fn lab_bench_blind() {
        let f = fx();
        crate::lab_bench::measure("k19_blind_keypair", k(), f.circuit, f.public, crate::lab_bench::runs_pub());
    }
}
