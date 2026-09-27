//! Consume plane: Halo2 Pasta (`prover_id=0`) + Poseidon merkle/nullifier.
//!
//! Statement (CIRCUITS-PREFLIGHT): guest proves membership; usher submits.
//! Public: `guest_root`, door `nullifier`, `event_id`. Witness: ticketSecret, sku, path.
//! `leaf = Poseidon(ticketSecret, sku, event_id)`
//! `nullifier = Poseidon(ticketSecret, event_id)`
//! Parent = Poseidon(left, right). Not Sinsemilla / Pedersen.
//!
//! On-chain verify is `deps.api.verify_halo2_proof` after `MsgStoreCodeWithCircuit`.
//! Local tests prove/verify here. MockBech32 is not a success path.

use crate::itl_ingress::{ict_mock_refused, is_mock_bech32};
use crate::poseidon::{
    door_nullifier, ticket_leaf, MerklePath, PoseidonTree, TREE_DEPTH, Fp as HostFp,
};
use ff::{Field, PrimeField};
use halo2_proofs::{
    circuit::{AssignedCell, Layouter, SimpleFloorPlanner, Value},
    plonk::{
        self, Advice, Circuit, Column, ConstraintSystem, Error, Expression, Fixed, Instance,
        Selector, SingleVerifier,
    },
    poly::Rotation,
    transcript::{Blake2bRead, Blake2bWrite, Challenge255},
};
use pasta_curves::{pallas, vesta};
use rand::rngs::OsRng;
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::OnceLock;

/// Halo2 IPA / Pasta. zk-cosmwasm `verify_halo2_proof` `prover_id=0`.
pub const PROVER_ID: u8 = 0;
pub const K: u32 = 11;
pub const INSTANCE_COUNT: usize = 3;
pub const MSG_STORE_CODE_WITH_CIRCUIT: &str = "/cosmwasm.wasm.v1.MsgStoreCodeWithCircuit";
pub const CONSUME_LOCAL_PROD_CMD: &str =
    "cargo run --example itl_consume_e2e -p ict-rs-cw-orch --features docker";

const T: usize = 3;
const RF: usize = 8;
const RP: usize = 56;
const ROUNDS: usize = RF + RP;

type Base = pallas::Base;

pub fn consume_local_prod_cmd() -> &'static str {
    CONSUME_LOCAL_PROD_CMD
}

pub fn fp_from_bytes(bytes: &[u8]) -> Base {
    let mut b = [0u8; 32];
    let n = bytes.len().min(32);
    b[..n].copy_from_slice(&bytes[..n]);
    b[31] &= 0x3f;
    Option::from(Base::from_repr(b)).expect("bytes < 2^254")
}

pub fn fp_from_str(s: &str) -> Base {
    fp_from_bytes(s.as_bytes())
}

pub fn fp_to_bytes(f: Base) -> [u8; 32] {
    f.to_repr()
}

pub fn host_fp_to_base(fp: HostFp) -> Base {
    fp_from_bytes(&fp.to_bytes())
}

fn pow5_expr(x: Expression<Base>) -> Expression<Base> {
    let sq = x.clone() * x.clone();
    sq.clone() * sq * x
}

fn round_constants() -> &'static [Base] {
    static RC: OnceLock<Vec<Base>> = OnceLock::new();
    RC.get_or_init(|| {
        let mut out = Vec::with_capacity(ROUNDS * T);
        let mut i = 0u32;
        while out.len() < ROUNDS * T {
            let mut h = Sha256::new();
            h.update(b"itl-poseidon-pallas-t3-v1");
            h.update(i.to_le_bytes());
            out.push(fp_from_bytes(&h.finalize()));
            i += 1;
        }
        out
    })
}

fn mds() -> &'static [[Base; T]; T] {
    static MDS: OnceLock<[[Base; T]; T]> = OnceLock::new();
    MDS.get_or_init(|| {
        let mut m = [[Base::ZERO; T]; T];
        for i in 0..T {
            for j in 0..T {
                m[i][j] = Base::from((i + j + 3) as u64)
                    .invert()
                    .expect("mds denom");
            }
        }
        m
    })
}

fn mix(state: &mut [Base; T]) {
    let m = mds();
    let mut out = [Base::ZERO; T];
    for i in 0..T {
        let mut acc = Base::ZERO;
        for j in 0..T {
            acc += m[i][j] * state[j];
        }
        out[i] = acc;
    }
    *state = out;
}

fn permute(state: &mut [Base; T]) {
    let rc = round_constants();
    let mut k = 0;
    let half = RF / 2;
    for _ in 0..half {
        for s in state.iter_mut() {
            *s += rc[k];
            k += 1;
        }
        for s in state.iter_mut() {
            *s = s.pow([5]);
        }
        mix(state);
    }
    for _ in 0..RP {
        for s in state.iter_mut() {
            *s += rc[k];
            k += 1;
        }
        state[0] = state[0].pow([5]);
        mix(state);
    }
    for _ in 0..half {
        for s in state.iter_mut() {
            *s += rc[k];
            k += 1;
        }
        for s in state.iter_mut() {
            *s = s.pow([5]);
        }
        mix(state);
    }
}

/// Same sponge as `poseidon.rs` (capacity arity, rate 2, squeeze state[1]).
pub fn poseidon_hash(inputs: &[Base]) -> Base {
    let mut state = [Base::ZERO, Base::ZERO, Base::ZERO];
    state[0] = Base::from(inputs.len() as u64);
    let mut i = 0;
    if inputs.is_empty() {
        permute(&mut state);
        return state[1];
    }
    while i < inputs.len() {
        let mut r = 1;
        while r < T && i < inputs.len() {
            state[r] += inputs[i];
            i += 1;
            r += 1;
        }
        permute(&mut state);
    }
    state[1]
}

pub fn pasta_poseidon2(a: Base, b: Base) -> Base {
    poseidon_hash(&[a, b])
}

pub fn pasta_poseidon3(a: Base, b: Base, c: Base) -> Base {
    poseidon_hash(&[a, b, c])
}

pub fn pasta_ticket_leaf(ticket_secret: &[u8], sku: &str, event_id: &str) -> Base {
    pasta_poseidon3(
        fp_from_bytes(ticket_secret),
        fp_from_str(sku),
        fp_from_str(event_id),
    )
}

pub fn pasta_door_nullifier(ticket_secret: &[u8], event_id: &str) -> Base {
    pasta_poseidon2(fp_from_bytes(ticket_secret), fp_from_str(event_id))
}

#[derive(Clone, Debug)]
pub struct ConsumeConfig {
    state: [Column<Advice>; 3],
    rc: [Column<Fixed>; 3],
    #[allow(dead_code)]
    const_col: Column<Fixed>,
    instance: Column<Instance>,
    s_full: Selector,
    s_partial: Selector,
    s_add: Selector,
    s_mux: Selector,
}

fn configure(meta: &mut ConstraintSystem<Base>) -> ConsumeConfig {
    let state = [
        meta.advice_column(),
        meta.advice_column(),
        meta.advice_column(),
    ];
    let rc = [meta.fixed_column(), meta.fixed_column(), meta.fixed_column()];
    let const_col = meta.fixed_column();
    meta.enable_constant(const_col);
    let instance = meta.instance_column();
    for c in &state {
        meta.enable_equality(*c);
    }
    meta.enable_equality(instance);

    let s_full = meta.selector();
    let s_partial = meta.selector();
    let s_add = meta.selector();
    let s_mux = meta.selector();
    let m = *mds();

    meta.create_gate("poseidon_full", |meta| {
        let s = meta.query_selector(s_full);
        let cur: Vec<Expression<Base>> = (0..T)
            .map(|i| {
                meta.query_advice(state[i], Rotation::cur())
                    + meta.query_fixed(rc[i])
            })
            .collect();
        let sbox: Vec<Expression<Base>> = cur.into_iter().map(pow5_expr).collect();
        (0..T)
            .map(|i| {
                let mut acc = Expression::Constant(Base::ZERO);
                for j in 0..T {
                    acc = acc + Expression::Constant(m[i][j]) * sbox[j].clone();
                }
                let next = meta.query_advice(state[i], Rotation::next());
                s.clone() * (acc - next)
            })
            .collect::<Vec<_>>()
    });

    meta.create_gate("poseidon_partial", |meta| {
        let s = meta.query_selector(s_partial);
        let after_rc: Vec<Expression<Base>> = (0..T)
            .map(|i| {
                meta.query_advice(state[i], Rotation::cur())
                    + meta.query_fixed(rc[i])
            })
            .collect();
        let mixed: Vec<Expression<Base>> = {
            let mut v = after_rc;
            v[0] = pow5_expr(v[0].clone());
            v
        };
        (0..T)
            .map(|i| {
                let mut acc = Expression::Constant(Base::ZERO);
                for j in 0..T {
                    acc = acc + Expression::Constant(m[i][j]) * mixed[j].clone();
                }
                let next = meta.query_advice(state[i], Rotation::next());
                s.clone() * (acc - next)
            })
            .collect::<Vec<_>>()
    });

    meta.create_gate("add", |meta| {
        let s = meta.query_selector(s_add);
        let a = meta.query_advice(state[0], Rotation::cur());
        let b = meta.query_advice(state[1], Rotation::cur());
        let o = meta.query_advice(state[2], Rotation::cur());
        vec![s * (a + b - o)]
    });

    meta.create_gate("mux_bool", |meta| {
        let s = meta.query_selector(s_mux);
        let cur = meta.query_advice(state[0], Rotation::cur());
        let sib = meta.query_advice(state[1], Rotation::cur());
        let bit = meta.query_advice(state[2], Rotation::cur());
        let left = meta.query_advice(state[0], Rotation::next());
        let right = meta.query_advice(state[1], Rotation::next());
        let one = Expression::Constant(Base::ONE);
        vec![
            s.clone()
                * (left
                    - ((one.clone() - bit.clone()) * cur.clone() + bit.clone() * sib.clone())),
            s.clone() * (right - (bit.clone() * cur + (one.clone() - bit.clone()) * sib)),
            s * bit.clone() * (bit - one),
        ]
    });

    ConsumeConfig {
        state,
        rc,
        const_col,
        instance,
        s_full,
        s_partial,
        s_add,
        s_mux,
    }
}

struct Chip {
    config: ConsumeConfig,
}

impl Chip {
    fn add(
        &self,
        mut layouter: impl Layouter<Base>,
        a: &AssignedCell<Base, Base>,
        b: &AssignedCell<Base, Base>,
    ) -> Result<AssignedCell<Base, Base>, Error> {
        layouter.assign_region(
            || "add",
            |mut region| {
                self.config.s_add.enable(&mut region, 0)?;
                a.copy_advice(|| "a", &mut region, self.config.state[0], 0)?;
                b.copy_advice(|| "b", &mut region, self.config.state[1], 0)?;
                let sum = a.value().copied() + b.value().copied();
                region.assign_advice(|| "sum", self.config.state[2], 0, || sum)
            },
        )
    }

    fn mux(
        &self,
        mut layouter: impl Layouter<Base>,
        cur: &AssignedCell<Base, Base>,
        sib: Value<Base>,
        bit: Value<Base>,
    ) -> Result<(AssignedCell<Base, Base>, AssignedCell<Base, Base>), Error> {
        layouter.assign_region(
            || "mux",
            |mut region| {
                self.config.s_mux.enable(&mut region, 0)?;
                cur.copy_advice(|| "cur", &mut region, self.config.state[0], 0)?;
                region.assign_advice(|| "sib", self.config.state[1], 0, || sib)?;
                region.assign_advice(|| "bit", self.config.state[2], 0, || bit)?;
                let left = cur.value().copied().zip(sib).zip(bit).map(|((c, s), b)| {
                    (Base::ONE - b) * c + b * s
                });
                let right = cur.value().copied().zip(sib).zip(bit).map(|((c, s), b)| {
                    b * c + (Base::ONE - b) * s
                });
                let l = region.assign_advice(|| "left", self.config.state[0], 1, || left)?;
                let r = region.assign_advice(|| "right", self.config.state[1], 1, || right)?;
                Ok((l, r))
            },
        )
    }

    fn permute(
        &self,
        mut layouter: impl Layouter<Base>,
        init: [AssignedCell<Base, Base>; 3],
    ) -> Result<[AssignedCell<Base, Base>; 3], Error> {
        let rc = round_constants();
        layouter.assign_region(
            || "permute",
            |mut region| {
                let mut cells = [(); 3].map(|_| None);
                for i in 0..T {
                    cells[i] = Some(init[i].copy_advice(
                        || "init",
                        &mut region,
                        self.config.state[i],
                        0,
                    )?);
                }
                let mut state = [
                    init[0].value().copied(),
                    init[1].value().copied(),
                    init[2].value().copied(),
                ];
                let mut k = 0usize;
                let half = RF / 2;
                let mut row = 0;
                let apply_round = |region: &mut halo2_proofs::circuit::Region<Base>,
                                       row: usize,
                                       full: bool,
                                       state: &mut [Value<Base>; 3],
                                       k: &mut usize|
                 -> Result<(), Error> {
                    if full {
                        self.config.s_full.enable(region, row)?;
                    } else {
                        self.config.s_partial.enable(region, row)?;
                    }
                    for i in 0..T {
                        region.assign_fixed(|| "rc", self.config.rc[i], row, || Value::known(rc[*k + i]))?;
                    }
                    let r0 = rc[*k];
                    let r1 = rc[*k + 1];
                    let r2 = rc[*k + 2];
                    *k += T;
                    let after = [
                        state[0] + Value::known(r0),
                        state[1] + Value::known(r1),
                        state[2] + Value::known(r2),
                    ];
                    let sboxed = if full {
                        [
                            after[0].map(|x| x.pow([5])),
                            after[1].map(|x| x.pow([5])),
                            after[2].map(|x| x.pow([5])),
                        ]
                    } else {
                        [
                            after[0].map(|x| x.pow([5])),
                            after[1],
                            after[2],
                        ]
                    };
                    let mm = mds();
                    let mut nxt = [Value::known(Base::ZERO); 3];
                    for i in 0..T {
                        let mut acc = Value::known(Base::ZERO);
                        for j in 0..T {
                            acc = acc + sboxed[j] * Value::known(mm[i][j]);
                        }
                        nxt[i] = acc;
                    }
                    *state = nxt;
                    Ok(())
                };

                for _ in 0..half {
                    apply_round(&mut region, row, true, &mut state, &mut k)?;
                    row += 1;
                    for i in 0..T {
                        cells[i] = Some(region.assign_advice(
                            || "st",
                            self.config.state[i],
                            row,
                            || state[i],
                        )?);
                    }
                }
                for _ in 0..RP {
                    apply_round(&mut region, row, false, &mut state, &mut k)?;
                    row += 1;
                    for i in 0..T {
                        cells[i] = Some(region.assign_advice(
                            || "st",
                            self.config.state[i],
                            row,
                            || state[i],
                        )?);
                    }
                }
                for _ in 0..half {
                    apply_round(&mut region, row, true, &mut state, &mut k)?;
                    row += 1;
                    for i in 0..T {
                        cells[i] = Some(region.assign_advice(
                            || "st",
                            self.config.state[i],
                            row,
                            || state[i],
                        )?);
                    }
                }
                Ok([
                    cells[0].clone().unwrap(),
                    cells[1].clone().unwrap(),
                    cells[2].clone().unwrap(),
                ])
            },
        )
    }

    fn load(
        &self,
        mut layouter: impl Layouter<Base>,
        v: Value<Base>,
    ) -> Result<AssignedCell<Base, Base>, Error> {
        layouter.assign_region(
            || "load",
            |mut region| region.assign_advice(|| "w", self.config.state[0], 0, || v),
        )
    }

    fn load_const(
        &self,
        mut layouter: impl Layouter<Base>,
        c: Base,
    ) -> Result<AssignedCell<Base, Base>, Error> {
        layouter.assign_region(
            || "const",
            |mut region| {
                let cell =
                    region.assign_advice(|| "c", self.config.state[0], 0, || Value::known(c))?;
                region.constrain_constant(cell.cell(), c)?;
                Ok(cell)
            },
        )
    }

    fn hash(
        &self,
        mut layouter: impl Layouter<Base>,
        inputs: &[AssignedCell<Base, Base>],
    ) -> Result<AssignedCell<Base, Base>, Error> {
        let len = self.load_const(layouter.namespace(|| "len"), Base::from(inputs.len() as u64))?;
        let zero = self.load_const(layouter.namespace(|| "z"), Base::ZERO)?;
        let mut state = [len, zero.clone(), zero];
        let mut i = 0;
        if inputs.is_empty() {
            state = self.permute(layouter.namespace(|| "p0"), state)?;
            return Ok(state[1].clone());
        }
        let mut p = 0u32;
        while i < inputs.len() {
            let mut r = 1;
            while r < T && i < inputs.len() {
                state[r] = self.add(
                    layouter.namespace(|| format!("abs{i}")),
                    &state[r],
                    &inputs[i],
                )?;
                i += 1;
                r += 1;
            }
            state = self.permute(layouter.namespace(|| format!("p{p}")), state)?;
            p += 1;
        }
        Ok(state[1].clone())
    }
}

/// Witness for one consume proof. Layout is fixed (depth = [`TREE_DEPTH`]).
#[derive(Clone)]
pub struct ConsumeCircuit {
    pub ticket_secret: Value<Base>,
    pub sku: Value<Base>,
    pub event_id: Value<Base>,
    pub index_bits: [Value<Base>; TREE_DEPTH],
    pub siblings: [Value<Base>; TREE_DEPTH],
}

impl ConsumeCircuit {
    pub fn empty() -> Self {
        Self {
            ticket_secret: Value::unknown(),
            sku: Value::unknown(),
            event_id: Value::unknown(),
            index_bits: [Value::unknown(); TREE_DEPTH],
            siblings: [Value::unknown(); TREE_DEPTH],
        }
    }

    pub fn from_witness(
        ticket_secret: &[u8],
        sku: &str,
        event_id: &str,
        path: &MerklePath,
    ) -> Result<Self, &'static str> {
        if path.siblings.len() != TREE_DEPTH {
            return Err("bad_path_depth");
        }
        let mut index_bits = [Value::unknown(); TREE_DEPTH];
        let mut siblings = [Value::unknown(); TREE_DEPTH];
        for d in 0..TREE_DEPTH {
            let bit = (path.index >> d) & 1;
            index_bits[d] = Value::known(Base::from(bit));
            siblings[d] = Value::known(host_fp_to_base(path.siblings[d]));
        }
        Ok(Self {
            ticket_secret: Value::known(fp_from_bytes(ticket_secret)),
            sku: Value::known(fp_from_str(sku)),
            event_id: Value::known(fp_from_str(event_id)),
            index_bits,
            siblings,
        })
    }
}

impl Default for ConsumeCircuit {
    fn default() -> Self {
        Self::empty()
    }
}

impl Circuit<Base> for ConsumeCircuit {
    type Config = ConsumeConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::empty()
    }

    fn configure(meta: &mut ConstraintSystem<Base>) -> Self::Config {
        configure(meta)
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<Base>,
    ) -> Result<(), Error> {
        let chip = Chip { config: config.clone() };
        let secret = chip.load(layouter.namespace(|| "secret"), self.ticket_secret)?;
        let sku = chip.load(layouter.namespace(|| "sku"), self.sku)?;
        let event_id = chip.load(layouter.namespace(|| "eid"), self.event_id)?;

        let leaf = chip.hash(layouter.namespace(|| "leaf"), &[secret.clone(), sku, event_id.clone()])?;
        let nullifier = chip.hash(layouter.namespace(|| "nf"), &[secret, event_id.clone()])?;

        let mut cur = leaf;
        for d in 0..TREE_DEPTH {
            let (left, right) = chip.mux(
                layouter.namespace(|| format!("mux{d}")),
                &cur,
                self.siblings[d],
                self.index_bits[d],
            )?;
            cur = chip.hash(layouter.namespace(|| format!("up{d}")), &[left, right])?;
        }

        layouter.constrain_instance(cur.cell(), config.instance, 0)?;
        layouter.constrain_instance(nullifier.cell(), config.instance, 1)?;
        layouter.constrain_instance(event_id.cell(), config.instance, 2)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ConsumeInstances {
    pub guest_root: Base,
    pub nullifier: Base,
    pub event_id: Base,
}

impl ConsumeInstances {
    pub fn from_ticket(
        guest_root: [u8; 32],
        ticket_secret: &[u8],
        event_id: &str,
    ) -> Self {
        Self {
            guest_root: fp_from_bytes(&guest_root),
            nullifier: pasta_door_nullifier(ticket_secret, event_id),
            event_id: fp_from_str(event_id),
        }
    }

    pub fn as_array(&self) -> [Base; INSTANCE_COUNT] {
        [self.guest_root, self.nullifier, self.event_id]
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(96);
        for f in self.as_array() {
            out.extend_from_slice(&fp_to_bytes(f));
        }
        out
    }

    pub fn to_binaries(&self) -> Vec<Vec<u8>> {
        self.as_array()
            .into_iter()
            .map(|f| fp_to_bytes(f).to_vec())
            .collect()
    }
}

fn to_halo2_instance(pubs: &ConsumeInstances) -> [[vesta::Scalar; INSTANCE_COUNT]; 1] {
    let a = pubs.as_array();
    [[a[0], a[1], a[2]]]
}

fn setup_keys() -> Result<
    (
        halo2_proofs::poly::commitment::Params<vesta::Affine>,
        plonk::ProvingKey<vesta::Affine>,
    ),
    String,
> {
    let params = halo2_proofs::poly::commitment::Params::new(K);
    let empty = ConsumeCircuit::empty();
    let vk = plonk::keygen_vk(&params, &empty).map_err(|e| format!("keygen_vk: {e}"))?;
    let pk = plonk::keygen_pk(&params, vk, &empty).map_err(|e| format!("keygen_pk: {e}"))?;
    Ok((params, pk))
}

fn keys() -> Result<
    &'static (
        halo2_proofs::poly::commitment::Params<vesta::Affine>,
        plonk::ProvingKey<vesta::Affine>,
    ),
    String,
> {
    static KYS: OnceLock<
        Result<
            (
                halo2_proofs::poly::commitment::Params<vesta::Affine>,
                plonk::ProvingKey<vesta::Affine>,
            ),
            String,
        >,
    > = OnceLock::new();
    match KYS.get_or_init(setup_keys) {
        Ok(k) => Ok(k),
        Err(e) => Err(e.clone()),
    }
}

pub fn prove_consume(circuit: ConsumeCircuit, inst: &ConsumeInstances) -> Result<Vec<u8>, String> {
    let (params, pk) = keys()?;
    let instance = to_halo2_instance(inst);
    let refs: Vec<&[vesta::Scalar]> = instance.iter().map(|r| &r[..]).collect();
    let mut transcript = Blake2bWrite::<_, vesta::Affine, Challenge255<_>>::init(vec![]);
    plonk::create_proof(params, pk, &[circuit], &[&refs], OsRng, &mut transcript)
        .map_err(|e| format!("create_proof: {e}"))?;
    Ok(transcript.finalize())
}

/// Local Pasta Halo2 verify — same statement zk-cosmwasm `verify_halo2_proof` checks.
pub fn verify_halo2_proof(proof: &[u8], inst: &ConsumeInstances) -> Result<bool, String> {
    let (params, pk) = keys()?;
    let instance = to_halo2_instance(inst);
    let refs: Vec<&[vesta::Scalar]> = instance.iter().map(|r| &r[..]).collect();
    let strategy = SingleVerifier::new(params);
    let mut transcript = Blake2bRead::<_, vesta::Affine, Challenge255<_>>::init(proof);
    match plonk::verify_proof(params, pk.get_vk(), strategy, &[&refs], &mut transcript) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// VK blob: `Params::write` || ITL1 commitments || 10-byte footer.
/// crates.io halo2 0.3 has no `VerifyingKey::write`. We do **not** store Debug
/// `pinned()`. zk-wasmvm `check_vk` still needs that fork's `VK::read`; until
/// then this blob is host-roundtrippable (params) and machine-readable.
pub const VK_MAGIC: &[u8; 4] = b"ITL1";

pub fn serialize_vk_with_footer() -> Result<Vec<u8>, String> {
    let (params, pk) = keys()?;
    let mut params_buf = Vec::new();
    params
        .write(&mut params_buf)
        .map_err(|e| format!("params write: {e}"))?;
    let _ = pk;
    // halo2 0.3 VerifyingKey fields and pinned() contents are private — no VK::write.
    // Store a machine-readable handle (not Debug). zk-wasmvm check_vk still needs that fork.
    let mut vk_buf = Vec::from(*VK_MAGIC);
    vk_buf.extend_from_slice(&K.to_le_bytes());
    vk_buf.extend_from_slice(&(INSTANCE_COUNT as u32).to_le_bytes());
    let mut footer = [0u8; 10];
    footer[0] = PROVER_ID;
    footer[1] = INSTANCE_COUNT as u8;
    footer[2..6].copy_from_slice(&(params_buf.len() as u32).to_le_bytes());
    footer[6..10].copy_from_slice(&(vk_buf.len() as u32).to_le_bytes());
    let mut out = params_buf;
    out.extend_from_slice(&vk_buf);
    out.extend_from_slice(&footer);
    Ok(out)
}

pub fn parse_vk_footer(blob: &[u8]) -> Result<(u8, u8, u32, u32), &'static str> {
    if blob.len() < 10 {
        return Err("short_vk");
    }
    let f = &blob[blob.len() - 10..];
    Ok((
        f[0],
        f[1],
        u32::from_le_bytes(f[2..6].try_into().unwrap()),
        u32::from_le_bytes(f[6..10].try_into().unwrap()),
    ))
}

fn b64(data: &[u8]) -> String {
    const CH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CH[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CH[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CH[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CH[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Store cw-ave wasm + Halo2 VK together. Standard `MsgStoreCode` is not enough.
pub fn msg_store_code_with_circuit(sender: &str, wasm: &[u8], vk: &[u8]) -> Json {
    json!({
        "@type": MSG_STORE_CODE_WITH_CIRCUIT,
        "sender": sender,
        "wasm_byte_code": b64(wasm),
        "verifying_key": b64(vk),
        "prover_id": PROVER_ID,
        "instantiate_permission": null,
    })
}

pub fn consume_backend_mocked(backend: &str, ict_mock: Option<&str>) -> bool {
    is_mock_bech32(backend)
        || backend.contains("MockBech32")
        || backend.eq_ignore_ascii_case("mock")
        || ict_mock_refused(ict_mock)
}

/// Gate: recorded success must be Docker Terp / Daemon, not MockBech32.
pub fn consume_success_allowed(backend: &str, ict_mock: Option<&str>) -> bool {
    !consume_backend_mocked(backend, ict_mock)
}

pub fn verify_consume_msg(proof: &[u8], nullifier: &[u8; 32], event_id_fp: &[u8; 32]) -> Json {
    json!({
        "verify_consume": {
            "proof": b64(proof),
            "nullifier": crate::poseidon::hex_encode(nullifier),
            "event_id": crate::poseidon::hex_encode(event_id_fp),
        }
    })
}

/// In-process door. Ushers public; guests are not a ListMembers roster.
pub struct ConsumeEngine {
    ushers: HashSet<String>,
    nullifiers: HashSet<[u8; 32]>,
    guest_root: Option<[u8; 32]>,
}

impl Default for ConsumeEngine {
    fn default() -> Self {
        Self {
            ushers: HashSet::new(),
            nullifiers: HashSet::new(),
            guest_root: None,
        }
    }
}

impl ConsumeEngine {
    pub fn add_usher(&mut self, addr: &str) {
        self.ushers.insert(addr.to_string());
    }

    pub fn set_guest_root(&mut self, root: [u8; 32]) {
        self.guest_root = Some(root);
    }

    pub fn guest_root(&self) -> Option<[u8; 32]> {
        self.guest_root
    }

    pub fn is_usher(&self, addr: &str) -> bool {
        self.ushers.contains(addr)
    }

    pub fn nullifier_spent(&self, nf: &[u8; 32]) -> bool {
        self.nullifiers.contains(nf)
    }

    /// Guest roster query: absent. Do not return ticket addrs.
    pub fn guest_list_members(&self) -> Option<Vec<String>> {
        None
    }

    pub fn verify_consume(
        &mut self,
        usher: &str,
        proof: &[u8],
        nullifier: [u8; 32],
        event_id_fp: [u8; 32],
    ) -> Result<(), &'static str> {
        if is_mock_bech32(usher) {
            return Err("mock_bech32");
        }
        if !self.ushers.contains(usher) {
            return Err("not_usher");
        }
        let root = self.guest_root.ok_or("guest_root_unset")?;
        if self.nullifiers.contains(&nullifier) {
            return Err("nullifier_spent");
        }
        let inst = ConsumeInstances {
            guest_root: fp_from_bytes(&root),
            nullifier: fp_from_bytes(&nullifier),
            event_id: fp_from_bytes(&event_id_fp),
        };
        match verify_halo2_proof(proof, &inst) {
            Ok(true) => {}
            _ => return Err("invalid_proof"),
        }
        self.nullifiers.insert(nullifier);
        Ok(())
    }
}

pub fn prove_issued(
    tree: &PoseidonTree,
    ticket_secret: &[u8],
    sku: &str,
    event_id: &str,
    leaf_index: u64,
) -> Result<(Vec<u8>, ConsumeInstances, [u8; 32]), String> {
    let path = tree.prove(leaf_index).map_err(|e| e.to_string())?;
    let leaf = ticket_leaf(ticket_secret, sku, event_id);
    if !path.verify(HostFp::from_bytes(&leaf), tree.root()) {
        return Err("path_mismatch".into());
    }
    let circuit = ConsumeCircuit::from_witness(ticket_secret, sku, event_id, &path)
        .map_err(|e| e.to_string())?;
    let inst = ConsumeInstances::from_ticket(tree.root_bytes(), ticket_secret, event_id);
    let proof = prove_consume(circuit, &inst)?;
    Ok((proof, inst, door_nullifier(ticket_secret, event_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poseidon::{poseidon2, poseidon3, Fp};
    use halo2_proofs::dev::MockProver;

    #[test]
    fn pasta_poseidon_matches_host_not_sinsemilla() {
        let a = Fp::from_u64(1);
        let b = Fp::from_u64(2);
        let c = Fp::from_u64(3);
        let h2 = poseidon2(a, b);
        let p2 = pasta_poseidon2(host_fp_to_base(a), host_fp_to_base(b));
        assert_eq!(h2.to_bytes(), fp_to_bytes(p2));
        let h3 = poseidon3(a, b, c);
        let p3 = pasta_poseidon3(host_fp_to_base(a), host_fp_to_base(b), host_fp_to_base(c));
        assert_eq!(h3.to_bytes(), fp_to_bytes(p3));
        let secret = [7u8; 32];
        assert_eq!(
            ticket_leaf(&secret, "sku", "evt"),
            fp_to_bytes(pasta_ticket_leaf(&secret, "sku", "evt"))
        );
        assert_eq!(
            door_nullifier(&secret, "evt"),
            fp_to_bytes(pasta_door_nullifier(&secret, "evt"))
        );
    }

    #[test]
    fn mock_prover_accepts_paid_leaf_rejects_unpaid() {
        let mut tree = PoseidonTree::default();
        let secret = [9u8; 32];
        let sku = "itl-speakeasy-oct18";
        let event_id = "evt/speakeasy-2026-10-18";
        let leaf = ticket_leaf(&secret, sku, event_id);
        let idx = tree.append_bytes(leaf).unwrap();
        let path = tree.prove(idx).unwrap();
        let circuit = ConsumeCircuit::from_witness(&secret, sku, event_id, &path).unwrap();
        let inst = ConsumeInstances::from_ticket(tree.root_bytes(), &secret, event_id);
        let pubs = vec![inst.as_array().to_vec()];
        MockProver::run(K, &circuit, pubs.clone())
            .expect("mock prover")
            .assert_satisfied();

        let unpaid = [3u8; 32];
        let unpaid_leaf = ticket_leaf(&unpaid, sku, event_id);
        // Reuse the paid path — unpaid leaf is not in the tree.
        let bad = ConsumeCircuit::from_witness(&unpaid, sku, event_id, &path).unwrap();
        let bad_inst = ConsumeInstances::from_ticket(tree.root_bytes(), &unpaid, event_id);
        // Even against the unpaid's own nullifier, root will not match this path.
        let _ = unpaid_leaf;
        let pubs_bad = vec![bad_inst.as_array().to_vec()];
        let prover = MockProver::run(K, &bad, pubs_bad).expect("mock prover runs");
        assert!(prover.verify().is_err(), "unpaid leaf must not satisfy");
    }

    #[test]
    fn vk_footer_is_halo2_pasta_prover_id_0() {
        let blob = serialize_vk_with_footer().expect("vk");
        let (pid, n, pl, vl) = parse_vk_footer(&blob).unwrap();
        assert_eq!(pid, PROVER_ID);
        assert_eq!(n, INSTANCE_COUNT as u8);
        assert!(pl > 0 && vl > 4);
        let vk = &blob[pl as usize..blob.len() - 10];
        assert_eq!(&vk[..4], VK_MAGIC);
        let s = String::from_utf8_lossy(vk);
        assert!(
            !s.contains("PinnedVerificationKey"),
            "VK must not be Debug pinned()"
        );
        let mut params_slice = &blob[..pl as usize];
        halo2_proofs::poly::commitment::Params::<vesta::Affine>::read(&mut params_slice)
            .expect("Params::read roundtrip");
    }

    #[test]
    fn store_code_with_circuit_not_plain_store_code() {
        let msg = msg_store_code_with_circuit("terp1sender", b"wasm", b"vk");
        assert_eq!(msg["@type"], MSG_STORE_CODE_WITH_CIRCUIT);
        assert_eq!(msg["prover_id"], 0);
        assert!(msg.get("verifying_key").is_some());
        assert!(!msg["@type"].as_str().unwrap().ends_with("MsgStoreCode")
            || msg["@type"].as_str().unwrap().contains("Circuit"));
        assert_ne!(msg["@type"], "/cosmwasm.wasm.v1.MsgStoreCode");
    }

    #[test]
    fn mockbech32_is_not_success() {
        assert!(consume_backend_mocked("MockBech32", None));
        assert!(consume_backend_mocked("daemon", Some("1")));
        assert!(!consume_success_allowed("MockBech32", None));
        assert!(consume_success_allowed("daemon", None));
        assert!(!consume_success_allowed("ict-rs-docker", Some("true")));
    }
}
