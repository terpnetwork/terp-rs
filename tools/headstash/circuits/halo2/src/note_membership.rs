//! Fixed-denomination note membership (rev 5, vk_id 3).
//!
//! One claim-tree leaf per note: `leaf = Poseidon(TAG_NOTE_LEAF, key_com, amount)`
//! where `amount` is the note's denomination value in uterp (a public input, so the
//! contract checks it is in the campaign denomination table). Depth 17 holds
//! 131,072 notes (the production decomposition has 98,841).
//!
//! Differences from the K=11 vk_id 2 circuit in `no_ecdsa`:
//! - depth 17 instead of 16;
//! - new leaf personalization `terp-hs-note-leaf-v2` (old and new claim leaves never coincide);
//! - every Poseidon personalization and every Merkle layer index is pinned to a
//!   fixed constant (`constrain_constant`). In vk_id 2 these were free advice
//!   cells, so a prover could pick any nullifier tag and mint unlimited
//!   nullifiers for one secret (see security review addendum, finding N1).
//!
//! Public inputs (unchanged layout): ctx, root, nullifier, dest_lo, dest_hi, amount.

use std::string::{String, ToString};
use std::vec::Vec;

use ff::{Field, PrimeField};
use halo2_gadgets::{
    poseidon::{
        primitives::{ConstantLength, P128Pow5T3},
        Hash as PoseidonHash, Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig,
    },
    utilities::cond_swap::{CondSwapChip, CondSwapInstructions},
};
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, SimpleFloorPlanner, Value},
    plonk::{self, Advice, Circuit, Column, ConstraintSystem, Instance},
    poly::commitment::Params,
    transcript::{Blake2bWrite, Challenge255},
};
use pasta_curves::{pallas, vesta};

use crate::distro_poseidon::{distro_crh_tag, personalization_to_fp, poseidon_distro_crh};
use crate::no_ecdsa::{claim_nullifier, key_commitment, pack_destination, verify_membership, MembershipProof};

pub const NOTE_DEPTH: usize = 17;
pub const NOTE_VK_ID: u64 = 3;
/// Measured minimum K for depth 17 (see `note_k_is_minimal`).
pub const NOTE_K: u32 = 11;
pub const NOTE_INSTANCES: u8 = 6;

pub const TAG_KEYCOM: &str = "terp-hs-keycom-v1";
pub const TAG_NULLIFIER: &str = "terp-hs-nf-v1";
pub const TAG_NOTE_LEAF: &str = "terp-hs-note-leaf-v2";

/// 1-2-5 series, 1 TERP .. 5,000 TERP, in uterp. Index = denom_class.
pub const DENOMS_UTERP: [u64; 12] = [
    1_000_000, 2_000_000, 5_000_000, 10_000_000, 20_000_000, 50_000_000, 100_000_000,
    200_000_000, 500_000_000, 1_000_000_000, 2_000_000_000, 5_000_000_000,
];

pub fn denom_class_of(amount: u64) -> Option<u8> {
    DENOMS_UTERP.iter().position(|d| *d == amount).map(|i| i as u8)
}

#[derive(Clone, Debug)]
pub struct NoteConfig {
    advice: [Column<Advice>; 6],
    poseidon: PoseidonConfig<pallas::Base, 3, 2>,
    swap: CondSwapChip<pallas::Base>,
    instance: Column<Instance>,
}

/// `PIN = true` is the production circuit. `PIN = false` reproduces the vk_id 2
/// shape (free tag cells) and exists only so tests can demonstrate finding N1.
#[derive(Clone, Debug)]
pub struct NoteCircuitImpl<const PIN: bool> {
    pub secret: Value<pallas::Base>,
    pub ctx: Value<pallas::Base>,
    pub amount: Value<u64>,
    pub position: Value<u32>,
    pub path: Value<[pallas::Base; NOTE_DEPTH]>,
    pub destination_lo: Value<pallas::Base>,
    pub destination_hi: Value<pallas::Base>,
    /// Test hook: witness this value in the nullifier tag cell instead of the real tag.
    pub nf_tag_witness: Option<pallas::Base>,
}

pub type NoteMembershipCircuit = NoteCircuitImpl<true>;

impl<const PIN: bool> Default for NoteCircuitImpl<PIN> {
    fn default() -> Self {
        Self {
            secret: Value::unknown(),
            ctx: Value::unknown(),
            amount: Value::unknown(),
            position: Value::unknown(),
            path: Value::unknown(),
            destination_lo: Value::unknown(),
            destination_hi: Value::unknown(),
            nf_tag_witness: None,
        }
    }
}

impl<const PIN: bool> Circuit<pallas::Base> for NoteCircuitImpl<PIN> {
    type Config = NoteConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        let instance = meta.instance_column();
        meta.enable_equality(instance);
        let advice = [
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
        ];
        for column in &advice {
            meta.enable_equality(*column);
        }
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
        let swap_config = CondSwapChip::configure(meta, advice[0..5].try_into().unwrap());
        NoteConfig { advice, poseidon, swap: CondSwapChip::construct(swap_config), instance }
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), plonk::Error> {
        let secret = load(&mut layouter, config.advice[0], "secret", self.secret)?;
        let ctx = load(&mut layouter, config.advice[1], "ctx", self.ctx)?;
        let amount = load(&mut layouter, config.advice[2], "amount", self.amount.map(pallas::Base::from))?;
        let dest_lo = load(&mut layouter, config.advice[3], "dest_lo", self.destination_lo)?;
        let dest_hi = load(&mut layouter, config.advice[4], "dest_hi", self.destination_hi)?;

        let tag_kc = personalization_to_fp(TAG_KEYCOM);
        let tag_nf = personalization_to_fp(TAG_NULLIFIER);
        let tag_leaf = personalization_to_fp(TAG_NOTE_LEAF);
        let kc_tag = fixed::<PIN>(&mut layouter, config.advice[5], "kc-tag", tag_kc, None)?;
        let key_com = poseidon(&mut layouter, &config, "key_com", [kc_tag, secret.clone()])?;
        let nf_tag = fixed::<PIN>(&mut layouter, config.advice[5], "nf-tag", tag_nf, self.nf_tag_witness)?;
        let nullifier = poseidon(&mut layouter, &config, "nullifier", [nf_tag, secret, ctx.clone()])?;
        let leaf_tag = fixed::<PIN>(&mut layouter, config.advice[5], "leaf-tag", tag_leaf, None)?;
        let leaf = poseidon(&mut layouter, &config, "leaf", [leaf_tag, key_com, amount.clone()])?;

        let mut node = leaf;
        for layer in 0..NOTE_DEPTH {
            let sibling = self.path.map(|words| words[layer]);
            let swap_bit = self.position.map(|pos| (pos >> layer) & 1 == 1);
            let (left, right) = config.swap.swap(layouter.namespace(|| "order"), (node, sibling), swap_bit)?;
            let tag = fixed::<PIN>(&mut layouter, config.advice[5], "crh-tag", distro_crh_tag(), None)?;
            let lc = fixed::<PIN>(&mut layouter, config.advice[4], "layer", pallas::Base::from(layer as u64), None)?;
            node = poseidon(&mut layouter, &config, "crh", [tag, lc, left, right])?;
        }

        let public = [ctx, node, nullifier, dest_lo, dest_hi, amount];
        for (offset, cell) in public.iter().enumerate() {
            layouter.constrain_instance(cell.cell(), config.instance, offset)?;
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

/// A domain constant. With PIN the cell is copy-constrained to the fixed
/// constant column, so the prover cannot choose it.
fn fixed<const PIN: bool>(
    layouter: &mut impl Layouter<pallas::Base>,
    column: Column<Advice>,
    label: &'static str,
    value: pallas::Base,
    witness: Option<pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    let w = witness.unwrap_or(value);
    layouter.assign_region(
        || label,
        |mut region| {
            let cell = region.assign_advice(|| label, column, 0, || Value::known(w))?;
            if PIN {
                region.constrain_constant(cell.cell(), value)?;
            }
            Ok(cell)
        },
    )
}

fn poseidon<const L: usize>(
    layouter: &mut impl Layouter<pallas::Base>,
    config: &NoteConfig,
    label: &'static str,
    inputs: [AssignedCell<pallas::Base, pallas::Base>; L],
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    let chip = PoseidonChip::construct(config.poseidon.clone());
    let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<L>, 3, 2>::init(chip, layouter.namespace(|| label))?;
    hasher.hash(layouter.namespace(|| label), inputs)
}

pub fn note_leaf(key_com: pallas::Base, amount: u64) -> pallas::Base {
    halo2_gadgets::poseidon::primitives::Hash::<_, P128Pow5T3, ConstantLength<3>, 3, 2>::init()
        .hash([personalization_to_fp(TAG_NOTE_LEAF), key_com, pallas::Base::from(amount)])
}

pub fn note_root(leaf: pallas::Base, position: u32, path: &[pallas::Base; NOTE_DEPTH]) -> pallas::Base {
    let mut node = leaf;
    for (layer, sibling) in path.iter().enumerate() {
        node = if position & (1 << layer) == 0 {
            poseidon_distro_crh(layer as u32, node, *sibling)
        } else {
            poseidon_distro_crh(layer as u32, *sibling, node)
        };
    }
    node
}

pub fn note_canonical_body(
    ctx: pallas::Base,
    root: pallas::Base,
    nullifier: pallas::Base,
    destination: &str,
    amount: u64,
) -> String {
    format!(
        "headstash-note-claim-v2:{}:{}:{}:{}:{}",
        hex::encode(ctx.to_repr()),
        hex::encode(root.to_repr()),
        hex::encode(nullifier.to_repr()),
        destination,
        amount
    )
}

pub fn note_public(
    secret: pallas::Base,
    ctx: pallas::Base,
    amount: u64,
    root: pallas::Base,
    destination: &str,
) -> Result<Vec<pallas::Base>, String> {
    let (lo, hi) = pack_destination(destination)?;
    Ok(vec![ctx, root, claim_nullifier(secret, ctx), lo, hi, pallas::Base::from(amount)])
}

pub fn note_params() -> Params<vesta::Affine> {
    Params::<vesta::Affine>::new(NOTE_K)
}

pub fn note_vk(params: &Params<vesta::Affine>) -> Result<plonk::VerifyingKey<vesta::Affine>, String> {
    plonk::keygen_vk(params, &NoteMembershipCircuit::default()).map_err(|err| format!("vk {err:?}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> Option<std::time::Instant> {
    Some(std::time::Instant::now())
}
#[cfg(target_arch = "wasm32")]
fn now() -> Option<()> {
    None
}
#[cfg(not(target_arch = "wasm32"))]
fn secs(t: Option<std::time::Instant>) -> f64 {
    t.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0)
}
#[cfg(target_arch = "wasm32")]
fn secs(_: Option<()>) -> f64 {
    0.0
}

/// Prove one note. Fails if the amount is not a denomination.
pub fn prove_note(
    secret: pallas::Base,
    ctx: pallas::Base,
    amount: u64,
    position: u32,
    path: [pallas::Base; NOTE_DEPTH],
    destination: &str,
) -> Result<MembershipProof, String> {
    if denom_class_of(amount).is_none() {
        return Err("amount".into());
    }
    if (position as u64) >= 1u64 << NOTE_DEPTH {
        return Err("index".into());
    }
    let (dest_lo, dest_hi) = pack_destination(destination)?;
    let root = note_root(note_leaf(key_commitment(secret), amount), position, &path);
    let public = note_public(secret, ctx, amount, root, destination)?;
    let circuit = NoteMembershipCircuit {
        secret: Value::known(secret),
        ctx: Value::known(ctx),
        amount: Value::known(amount),
        position: Value::known(position),
        path: Value::known(path),
        destination_lo: Value::known(dest_lo),
        destination_hi: Value::known(dest_hi),
        nf_tag_witness: None,
    };
    let params = note_params();
    let vk = note_vk(&params)?;
    let pk = plonk::keygen_pk(&params, vk, &circuit).map_err(|err| format!("pk {err:?}"))?;
    let mut pk_buf = Vec::new();
    pk.write(&mut pk_buf).map_err(|err| err.to_string())?;
    let started = now();
    let mut rng = crate::os_rng();
    let mut transcript = Blake2bWrite::<_, vesta::Affine, Challenge255<_>>::init(Vec::new());
    plonk::create_proof(&params, &pk, &[circuit], &[&[&public]], &mut rng, &mut transcript)
        .map_err(|err| format!("prove {err:?}"))?;
    let proof = transcript.finalize();
    let prove_secs = secs(started);
    let started = now();
    verify_membership(&params, pk.get_vk(), &proof, &public)?;
    let verify_secs = secs(started);
    Ok(MembershipProof {
        k: NOTE_K,
        proof,
        body: note_canonical_body(ctx, root, public[2], destination, amount),
        public,
        prove_secs,
        verify_secs,
        pk_bytes: pk_buf.len(),
    })
}

fn checksum32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(cosmwasm_std::Checksum::generate(bytes).as_slice());
    out
}

/// Params, constraint system, verifying key and the 80-byte footer (same layout as vk_id 2).
pub fn note_circuit_blob() -> Result<Vec<u8>, String> {
    let params = note_params();
    let vk = note_vk(&params)?;
    let mut buf = Vec::new();
    params.write(&mut buf).map_err(|_| "circuit".to_string())?;
    let param_len = buf.len();
    vk.cs().write(&mut buf).map_err(|_| "circuit".to_string())?;
    let cs_len = buf.len() - param_len;
    vk.write(&mut buf).map_err(|_| "circuit".to_string())?;
    let vk_len = buf.len() - cs_len - param_len;
    let param_hash = checksum32(&buf[..param_len]);
    let vk_hash = checksum32(&buf[param_len..]);
    buf.extend_from_slice(
        &zk_cosmwasm::CircuitFooter::new(
            zk_cosmwasm::CircuitType::Plonkish,
            zk_cosmwasm::curves::CurveType::Pasta,
            NOTE_K as u8,
            NOTE_INSTANCES,
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

pub fn note_vk_hash() -> Result<[u8; 32], String> {
    Ok(checksum32(&note_circuit_blob()?))
}

fn parse_fp(hex_str: &str) -> Result<pallas::Base, String> {
    if hex_str.len() != 64 || !hex_str.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err("field".into());
    }
    let bytes = hex::decode(hex_str).map_err(|_| "field".to_string())?;
    let mut repr = [0u8; 32];
    repr.copy_from_slice(&bytes);
    Option::from(pallas::Base::from_repr(repr)).ok_or_else(|| "field".to_string())
}

struct Witness {
    secret: pallas::Base,
    ctx: pallas::Base,
    root: pallas::Base,
    amount: u64,
    index: u32,
    path: [pallas::Base; NOTE_DEPTH],
    destination: String,
}

fn parse_witness(witness_json: &str) -> Result<Witness, String> {
    let v: serde_json::Value = serde_json::from_str(witness_json).map_err(|_| "witness".to_string())?;
    let field = |k: &str| -> Result<pallas::Base, String> { parse_fp(v.get(k).and_then(|x| x.as_str()).ok_or(k.to_string())?) };
    let amount = match v.get("amount").ok_or("amount")? {
        serde_json::Value::String(s) => s.parse::<u64>().map_err(|_| "amount")?,
        serde_json::Value::Number(n) => n.as_u64().ok_or("amount")?,
        _ => return Err("amount".into()),
    };
    if denom_class_of(amount).is_none() {
        return Err("amount".into());
    }
    let index = v.get("index").and_then(|x| x.as_u64()).ok_or("index")?;
    if index >= 1u64 << NOTE_DEPTH {
        return Err("index".into());
    }
    let sib = v.get("siblings_hex").and_then(|x| x.as_array()).ok_or("siblings")?;
    if sib.len() != NOTE_DEPTH {
        return Err("siblings".into());
    }
    let mut path = [pallas::Base::ZERO; NOTE_DEPTH];
    for (i, s) in sib.iter().enumerate() {
        path[i] = parse_fp(s.as_str().ok_or("sibling")?)?;
    }
    Ok(Witness {
        secret: field("secret_hex")?,
        ctx: field("ctx_hex")?,
        root: field("root_hex")?,
        amount,
        index: index as u32,
        path,
        destination: v.get("destination").and_then(|x| x.as_str()).ok_or("destination")?.to_string(),
    })
}

/// Prove a note witness (`secret_hex, ctx_hex, root_hex, amount, index, siblings_hex[17], destination`).
pub fn prove_note_witness_json(witness_json: &str) -> Result<MembershipProof, String> {
    let w = parse_witness(witness_json)?;
    if note_root(note_leaf(key_commitment(w.secret), w.amount), w.index, &w.path) != w.root {
        return Err("root".into());
    }
    prove_note(w.secret, w.ctx, w.amount, w.index, w.path, &w.destination)
}

/// Native verifier for a note proof under vk_id 3.
pub fn verify_note_witness_proof(witness_json: &str, proof: &[u8]) -> Result<(), String> {
    let w = parse_witness(witness_json)?;
    let public = note_public(w.secret, w.ctx, w.amount, w.root, &w.destination)?;
    let params = note_params();
    let vk = note_vk(&params)?;
    verify_membership(&params, &vk, proof, &public)
}

pub fn minimal_note_k(circuit: &NoteMembershipCircuit, public: &[pallas::Base]) -> Option<u32> {
    (8..20).find(|k| {
        halo2_proofs::dev::MockProver::run(*k, circuit, vec![public.to_vec()])
            .map(|p| p.verify().is_ok())
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_leaf_helper_matches_circuit_leaf() {
        let k = pallas::Base::from(77u64);
        for amount in DENOMS_UTERP {
            assert_eq!(
                note_leaf(k, amount),
                crate::distro_poseidon::note_registration_leaf(k, amount)
            );
        }
    }

    const DEST: &str = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";

    fn fixture(amount: u64) -> (NoteMembershipCircuit, Vec<pallas::Base>, pallas::Base, pallas::Base) {
        let secret = pallas::Base::from(0x31u64);
        let ctx = pallas::Base::from(0x44u64);
        let index = 98_840u32;
        let mut path = [pallas::Base::ZERO; NOTE_DEPTH];
        path[0] = pallas::Base::from(7u64);
        path[16] = pallas::Base::from(9u64);
        let root = note_root(note_leaf(key_commitment(secret), amount), index, &path);
        let (lo, hi) = pack_destination(DEST).unwrap();
        let c = NoteMembershipCircuit {
            secret: Value::known(secret),
            ctx: Value::known(ctx),
            amount: Value::known(amount),
            position: Value::known(index),
            path: Value::known(path),
            destination_lo: Value::known(lo),
            destination_hi: Value::known(hi),
            nf_tag_witness: None,
        };
        (c, note_public(secret, ctx, amount, root, DEST).unwrap(), secret, ctx)
    }

    pub(crate) fn sample_witness_json() -> String {
        let secret = pallas::Base::from(0x31u64);
        let ctx = pallas::Base::from(0x44u64);
        let amount = 5_000_000_000u64;
        let index = 98_840u32;
        let mut path = [pallas::Base::ZERO; NOTE_DEPTH];
        path[0] = pallas::Base::from(7u64);
        path[16] = pallas::Base::from(9u64);
        let root = note_root(note_leaf(key_commitment(secret), amount), index, &path);
        let siblings: Vec<String> = path.iter().map(|x| hex::encode(x.to_repr())).collect();
        serde_json::json!({
            "secret_hex": hex::encode(secret.to_repr()), "ctx_hex": hex::encode(ctx.to_repr()),
            "root_hex": hex::encode(root.to_repr()), "amount": amount.to_string(), "index": index,
            "siblings_hex": siblings, "destination": DEST,
        })
        .to_string()
    }

    #[test]
    fn note_k_is_minimal() {
        let (c, public, _, _) = fixture(5_000_000_000);
        assert_eq!(minimal_note_k(&c, &public), Some(NOTE_K));
    }

    #[test]
    fn pinned_tags_reject_a_forged_nullifier() {
        let fake = pallas::Base::from(0xdeadu64);
        for pinned in [false, true] {
            let (c, mut public, secret, ctx) = fixture(1_000_000);
            public[2] = halo2_gadgets::poseidon::primitives::Hash::<_, P128Pow5T3, ConstantLength<3>, 3, 2>::init()
                .hash([fake, secret, ctx]);
            let ok = if pinned {
                let c = NoteCircuitImpl::<true> { nf_tag_witness: Some(fake), ..c };
                halo2_proofs::dev::MockProver::run(NOTE_K, &c, vec![public]).unwrap().verify().is_ok()
            } else {
                let c = NoteCircuitImpl::<false> {
                    secret: c.secret, ctx: c.ctx, amount: c.amount, position: c.position, path: c.path,
                    destination_lo: c.destination_lo, destination_hi: c.destination_hi, nf_tag_witness: Some(fake),
                };
                halo2_proofs::dev::MockProver::run(NOTE_K, &c, vec![public]).unwrap().verify().is_ok()
            };
            // Unpinned (vk_id 2 shape): a second nullifier for the same note verifies.
            assert_eq!(ok, !pinned, "pinned={pinned}");
        }
    }

    #[test]
    fn wrong_amount_or_root_fails() {
        let (c, mut public, _, _) = fixture(2_000_000);
        public[5] = pallas::Base::from(5_000_000u64);
        assert!(halo2_proofs::dev::MockProver::run(NOTE_K, &c, vec![public.clone()]).unwrap().verify().is_err());
        public[5] = pallas::Base::from(2_000_000u64);
        public[1] += pallas::Base::ONE;
        assert!(halo2_proofs::dev::MockProver::run(NOTE_K, &c, vec![public]).unwrap().verify().is_err());
    }

    #[test]
    fn denominations() {
        assert_eq!(denom_class_of(1_000_000), Some(0));
        assert_eq!(denom_class_of(5_000_000_000), Some(11));
        assert_eq!(denom_class_of(15), None);
        assert!(prove_note(pallas::Base::ONE, pallas::Base::ONE, 15, 0, [pallas::Base::ZERO; NOTE_DEPTH], DEST).is_err());
        assert_ne!(note_leaf(pallas::Base::ONE, 1_000_000), crate::no_ecdsa::membership_leaf(pallas::Base::ONE, 1_000_000));
    }

    #[test]
    fn note_proof_verifies_and_vk_is_recorded() {
        let witness = sample_witness_json();
        let proof = prove_note_witness_json(&witness).expect("prove");
        verify_note_witness_proof(&witness, &proof.proof).expect("verify");
        let blob = note_circuit_blob().unwrap();
        let hash = hex::encode(checksum32(&blob));
        std::eprintln!(
            "note k {} proof_bytes {} prove_secs {:.3} verify_secs {:.3} pk_bytes {} vk_blob_bytes {} vk_hash {}",
            proof.k, proof.proof.len(), proof.prove_secs, proof.verify_secs, proof.pk_bytes, blob.len(), hash
        );
        assert_eq!(NOTE_VK_ID, 3);
        assert_eq!(&blob[blob.len() - 80..][2], &(NOTE_K as u8));
        assert_eq!(hash, NOTE_VK_HASH);
        assert_eq!(proof.proof.len(), NOTE_PROOF_BYTES);
        if let Ok(p) = std::env::var("HS_WRITE_NOTE_WITNESS") {
            std::fs::write(p, &witness).unwrap();
        }
    }

    #[test]
    #[ignore = "HS_NOTE_WITNESS and HS_NOTE_BROWSER_PROOF from the wasm build"]
    fn browser_note_proof_matches_native_vk() {
        let witness = std::fs::read_to_string(std::env::var("HS_NOTE_WITNESS").unwrap()).unwrap();
        let hexp = std::fs::read_to_string(std::env::var("HS_NOTE_BROWSER_PROOF").unwrap()).unwrap();
        let proof = hex::decode(hexp.trim()).unwrap();
        assert_eq!(proof.len(), NOTE_PROOF_BYTES);
        verify_note_witness_proof(witness.trim(), &proof).expect("verify");
    }
}

/// sha256 (cosmwasm Checksum) of `note_circuit_blob()`.
pub const NOTE_VK_HASH: &str = "34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398";
pub const NOTE_PROOF_BYTES: usize = 2752;
