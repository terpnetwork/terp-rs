use ff::PrimeField;
use group::ff::Field;
use halo2_proofs::{
    circuit::{AssignedCell, Chip, Layouter, Region, SimpleFloorPlanner, Value},
    plonk::{self, Advice, Circuit, Column, ConstraintSystem, Error, Fixed, Instance, Selector, SingleVerifier},
    poly::Rotation,
    transcript::{Blake2bRead, Blake2bWrite, Challenge255},
};
use pasta_curves::{pallas, vesta};
use rand::rngs::OsRng;
use std::marker::PhantomData;
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------------
// Field chip (multiplication gate)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct FieldConfig {
    advice: [Column<Advice>; 2],
    instance: Column<Instance>,
    s_mul: Selector,
}

#[derive(Debug)]
struct FieldChip<F: Field> {
    config: FieldConfig,
    _marker: PhantomData<F>,
}

#[derive(Clone)]
struct Number<F: PrimeField>(AssignedCell<F, F>);

impl<F: Field> Chip<F> for FieldChip<F> {
    type Config = FieldConfig;
    type Loaded = ();
    fn config(&self) -> &Self::Config { &self.config }
    fn loaded(&self) -> &Self::Loaded { &() }
}

impl<F: PrimeField> FieldChip<F> {
    fn construct(config: FieldConfig) -> Self {
        Self { config, _marker: PhantomData }
    }

    fn configure(
        meta: &mut ConstraintSystem<F>,
        advice: [Column<Advice>; 2],
        instance: Column<Instance>,
        constant: Column<Fixed>,
    ) -> FieldConfig {
        meta.enable_equality(instance);
        meta.enable_constant(constant);
        for column in &advice {
            meta.enable_equality(*column);
        }
        let s_mul = meta.selector();
        meta.create_gate("mul", |meta| {
            let lhs = meta.query_advice(advice[0], Rotation::cur());
            let rhs = meta.query_advice(advice[1], Rotation::cur());
            let out = meta.query_advice(advice[0], Rotation::next());
            let s_mul = meta.query_selector(s_mul);
            vec![s_mul * (lhs * rhs - out)]
        });
        FieldConfig { advice, instance, s_mul }
    }

    fn load_private(&self, mut layouter: impl Layouter<F>, value: Value<F>) -> Result<Number<F>, Error> {
        layouter.assign_region(
            || "load private",
            |mut region| {
                region.assign_advice(|| "private input", self.config.advice[0], 0, || value).map(Number)
            },
        )
    }

    fn load_constant(&self, mut layouter: impl Layouter<F>, constant: F) -> Result<Number<F>, Error> {
        layouter.assign_region(
            || "load constant",
            |mut region| {
                region.assign_advice_from_constant(|| "constant value", self.config.advice[0], 0, constant).map(Number)
            },
        )
    }

    fn load_public(&self, mut layouter: impl Layouter<F>, row: usize) -> Result<Number<F>, Error> {
        layouter.assign_region(
            || "load public",
            |mut region| {
                region.assign_advice_from_instance(|| "public input", self.config.instance, row, self.config.advice[0], 0).map(Number)
            },
        )
    }

    fn mul(&self, mut layouter: impl Layouter<F>, a: Number<F>, b: Number<F>) -> Result<Number<F>, Error> {
        layouter.assign_region(
            || "mul",
            |mut region: Region<'_, F>| {
                self.config.s_mul.enable(&mut region, 0)?;
                a.0.copy_advice(|| "lhs", &mut region, self.config.advice[0], 0)?;
                b.0.copy_advice(|| "rhs", &mut region, self.config.advice[1], 0)?;
                let value = a.0.value().copied() * b.0.value();
                region.assign_advice(|| "lhs * rhs", self.config.advice[0], 1, || value).map(Number)
            },
        )
    }

    fn constrain_equal(&self, mut layouter: impl Layouter<F>, a: &AssignedCell<F, F>, b: &AssignedCell<F, F>) -> Result<(), Error> {
        layouter.assign_region(
            || "constrain equal",
            |mut region| region.constrain_equal(a.cell(), b.cell()),
        )
    }
}

// ---------------------------------------------------------------------------
// NoRickCircuit — proves a 20-byte witness does NOT contain "rick"
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct NoRickCircuit<F: PrimeField> {
    priv_input: Vec<Value<F>>,
}

impl<F: PrimeField> Default for NoRickCircuit<F> {
    fn default() -> Self {
        Self { priv_input: vec![Value::unknown(); 20] }
    }
}

impl<F: PrimeField> Circuit<F> for NoRickCircuit<F> {
    type Config = FieldConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self { Self::default() }

    fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
        let advice = [meta.advice_column(), meta.advice_column()];
        let instance = meta.instance_column();
        let constant = meta.fixed_column();
        FieldChip::configure(meta, advice, instance, constant)
    }

    fn synthesize(&self, config: Self::Config, mut layouter: impl Layouter<F>) -> Result<(), Error> {
        let chip = FieldChip::<F>::construct(config);

        let pow_256_1 = F::from(256u64);
        let pow_256_2 = F::from(65536u64);
        let pow_256_3 = F::from(16777216u64);

        // Load private input characters
        let mut chars = Vec::new();
        for (i, &char_val) in self.priv_input.iter().enumerate() {
            let c = chip.load_private(layouter.namespace(|| format!("char {}", i)), char_val)?;
            chars.push(c);
        }

        // Load forbidden word from instance row 0
        let rick_constant = chip.load_public(layouter.namespace(|| "load forbidden"), 0)?;

        // Check all 17 positions where a 4-byte word could start
        let mut conditions = Vec::new();
        for idx in 0..17 {
            let c0 = chars[idx].clone();
            let c1 = chars[idx + 1].clone();
            let c2 = chars[idx + 2].clone();
            let c3 = chars[idx + 3].clone();

            let packed_val = c0.0.value().copied()
                + c1.0.value().copied() * Value::known(pow_256_1)
                + c2.0.value().copied() * Value::known(pow_256_2)
                + c3.0.value().copied() * Value::known(pow_256_3);

            let diff_val = packed_val - rick_constant.0.value().copied();
            let diff = chip.load_private(layouter.namespace(|| format!("diff {}", idx)), diff_val)?;
            conditions.push(diff);
        }

        // Product of all differences (zero iff any position matched)
        let mut product = conditions[0].clone();
        for i in 1..conditions.len() {
            product = chip.mul(
                layouter.namespace(|| format!("mul {}", i)),
                product,
                conditions[i].clone(),
            )?;
        }

        // product * inverse = 1 (fails if product is zero)
        let inv_p_val = product.0.value().map(|p| p.invert().unwrap_or(F::ZERO));
        let inv_p = chip.load_private(layouter.namespace(|| "inv_p"), inv_p_val)?;
        let product_times_inv = chip.mul(layouter.namespace(|| "p*inv"), product, inv_p)?;

        let one = chip.load_constant(layouter.namespace(|| "one"), F::ONE)?;
        chip.constrain_equal(layouter.namespace(|| "== 1"), &product_times_inv.0, &one.0)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn str_to_field<F: PrimeField>(s: &str) -> F {
    let mut repr = F::default().to_repr();
    let src = s.as_bytes();
    let len = core::cmp::min(src.len(), repr.as_ref().len());
    repr.as_mut()[..len].copy_from_slice(&src[..len]);
    F::from_repr(repr).expect("str_to_field: invalid representation")
}

fn to_halo2_instance(forbidden: &str) -> [[vesta::Scalar; 1]; 1] {
    let mut instance = [vesta::Scalar::zero(); 1];
    instance[0] = str_to_field(forbidden);
    [instance]
}

fn to_cosmwasm_instance(forbidden: &str) -> Vec<u8> {
    let instances = to_halo2_instance(forbidden);
    let mut bytes = Vec::with_capacity(32);
    for row in instances.iter() {
        for scalar in row.iter() {
            bytes.extend_from_slice(scalar.to_repr().as_ref());
        }
    }
    bytes
}

// ---------------------------------------------------------------------------
// Host prove / verify (also used by WASM exports)
// ---------------------------------------------------------------------------

const K: u32 = 10;

fn setup_keys() -> Result<
    (
        halo2_proofs::poly::commitment::Params<vesta::Affine>,
        plonk::ProvingKey<vesta::Affine>,
    ),
    String,
> {
    let params = halo2_proofs::poly::commitment::Params::new(K);
    let empty_circuit: NoRickCircuit<pallas::Base> = Default::default();
    let vk = plonk::keygen_vk(&params, &empty_circuit).map_err(|e| format!("keygen_vk: {e}"))?;
    let pk = plonk::keygen_pk(&params, vk, &empty_circuit).map_err(|e| format!("keygen_pk: {e}"))?;
    Ok((params, pk))
}

fn circuit_from_secret(secret_word: &str) -> NoRickCircuit<pallas::Base> {
    let mut bytes = secret_word.as_bytes().to_vec();
    bytes.resize(20, 0);
    let priv_input: Vec<Value<pallas::Base>> = bytes
        .iter()
        .map(|&b| Value::known(pallas::Base::from(b as u64)))
        .collect();
    NoRickCircuit { priv_input }
}

/// Host-equivalent of the WASM guest: prove `secret_word` does not contain `forbidden`.
pub fn prove_no_rick(secret_word: &str, forbidden: &str) -> Result<Vec<u8>, String> {
    let circuit = circuit_from_secret(secret_word);
    let (params, pk) = setup_keys()?;
    let instance = to_halo2_instance(forbidden);
    let instance_refs: Vec<&[vesta::Scalar]> = instance.iter().map(|row| &row[..]).collect();

    let mut transcript = Blake2bWrite::<_, vesta::Affine, Challenge255<_>>::init(vec![]);
    plonk::create_proof(
        &params,
        &pk,
        &[circuit],
        &[&instance_refs],
        OsRng,
        &mut transcript,
    )
    .map_err(|e| format!("create_proof: {e}"))?;
    Ok(transcript.finalize())
}

/// Host-equivalent verify for a proof produced by [`prove_no_rick`] / WASM `generate_proof`.
pub fn verify_no_rick(proof: &[u8], forbidden: &str) -> Result<bool, String> {
    let (params, pk) = setup_keys()?;
    let instance = to_halo2_instance(forbidden);
    let instance_refs: Vec<&[vesta::Scalar]> = instance.iter().map(|row| &row[..]).collect();
    let strategy = SingleVerifier::new(&params);
    let mut transcript = Blake2bRead::<_, vesta::Affine, Challenge255<_>>::init(proof);
    match plonk::verify_proof(
        &params,
        pk.get_vk(),
        strategy,
        &[&instance_refs],
        &mut transcript,
    ) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// WASM exports
// ---------------------------------------------------------------------------

/// Encode circuit instance bytes for the forbidden word.
#[wasm_bindgen]
pub fn encode_instances(forbidden: &str) -> Vec<u8> {
    to_cosmwasm_instance(forbidden)
}

/// Generate a halo2 proof that `secret_word` does not contain `forbidden`.
///
/// Returns base64-encoded proof bytes.
#[wasm_bindgen]
pub fn generate_proof(secret_word: &str, forbidden: &str) -> Result<String, JsValue> {
    let proof_bytes = prove_no_rick(secret_word, forbidden).map_err(|e| JsValue::from_str(&e))?;
    Ok(b64_encode(&proof_bytes))
}

/// Build the JSON ExecuteMsg::Proove for the no-rick contract.
#[wasm_bindgen]
pub fn encode_proof_msg(cid: u64, forbidden: &str, proof_b64: &str) -> String {
    serde_json::json!({
        "proove": {
            "cid": cid,
            "forbidden": forbidden,
            "proof": proof_b64
        }
    })
    .to_string()
}

/// Convert a string to its field element hex (debugging).
#[wasm_bindgen]
pub fn str_to_field_hex(s: &str) -> String {
    let f: vesta::Scalar = str_to_field(s);
    hex_encode(f.to_repr().as_ref())
}

// ---------------------------------------------------------------------------
// Minimal base64 + hex encoders (avoid extra crate deps)
// ---------------------------------------------------------------------------

const B64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(B64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(B64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(B64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(B64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg(test)]
mod host_e2e {
    use super::*;

    #[test]
    fn prove_verify_accepts_clean_secret() {
        let proof = prove_no_rick("hello world........", "rick").expect("prove");
        assert!(!proof.is_empty());
        assert!(verify_no_rick(&proof, "rick").expect("verify"));
    }

    #[test]
    fn encode_workflow_matches_guest_export() {
        let instances = encode_instances("rick");
        assert_eq!(instances.len(), 32);
        let proof_b64 = generate_proof("safe phrase........", "rick").expect("guest wrapper");
        assert!(!proof_b64.is_empty());
        let msg = encode_proof_msg(1, "rick", &proof_b64);
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["proove"]["forbidden"], "rick");
        assert_eq!(v["proove"]["proof"], proof_b64);
    }

    #[test]
    fn verify_rejects_tampered_proof() {
        let mut proof = prove_no_rick("clean................", "rick").expect("prove");
        if let Some(b) = proof.last_mut() {
            *b ^= 0xff;
        }
        assert!(!verify_no_rick(&proof, "rick").expect("verify tampered"));
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}
