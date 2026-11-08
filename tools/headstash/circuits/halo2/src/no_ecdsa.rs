//! Membership claim without an in-circuit secp256k1 gadget.
//!
//! The registrar checks the eligible key's signature natively and stores
//! `key_com = Poseidon(secret)`. This circuit proves that leaf
//! `Poseidon(key_com, amount)` sits in the distro tree, that
//! `nullifier = Poseidon(secret, ctx)`, and it exposes the canonical body
//! fields as public inputs.

use std::string::{String, ToString};
use std::vec::Vec;

use ff::{Field, PrimeField};
use halo2_gadgets::{
    poseidon::{
        Hash as PoseidonHash, Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig,
        primitives::{ConstantLength, P128Pow5T3},
    },
    utilities::cond_swap::{CondSwapChip, CondSwapInstructions},
};
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, SimpleFloorPlanner, Value},
    plonk::{self, Advice, Circuit, Column, ConstraintSystem, Instance, SingleVerifier},
    poly::commitment::Params,
    transcript::{Blake2bRead, Blake2bWrite, Challenge255},
};
use pasta_curves::{pallas, vesta};
use rand_core::RngCore;

use crate::distro_poseidon::{personalization_to_fp, poseidon_distro_crh};

pub const MEMBERSHIP_DEPTH: usize = 16;

const TAG_KEYCOM: &str = "terp-hs-keycom-v1";
const TAG_NULLIFIER: &str = "terp-hs-nf-v1";
const TAG_LEAF: &str = "terp-hs-claim-leaf-v1";

#[derive(Clone, Debug)]
pub struct MembershipConfig {
    advice: [Column<Advice>; 6],
    poseidon: PoseidonConfig<pallas::Base, 3, 2>,
    swap: CondSwapChip<pallas::Base>,
    instance: Column<Instance>,
}

#[derive(Clone, Debug)]
pub struct MembershipCircuit {
    pub secret: Value<pallas::Base>,
    pub ctx: Value<pallas::Base>,
    pub amount: Value<u64>,
    pub position: Value<u32>,
    pub path: Value<[pallas::Base; MEMBERSHIP_DEPTH]>,
    pub destination_lo: Value<pallas::Base>,
    pub destination_hi: Value<pallas::Base>,
}

impl Default for MembershipCircuit {
    fn default() -> Self {
        Self {
            secret: Value::unknown(),
            ctx: Value::unknown(),
            amount: Value::unknown(),
            position: Value::unknown(),
            path: Value::unknown(),
            destination_lo: Value::unknown(),
            destination_hi: Value::unknown(),
        }
    }
}

impl Circuit<pallas::Base> for MembershipCircuit {
    type Config = MembershipConfig;
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
            advice[0..3].try_into().unwrap(),
            advice[3],
            rc_a,
            rc_b,
        );
        let swap_config = CondSwapChip::configure(meta, advice[0..5].try_into().unwrap());
        MembershipConfig {
            advice,
            poseidon,
            swap: CondSwapChip::construct(swap_config),
            instance,
        }
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), plonk::Error> {
        let secret = load(&mut layouter, config.advice[0], "secret", self.secret)?;
        let ctx = load(&mut layouter, config.advice[1], "ctx", self.ctx)?;
        let amount_fp = self.amount.map(pallas::Base::from);
        let amount = load(&mut layouter, config.advice[2], "amount", amount_fp)?;
        let dest_lo = load(
            &mut layouter,
            config.advice[3],
            "dest_lo",
            self.destination_lo,
        )?;
        let dest_hi = load(
            &mut layouter,
            config.advice[4],
            "dest_hi",
            self.destination_hi,
        )?;

        let key_com = hash2(
            &mut layouter,
            &config,
            "key_com",
            personalization_to_fp(TAG_KEYCOM),
            secret.clone(),
        )?;
        let nullifier = hash3(
            &mut layouter,
            &config,
            "nullifier",
            personalization_to_fp(TAG_NULLIFIER),
            secret,
            ctx.clone(),
        )?;
        let leaf = hash3(
            &mut layouter,
            &config,
            "leaf",
            personalization_to_fp(TAG_LEAF),
            key_com,
            amount.clone(),
        )?;
        let root = merkle_root(&mut layouter, &config, leaf, self.position, self.path)?;

        let public = [ctx, root, nullifier, dest_lo, dest_hi, amount];
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
    layouter.assign_region(
        || label,
        |mut region| region.assign_advice(|| label, column, 0, || value),
    )
}

fn hash2(
    layouter: &mut impl Layouter<pallas::Base>,
    config: &MembershipConfig,
    label: &'static str,
    tag: pallas::Base,
    left: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    let tag = load(layouter, config.advice[5], label, Value::known(tag))?;
    let chip = PoseidonChip::construct(config.poseidon.clone());
    let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<2>, 3, 2>::init(
        chip,
        layouter.namespace(|| label),
    )?;
    hasher.hash(layouter.namespace(|| label), [tag, left])
}

fn hash3(
    layouter: &mut impl Layouter<pallas::Base>,
    config: &MembershipConfig,
    label: &'static str,
    tag: pallas::Base,
    left: AssignedCell<pallas::Base, pallas::Base>,
    right: AssignedCell<pallas::Base, pallas::Base>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    let tag = load(layouter, config.advice[5], label, Value::known(tag))?;
    let chip = PoseidonChip::construct(config.poseidon.clone());
    let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<3>, 3, 2>::init(
        chip,
        layouter.namespace(|| label),
    )?;
    hasher.hash(layouter.namespace(|| label), [tag, left, right])
}

fn merkle_root(
    layouter: &mut impl Layouter<pallas::Base>,
    config: &MembershipConfig,
    leaf: AssignedCell<pallas::Base, pallas::Base>,
    position: Value<u32>,
    path: Value<[pallas::Base; MEMBERSHIP_DEPTH]>,
) -> Result<AssignedCell<pallas::Base, pallas::Base>, plonk::Error> {
    let mut node = leaf;
    for layer in 0..MEMBERSHIP_DEPTH {
        let sibling = path.map(|words| words[layer]);
        let swap_bit = position.map(|pos| (pos >> layer) & 1 == 1);
        let (left, right) =
            config
                .swap
                .swap(layouter.namespace(|| "order"), (node, sibling), swap_bit)?;
        let tag = load(
            layouter,
            config.advice[5],
            "crh-tag",
            Value::known(crate::distro_poseidon::distro_crh_tag()),
        )?;
        let layer_cell = load(
            layouter,
            config.advice[4],
            "layer",
            Value::known(pallas::Base::from(layer as u64)),
        )?;
        let chip = PoseidonChip::construct(config.poseidon.clone());
        let hasher = PoseidonHash::<_, _, P128Pow5T3, ConstantLength<4>, 3, 2>::init(
            chip,
            layouter.namespace(|| "crh"),
        )?;
        node = hasher.hash(layouter.namespace(|| "crh"), [tag, layer_cell, left, right])?;
    }
    Ok(node)
}

pub fn key_commitment(secret: pallas::Base) -> pallas::Base {
    poseidon_hash2(personalization_to_fp(TAG_KEYCOM), secret)
}

pub fn claim_nullifier(secret: pallas::Base, ctx: pallas::Base) -> pallas::Base {
    poseidon_hash3(personalization_to_fp(TAG_NULLIFIER), secret, ctx)
}

pub fn membership_leaf(key_com: pallas::Base, amount: u64) -> pallas::Base {
    poseidon_hash3(
        personalization_to_fp(TAG_LEAF),
        key_com,
        pallas::Base::from(amount),
    )
}

fn poseidon_hash2(tag: pallas::Base, left: pallas::Base) -> pallas::Base {
    halo2_gadgets::poseidon::primitives::Hash::<_, P128Pow5T3, ConstantLength<2>, 3, 2>::init()
        .hash([tag, left])
}

fn poseidon_hash3(tag: pallas::Base, left: pallas::Base, right: pallas::Base) -> pallas::Base {
    halo2_gadgets::poseidon::primitives::Hash::<_, P128Pow5T3, ConstantLength<3>, 3, 2>::init()
        .hash([tag, left, right])
}

pub fn auth_root(
    leaf: pallas::Base,
    position: u32,
    path: &[pallas::Base; MEMBERSHIP_DEPTH],
) -> pallas::Base {
    let mut node = leaf;
    for (layer, sibling) in path.iter().enumerate() {
        if position & (1 << layer) == 0 {
            node = poseidon_distro_crh(layer as u32, node, *sibling);
        } else {
            node = poseidon_distro_crh(layer as u32, *sibling, node);
        }
    }
    node
}

pub fn pack_destination(destination: &str) -> Result<(pallas::Base, pallas::Base), String> {
    let bytes = destination.as_bytes();
    if bytes.len() > 62 {
        return Err("destination longer than 62 bytes".to_string());
    }
    let mut lo = [0u8; 32];
    let mut hi = [0u8; 32];
    let split = core::cmp::min(31, bytes.len());
    lo[..split].copy_from_slice(&bytes[..split]);
    if bytes.len() > 31 {
        hi[..bytes.len() - 31].copy_from_slice(&bytes[31..]);
    }
    let lo = Option::from(pallas::Base::from_repr(lo)).ok_or("dest lo")?;
    let hi = Option::from(pallas::Base::from_repr(hi)).ok_or("dest hi")?;
    Ok((lo, hi))
}

pub fn canonical_body(
    ctx: pallas::Base,
    root: pallas::Base,
    nullifier: pallas::Base,
    destination: &str,
    amount: u64,
) -> String {
    format!(
        "headstash-claim-v1:{}:{}:{}:{}:{}",
        hex::encode(ctx.to_repr()),
        hex::encode(root.to_repr()),
        hex::encode(nullifier.to_repr()),
        destination,
        amount
    )
}

#[derive(Clone, Debug)]
pub struct MembershipProof {
    pub k: u32,
    pub proof: Vec<u8>,
    pub public: Vec<pallas::Base>,
    pub body: String,
    pub prove_secs: f64,
    pub verify_secs: f64,
    pub pk_bytes: usize,
}

pub fn prove_membership(
    secret: pallas::Base,
    ctx: pallas::Base,
    amount: u64,
    position: u32,
    path: [pallas::Base; MEMBERSHIP_DEPTH],
    destination: &str,
) -> Result<MembershipProof, String> {
    let (dest_lo, dest_hi) = pack_destination(destination)?;
    let key_com = key_commitment(secret);
    let leaf = membership_leaf(key_com, amount);
    let root = auth_root(leaf, position, &path);
    let nullifier = claim_nullifier(secret, ctx);
    let public = vec![
        ctx,
        root,
        nullifier,
        dest_lo,
        dest_hi,
        pallas::Base::from(amount),
    ];
    let circuit = MembershipCircuit {
        secret: Value::known(secret),
        ctx: Value::known(ctx),
        amount: Value::known(amount),
        position: Value::known(position),
        path: Value::known(path),
        destination_lo: Value::known(dest_lo),
        destination_hi: Value::known(dest_hi),
    };
    let started = tick();
    let k = minimal_k(&circuit, &public)?;
    std::eprintln!("no-ecdsa search_k_secs {:.3}", since(started));
    let started = tick();
    let params = Params::<vesta::Affine>::new(k);
    std::eprintln!("no-ecdsa params_secs {:.3}", since(started));
    let started = tick();
    let vk = plonk::keygen_vk(&params, &circuit).map_err(|err| format!("vk {err:?}"))?;
    std::eprintln!("no-ecdsa vk_secs {:.3}", since(started));
    let started = tick();
    let pk = plonk::keygen_pk(&params, vk, &circuit).map_err(|err| format!("pk {err:?}"))?;
    std::eprintln!("no-ecdsa pk_secs {:.3}", since(started));
    let mut pk_buf = Vec::new();
    pk.write(&mut pk_buf).map_err(|err| err.to_string())?;
    std::eprintln!("no-ecdsa pk_bytes {}", pk_buf.len());
    let started = tick();
    let mut rng = crate::os_rng();
    let mut transcript = Blake2bWrite::<_, vesta::Affine, Challenge255<_>>::init(Vec::new());
    plonk::create_proof(
        &params,
        &pk,
        &[circuit],
        &[&[&public]],
        &mut rng,
        &mut transcript,
    )
    .map_err(|err| format!("prove {err:?}"))?;
    let proof = transcript.finalize();
    let prove_secs = since(started);
    std::eprintln!("no-ecdsa prove_secs {:.3}", prove_secs);
    let started = tick();
    verify_membership(&params, pk.get_vk(), &proof, &public)?;
    let verify_secs = since(started);
    std::eprintln!("no-ecdsa verify_secs {:.3}", verify_secs);
    Ok(MembershipProof {
        k,
        proof,
        public,
        body: canonical_body(ctx, root, nullifier, destination, amount),
        prove_secs,
        verify_secs,
        pk_bytes: pk_buf.len(),
    })
}

/// Footer public-input count: ctx, root, nullifier, dest_lo, dest_hi, amount.
pub const MEMBERSHIP_INSTANCES: u8 = 6;
/// Proposed registry id. The epoch circuit stays at id 1.
pub const MEMBERSHIP_VK_ID: u64 = 2;

fn checksum32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(cosmwasm_std::Checksum::generate(bytes).as_slice());
    out
}

/// Params, post-synthesis constraint system, verifying key, and the 80-byte footer.
/// Same layout as `pasta_circuit_blob` / `epoch_vk_hash` on the epoch branch.
pub fn membership_circuit_blob() -> Result<Vec<u8>, String> {
    let circuit = MembershipCircuit::default();
    let k = 11u32;
    let params = Params::<vesta::Affine>::new(k);
    let vk = plonk::keygen_vk(&params, &circuit).map_err(|err| format!("vk {err:?}"))?;
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
            k as u8,
            MEMBERSHIP_INSTANCES,
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

pub fn membership_vk_hash() -> Result<[u8; 32], String> {
    Ok(checksum32(&membership_circuit_blob()?))
}

/// Six 32-byte little-endian limbs the K=11 host reads: ctx, root,
/// nullifier, destination_lo, destination_hi, amount. The first three are
/// the `to_repr` bytes already stored in the claim message.
pub fn membership_public_inputs(
    ctx: &[u8; 32],
    root: &[u8; 32],
    nullifier: &[u8; 32],
    destination: &str,
    amount: u64,
) -> Result<Vec<u8>, String> {
    let ctx = canon_field(ctx)?;
    let root = canon_field(root)?;
    let nullifier = canon_field(nullifier)?;
    let (dest_lo, dest_hi) = pack_destination(destination)?;
    let amount = pallas::Base::from(amount);
    let mut out = Vec::with_capacity(32 * 6);
    for limb in [ctx, root, nullifier, dest_lo, dest_hi, amount] {
        out.extend_from_slice(limb.to_repr().as_ref());
    }
    Ok(out)
}

fn canon_field(bytes: &[u8; 32]) -> Result<pallas::Base, String> {
    let value = Option::from(pallas::Base::from_repr(*bytes)).ok_or("field")?;
    Ok(value)
}

/// Blob, proof, and the 6x32-byte public limbs the host verifier reads.
#[derive(Clone, Debug)]
pub struct MembershipHostProof {
    pub blob: Vec<u8>,
    pub proof: Vec<u8>,
    pub instances: Vec<u8>,
}

pub fn membership_host_proof() -> Result<MembershipHostProof, String> {
    let secret = pallas::Base::from(0x11u64);
    let ctx = pallas::Base::from(0x22u64);
    let path = [pallas::Base::ZERO; MEMBERSHIP_DEPTH];
    let destination = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
    let proved = prove_membership(secret, ctx, 15, 0, path, destination)?;
    let blob = membership_circuit_blob()?;
    let mut instances = Vec::with_capacity(32 * proved.public.len());
    for limb in &proved.public {
        instances.extend_from_slice(limb.to_repr().as_ref());
    }
    Ok(MembershipHostProof {
        blob,
        proof: proved.proof,
        instances,
    })
}

pub fn verify_membership(
    params: &Params<vesta::Affine>,
    vk: &plonk::VerifyingKey<vesta::Affine>,
    proof: &[u8],
    public: &[pallas::Base],
) -> Result<(), String> {
    let strategy = SingleVerifier::new(params);
    let mut transcript = Blake2bRead::<_, vesta::Affine, Challenge255<_>>::init(proof);
    plonk::verify_proof(params, vk, strategy, &[&[&public[..]]], &mut transcript)
        .map_err(|err| format!("verify {err:?}"))
}

/// `Instant::now` panics on wasm32. The browser wall clock is measured in JS.
struct Tick {
    #[cfg(not(target_arch = "wasm32"))]
    inner: std::time::Instant,
}

fn tick() -> Tick {
    Tick {
        #[cfg(not(target_arch = "wasm32"))]
        inner: std::time::Instant::now(),
    }
}

fn since(start: Tick) -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = start;
        0.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        start.inner.elapsed().as_secs_f64()
    }
}

pub fn minimal_k(circuit: &MembershipCircuit, public: &[pallas::Base]) -> Result<u32, String> {
    for k in 8..20 {
        match halo2_proofs::dev::MockProver::run(k, circuit, vec![public.to_vec()]) {
            Ok(prover) => match prover.verify() {
                Ok(()) => return Ok(k),
                Err(_) => continue,
            },
            Err(_) => continue,
        }
    }
    Err("no K in 8..20 satisfied the membership circuit".to_string())
}

// Browser witness entry points (used by browser_prove.rs and the claim page).
pub fn registration_message(ctx: pallas::Base, key_com: pallas::Base, amount: u64) -> String {
    format!(
        "headstash-register-v1:{}:{}:{}",
        hex::encode(ctx.to_repr()),
        hex::encode(key_com.to_repr()),
        amount
    )
}

fn parse_fp(hex_str: &str) -> Result<pallas::Base, String> {
    if hex_str.len() != 64
        || !hex_str
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("field".into());
    }
    let bytes = hex::decode(hex_str).map_err(|_| "field".to_string())?;
    let mut repr = [0u8; 32];
    repr.copy_from_slice(&bytes);
    Option::from(pallas::Base::from_repr(repr)).ok_or_else(|| "field".to_string())
}

fn parse_amount(value: &serde_json::Value) -> Result<u64, String> {
    let text = match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        _ => return Err("amount".into()),
    };
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("amount".into());
    }
    text.parse::<u64>().map_err(|_| "amount".to_string())
}

/// Prove a caller-supplied witness. The supplied root must be the Poseidon root of this leaf.
pub fn prove_witness_json(witness_json: &str) -> Result<MembershipProof, String> {
    let value: serde_json::Value =
        serde_json::from_str(witness_json).map_err(|_| "witness".to_string())?;
    let secret = parse_fp(
        value
            .get("secret_hex")
            .and_then(|item| item.as_str())
            .ok_or("secret")?,
    )?;
    let ctx = parse_fp(
        value
            .get("ctx_hex")
            .and_then(|item| item.as_str())
            .ok_or("ctx")?,
    )?;
    let root = parse_fp(
        value
            .get("root_hex")
            .and_then(|item| item.as_str())
            .ok_or("root")?,
    )?;
    let destination = value
        .get("destination")
        .and_then(|item| item.as_str())
        .ok_or("destination")?;
    let amount = parse_amount(value.get("amount").ok_or("amount")?)?;
    let index = value
        .get("index")
        .and_then(|item| item.as_u64())
        .ok_or("index")?;
    if index >= 1u64 << MEMBERSHIP_DEPTH {
        return Err("index".into());
    }
    let siblings = value
        .get("siblings_hex")
        .and_then(|item| item.as_array())
        .ok_or("siblings")?;
    if siblings.len() != MEMBERSHIP_DEPTH {
        return Err("siblings".into());
    }
    let mut path = [pallas::Base::ZERO; MEMBERSHIP_DEPTH];
    for (layer, sibling) in siblings.iter().enumerate() {
        path[layer] = parse_fp(sibling.as_str().ok_or("sibling")?)?;
    }
    let computed = auth_root(
        membership_leaf(key_commitment(secret), amount),
        index as u32,
        &path,
    );
    if computed != root {
        return Err("root".into());
    }
    let proof = prove_membership(secret, ctx, amount, index as u32, path, destination)?;
    if proof.k != 11 || proof.public.first() != Some(&ctx) || proof.public.get(1) != Some(&root) {
        return Err("public".into());
    }
    Ok(proof)
}

/// Verify proof bytes for a witness under the K=11 membership key (vk id 2).
pub fn verify_witness_proof(witness_json: &str, proof: &[u8]) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(witness_json).map_err(|_| "witness".to_string())?;
    let secret = parse_fp(
        value
            .get("secret_hex")
            .and_then(|item| item.as_str())
            .ok_or("secret")?,
    )?;
    let ctx = parse_fp(
        value
            .get("ctx_hex")
            .and_then(|item| item.as_str())
            .ok_or("ctx")?,
    )?;
    let root = parse_fp(
        value
            .get("root_hex")
            .and_then(|item| item.as_str())
            .ok_or("root")?,
    )?;
    let destination = value
        .get("destination")
        .and_then(|item| item.as_str())
        .ok_or("destination")?;
    let amount = parse_amount(value.get("amount").ok_or("amount")?)?;
    let (dest_lo, dest_hi) = pack_destination(destination)?;
    let nullifier = claim_nullifier(secret, ctx);
    let public = vec![
        ctx,
        root,
        nullifier,
        dest_lo,
        dest_hi,
        pallas::Base::from(amount),
    ];
    let circuit = MembershipCircuit::default();
    let params = Params::<vesta::Affine>::new(11);
    let vk = plonk::keygen_vk(&params, &circuit).map_err(|err| format!("vk {err:?}"))?;
    verify_membership(&params, &vk, proof, &public)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        pallas::Base,
        pallas::Base,
        u64,
        u32,
        [pallas::Base; MEMBERSHIP_DEPTH],
        &'static str,
    ) {
        let secret = pallas::Base::from(0x11u64);
        let ctx = pallas::Base::from(0x22u64);
        let amount = 15u64;
        let position = 0u32;
        let path = [pallas::Base::ZERO; MEMBERSHIP_DEPTH];
        let destination = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
        (secret, ctx, amount, position, path, destination)
    }

    #[test]
    fn membership_proof_verifies_at_min_k() {
        let (secret, ctx, amount, position, path, destination) = fixture();
        let started = tick();
        let proof =
            prove_membership(secret, ctx, amount, position, path, destination).expect("prove");
        let secs = since(started);
        let key_com = key_commitment(secret);
        assert_eq!(proof.public[2], claim_nullifier(secret, ctx));
        assert!(proof.body.starts_with("headstash-claim-v1:"));
        assert!(proof.body.contains(destination));
        std::eprintln!(
            "no-ecdsa k {} proof_bytes {} key_com_secs_wall {:.3} key_com {}",
            proof.k,
            proof.proof.len(),
            secs,
            hex::encode(key_com.to_repr())
        );
    }

    #[test]
    fn membership_public_inputs_follow_the_claim_message() {
        let ctx = pallas::Base::from(3u64);
        let root = pallas::Base::from(4u64);
        let nullifier = pallas::Base::from(5u64);
        let destination = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
        let amount = 15u64;
        let (dest_lo, dest_hi) = pack_destination(destination).unwrap();
        let mut ctx_b = [0u8; 32];
        let mut root_b = [0u8; 32];
        let mut nullifier_b = [0u8; 32];
        ctx_b.copy_from_slice(ctx.to_repr().as_ref());
        root_b.copy_from_slice(root.to_repr().as_ref());
        nullifier_b.copy_from_slice(nullifier.to_repr().as_ref());
        let inputs =
            membership_public_inputs(&ctx_b, &root_b, &nullifier_b, destination, amount).unwrap();
        assert_eq!(inputs.len(), 192);
        assert_eq!(&inputs[0..32], ctx_b.as_slice());
        assert_eq!(&inputs[32..64], root_b.as_slice());
        assert_eq!(&inputs[64..96], nullifier_b.as_slice());
        assert_eq!(&inputs[96..128], dest_lo.to_repr().as_slice());
        assert_eq!(&inputs[128..160], dest_hi.to_repr().as_slice());
        assert_eq!(
            &inputs[160..192],
            pallas::Base::from(amount).to_repr().as_slice()
        );
        let body = canonical_body(ctx, root, nullifier, destination, amount);
        assert!(body.contains(&hex::encode(ctx_b)));
    }

    fn membership_vk_hash_is_the_footer_blob() {
        let blob = membership_circuit_blob().expect("blob");
        let hash = checksum32(&blob);
        assert!(blob.len() > 80);
        let footer = &blob[blob.len() - 80..];
        assert_eq!(footer[0], 0);
        assert_eq!(footer[2], 11);
        assert_eq!(footer[3], MEMBERSHIP_INSTANCES);
        std::eprintln!("membership-vk-hash {}", hex::encode(hash));
        std::eprintln!("membership-vk-bytes {}", blob.len());
        assert_eq!(
            hex::encode(hash),
            "94d4326108e28451829b68d64df58cf975fafdb874a5a554f3cbe8931eefed11",
        );
        assert_eq!(blob.len(), 135_023);
    }

    fn sample_witness_json() -> String {
        let secret = pallas::Base::from(0x31u64);
        let ctx = pallas::Base::from(0x44u64);
        let amount = 21u64;
        let index = 5u32;
        let mut path = [pallas::Base::ZERO; MEMBERSHIP_DEPTH];
        path[0] = pallas::Base::from(7u64);
        path[3] = pallas::Base::from(9u64);
        let root = auth_root(
            membership_leaf(key_commitment(secret), amount),
            index,
            &path,
        );
        let destination = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
        let siblings: Vec<String> = path
            .iter()
            .map(|limb| hex::encode(limb.to_repr()))
            .collect();
        serde_json::json!({
            "secret_hex": hex::encode(secret.to_repr()),
            "ctx_hex": hex::encode(ctx.to_repr()),
            "root_hex": hex::encode(root.to_repr()),
            "amount": amount.to_string(),
            "index": index,
            "siblings_hex": siblings,
            "destination": destination,
        })
        .to_string()
    }

    #[test]
    fn witness_proof_verifies_at_k11() {
        let witness = sample_witness_json();
        let proof = prove_witness_json(&witness).expect("prove");
        assert_eq!(proof.k, 11);
        assert_eq!(proof.proof.len(), 2752);
        assert_eq!(MEMBERSHIP_VK_ID, 2);
        verify_witness_proof(&witness, &proof.proof).expect("verify");
        if std::env::var("HS_WRITE_WITNESS").is_ok() {
            std::fs::write("/tmp/hs-witness.json", &witness).expect("write");
        }
    }

    #[test]
    #[ignore = "HS_WITNESS and HS_BROWSER_PROOF from the loopback worker"]
    fn browser_witness_proof_matches_k11_vk() {
        let witness =
            std::fs::read_to_string(std::env::var("HS_WITNESS").expect("witness")).expect("read");
        let proof_hex = std::fs::read_to_string(std::env::var("HS_BROWSER_PROOF").expect("proof"))
            .expect("read");
        let proof = hex::decode(proof_hex.trim()).expect("hex");
        assert_eq!(proof.len(), 2752);
        verify_witness_proof(witness.trim(), &proof).expect("verify");
    }

    #[test]
    fn registration_message_is_little_endian_hex() {
        let secret = pallas::Base::from(0x11u64);
        let ctx = pallas::Base::from(0x22u64);
        let key_com = key_commitment(secret);
        let message = registration_message(ctx, key_com, 15);
        let ctx_hex = hex::encode(ctx.to_repr());
        let key_hex = hex::encode(key_com.to_repr());
        assert_eq!(ctx_hex.len(), 64);
        assert_eq!(key_hex.len(), 64);
        assert_eq!(
            message,
            format!("headstash-register-v1:{ctx_hex}:{key_hex}:15")
        );
        assert_eq!(
            key_hex,
            "30a1926427275f67355ac7c48031acbc2266cac18cd86447c563b54e70113a08"
        );
        let body = canonical_body(
            ctx,
            key_com,
            key_com,
            "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw",
            15,
        );
        assert!(body.starts_with("headstash-claim-v1:"));
        let public = vec![
            ctx,
            key_com,
            key_com,
            key_com,
            key_com,
            pallas::Base::from(15u64),
        ];
        assert_eq!(public.len(), 6);
    }
}
