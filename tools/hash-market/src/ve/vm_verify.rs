//! Embedded CosmWasm VM verifier for halo2 claims.
//!
//! The host method is `do_proof_instance_verify` (`Api::proof_instance_verify`).
//! Keys are deserialized in `register`. A claim only selects one by `vk_id`.
//!
//! Wasmer's `Store` is not `Send`. `Halo2Host` is `Send + Sync`, so the store
//! stays on a worker thread. Callers send a check and wait for the outcome.

use std::collections::{BTreeMap, HashMap};
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{encode, Bech32, Hrp};
use cosmwasm_std::{
    from_json, to_json_binary, Addr, Binary, CircuitInfoResponse, ContractResult,
    Empty, Order, QueryRequest, Record, SystemError, SystemResult, WasmQuery,
};
use cosmwasm_vm::{
    compile_module, do_proof_instance_verify, write_region, AnyVerifyingKey, BackendApi,
    BackendError, BackendResult, CircuitLoader, Environment, GasInfo, Querier, Size, Storage,
};
use sha2::{Digest, Sha256};
use wasmer::{imports, FunctionEnv, FunctionEnvMut, Instance, Module, Store, Value};

use headstash_epoch::{
    epoch_circuit_blob, epoch_vk_hash, install_halo2_host, Halo2Host, HostVerifyError, EPOCH_VK_ID,
};

/// Refill before each claim. An epoch proof is about 5.7e9 gas.
const GAS_REFILL: u64 = 50_000_000_000;

/// wasmd `DefaultGasMultiplier`. Humanize is 4x, canonicalize is 5x.
const WASMD_GAS_MULTIPLIER: u64 = 140_000;
const GAS_COST_HUMANIZE: u64 = 4 * WASMD_GAS_MULTIPLIER;
const GAS_COST_CANONICALIZE: u64 = 5 * WASMD_GAS_MULTIPLIER;
const GAS_COST_STORAGE: u64 = WASMD_GAS_MULTIPLIER;
const BECH32_PREFIX: &str = "terp";

/// Address conversion only. `do_proof_instance_verify` calls `cosmwasm-crypto`
/// itself; this type does not accept or reject a proof.
#[derive(Clone, Copy, Debug)]
struct HostApi;

struct IterCursor {
    records: Vec<Record>,
    pos: usize,
}

struct MemoryStorage {
    data: BTreeMap<Vec<u8>, Vec<u8>>,
    iterators: BTreeMap<u32, IterCursor>,
    next_id: u32,
}

/// Serves `WasmQuery::CircuitInfo` from the registry. Every other query,
/// including `WasmQuery::Circuit`, is refused so a cache miss cannot cold-load
/// a blob. `do_proof_instance_verify` always asks for CircuitInfo first.
struct HostQuerier {
    registry: Arc<Mutex<Registry>>,
}

const ALLOC_WAT: &str = r#"(module
  (memory (export "memory") 64)
  (global $bump (mut i32) (i32.const 65536))
  (func (export "allocate") (param $size i32) (result i32)
    (local $region i32)
    (local $data i32)
    (local $aligned i32)
    (local.set $region (global.get $bump))
    (local.set $data (i32.add (local.get $region) (i32.const 16)))
    (local.set $aligned
      (i32.and
        (i32.add (i32.add (local.get $data) (local.get $size)) (i32.const 15))
        (i32.const -16)))
    (if (i32.gt_u (local.get $aligned) (i32.const 4194304))
      (then
        (local.set $region (i32.const 65536))
        (local.set $data (i32.add (local.get $region) (i32.const 16)))
        (local.set $aligned
          (i32.and
            (i32.add (i32.add (local.get $data) (local.get $size)) (i32.const 15))
            (i32.const -16)))))
    (global.set $bump (local.get $aligned))
    (i32.store (local.get $region) (local.get $data))
    (i32.store offset=4 (local.get $region) (local.get $size))
    (i32.store offset=8 (local.get $region) (i32.const 0))
    (local.get $region)))
"#;

type HostEnv = Environment<HostApi, MemoryStorage, HostQuerier>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostOutcome {
    Valid,
    Rejected,
    Unavailable,
}

struct KeySlot {
    circuit_key: [u8; 72],
    blob_hash: [u8; 32],
    vk: AnyVerifyingKey,
}

struct Registry {
    by_id: HashMap<u64, KeySlot>,
    by_key: HashMap<[u8; 72], AnyVerifyingKey>,
}

struct VmState {
    store: Store,
    env: FunctionEnv<HostEnv>,
    instance: Box<Instance>,
    _module: Module,
}

enum Job {
    Check {
        vk_id: u64,
        proof: Vec<u8>,
        instances: Vec<u8>,
        reply: Sender<HostOutcome>,
    },
}

/// One worker thread owns the VM. The registry is shared so a claim never
/// builds a key.
pub struct VmVerifier {
    tx: Mutex<Sender<Job>>,
    registry: Arc<Mutex<Registry>>,
}

impl VmVerifier {
    pub fn open() -> Result<Self, String> {
        let registry = Arc::new(Mutex::new(Registry {
            by_id: HashMap::new(),
            by_key: HashMap::new(),
        }));
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let registry_thread = Arc::clone(&registry);
        thread::Builder::new()
            .name("halo2-host".into())
            .spawn(move || worker(registry_thread, rx, ready_tx))
            .map_err(|_| "vm")?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err("vm".into()),
        }
        Ok(Self {
            tx: Mutex::new(tx),
            registry,
        })
    }

    /// Deserialize `blob` once and cache it under `vk_id`.
    pub fn register(&self, vk_id: u64, blob: &[u8]) -> Result<[u8; 32], String> {
        if vk_id == 0 || vk_id > u32::MAX as u64 {
            return Err("vk id".into());
        }
        let footer = cosmwasm_vm::check_circuit(blob).map_err(|_| "circuit")?;
        let vk = AnyVerifyingKey::try_from(blob).map_err(|_| "circuit")?;
        if vk.circuit_key() != footer.to_circuit_key() {
            return Err("circuit".into());
        }
        let blob_hash = sha256(blob);
        let mut guard = self.registry.lock().map_err(|_| "registry")?;
        if guard.by_id.contains_key(&vk_id) {
            return Err("vk id".into());
        }
        guard.by_key.insert(vk.circuit_key(), vk.clone());
        guard.by_id.insert(
            vk_id,
            KeySlot {
                circuit_key: vk.circuit_key(),
                blob_hash,
                vk,
            },
        );
        Ok(blob_hash)
    }

    pub fn hash_of(&self, vk_id: u64) -> Option<[u8; 32]> {
        self.registry
            .lock()
            .ok()
            .and_then(|guard| guard.by_id.get(&vk_id).map(|slot| slot.blob_hash))
    }

    /// Select the cached key for `vk_id` and ask the host. The registry
    /// lock is not held across the host call.
    pub fn check(&self, vk_id: u64, proof: &[u8], instances: &[u8]) -> HostOutcome {
        let (reply_tx, reply_rx) = mpsc::channel();
        let job = Job::Check {
            vk_id,
            proof: proof.to_vec(),
            instances: instances.to_vec(),
            reply: reply_tx,
        };
        let sent = match self.tx.lock() {
            Ok(tx) => tx.send(job).is_ok(),
            Err(_) => false,
        };
        if !sent {
            return HostOutcome::Unavailable;
        }
        reply_rx.recv().unwrap_or(HostOutcome::Unavailable)
    }
}

impl Halo2Host for VmVerifier {
    fn verify(&self, vk_id: u64, proof: &[u8], instances: &[u8]) -> Result<(), HostVerifyError> {
        match self.check(vk_id, proof, instances) {
            HostOutcome::Valid => Ok(()),
            HostOutcome::Rejected | HostOutcome::Unavailable => Err(HostVerifyError::Rejected),
        }
    }

    fn vk_hash(&self, vk_id: u64) -> Option<[u8; 32]> {
        self.hash_of(vk_id)
    }
}

/// Build the epoch blob, cache it under [`EPOCH_VK_ID`], and install the host.
pub fn install_epoch_host() -> Result<[u8; 32], String> {
    if let Some(hash) = headstash_epoch::halo2_vk_hash(EPOCH_VK_ID) {
        return Ok(hash);
    }
    let vm = VmVerifier::open()?;
    let hash = vm.register(EPOCH_VK_ID, epoch_circuit_blob())?;
    if hash != epoch_vk_hash() {
        return Err("vk hash".into());
    }
    install_halo2_host(Arc::new(vm));
    Ok(hash)
}

fn worker(registry: Arc<Mutex<Registry>>, rx: Receiver<Job>, ready: Sender<Result<(), String>>) {
    let mut vm = match VmState::open(registry) {
        Ok(vm) => vm,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    while let Ok(job) = rx.recv() {
        match job {
            Job::Check {
                vk_id,
                proof,
                instances,
                reply,
            } => {
                let _ = reply.send(vm.check(vk_id, &proof, &instances));
            }
        }
    }
}

impl VmState {
    fn open(registry: Arc<Mutex<Registry>>) -> Result<Self, String> {
        let wasm = wat::parse_str(ALLOC_WAT).map_err(|_| "wasm")?;
        let (module, engine) = compile_module(&wasm, Some(Size::mebi(16))).map_err(|_| "wasm")?;
        let mut store = Store::new(engine);
        let environment = Environment::new(HostApi, GAS_REFILL);
        let env = FunctionEnv::new(&mut store, environment);
        let imports = imports! {};
        let instance = Instance::new(&mut store, &module, &imports).map_err(|_| "wasm")?;
        let instance = Box::new(instance);
        let memory = instance
            .exports
            .get_memory("memory")
            .map_err(|_| "wasm")?
            .clone();
        let instance_ptr = NonNull::from(instance.as_ref());
        env.as_mut(&mut store).memory = Some(memory);

        let loader = circuit_loader(Arc::clone(&registry));
        let querier = HostQuerier { registry };
        {
            let mut fe = env.clone().into_mut(&mut store);
            let (data, mut store_mut) = fe.data_and_store_mut();
            data.set_wasmer_instance(Some(instance_ptr));
            data.set_gas_left(&mut store_mut, GAS_REFILL);
            data.set_storage_readonly(false);
            data.move_in(MemoryStorage::new(), querier);
            data.set_circuit_loader(Some(loader));
        }
        Ok(Self {
            store,
            env,
            instance,
            _module: module,
        })
    }

    fn check(&mut self, vk_id: u64, proof: &[u8], instances: &[u8]) -> HostOutcome {
        if vk_id == 0 || vk_id > u32::MAX as u64 || proof.len() > u32::MAX as usize {
            return HostOutcome::Unavailable;
        }
        let started = Instant::now();
        let mut fe = self.env.clone().into_mut(&mut self.store);
        if let Err(err) = refill(&mut fe) {
            std::eprintln!("epoch-claim host_err {err}");
            return HostOutcome::Unavailable;
        }
        let proof_ptr = match alloc_write(&mut fe, proof) {
            Ok(ptr) => ptr,
            Err(err) => {
                std::eprintln!("epoch-claim host_err {err}");
                return HostOutcome::Unavailable;
            }
        };
        let inst_ptr = match alloc_write(&mut fe, instances) {
            Ok(ptr) => ptr,
            Err(err) => {
                std::eprintln!("epoch-claim host_err {err}");
                return HostOutcome::Unavailable;
            }
        };
        if let Err(err) = refill(&mut fe) {
            std::eprintln!("epoch-claim host_err {err}");
            return HostOutcome::Unavailable;
        }
        let code = do_proof_instance_verify(
            fe.as_mut(),
            vk_id as u32,
            proof_ptr,
            proof.len() as u32,
            inst_ptr,
            instances.len() as u32,
        );
        let secs = started.elapsed().as_secs_f64();
        match code {
            Ok(0) => {
                std::eprintln!("epoch-claim host_verify vk_id {vk_id} secs {secs:.3} code 0");
                HostOutcome::Valid
            }
            Ok(1) => {
                std::eprintln!("epoch-claim host_verify vk_id {vk_id} secs {secs:.3} code 1");
                HostOutcome::Rejected
            }
            Ok(other) => {
                std::eprintln!("epoch-claim host_verify vk_id {vk_id} secs {secs:.3} code {other}");
                HostOutcome::Unavailable
            }
            Err(err) => {
                std::eprintln!("epoch-claim host_err {err}");
                HostOutcome::Unavailable
            }
        }
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let dig = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

fn circuit_loader(registry: Arc<Mutex<Registry>>) -> CircuitLoader {
    Arc::new(move |key| {
        // A poisoned registry must not cold-load a different blob. `Ok(None)`
        // sends the host down the Circuit query, which this sidecar rejects.
        let Ok(guard) = registry.lock() else {
            return Ok(None);
        };
        Ok(guard.by_key.get(&key).cloned())
    })
}

impl HostApi {
    fn new() -> Self {
        HostApi
    }
}

fn validate_canonical(bytes: &[u8]) -> Result<(), BackendError> {
    match bytes.len() {
        20 | 32 => Ok(()),
        _ => Err(BackendError::user_err("Invalid canonical address length")),
    }
}

impl BackendApi for HostApi {
    fn addr_validate(&self, input: &str) -> BackendResult<()> {
        let (canonical, gas_canon) = self.addr_canonicalize(input);
        let canonical = match canonical {
            Ok(canonical) => canonical,
            Err(err) => return (Err(err), gas_canon),
        };
        let (human, gas_human) = self.addr_humanize(&canonical);
        let mut gas = gas_canon;
        gas += gas_human;
        let human = match human {
            Ok(human) => human,
            Err(err) => return (Err(err), gas),
        };
        if input != human.as_str() {
            return (
                Err(BackendError::user_err(
                    "Invalid input: address not normalized",
                )),
                gas,
            );
        }
        (Ok(()), gas)
    }

    fn addr_canonicalize(&self, input: &str) -> BackendResult<Vec<u8>> {
        let gas = GasInfo::with_cost(GAS_COST_CANONICALIZE);
        let parsed = match CheckedHrpstring::new::<Bech32>(input) {
            Ok(parsed) => parsed,
            Err(_) => return (Err(BackendError::user_err("Error decoding bech32")), gas),
        };
        if !parsed
            .hrp()
            .as_bytes()
            .eq_ignore_ascii_case(BECH32_PREFIX.as_bytes())
        {
            return (Err(BackendError::user_err("Wrong bech32 prefix")), gas);
        }
        let bytes: Vec<u8> = parsed.byte_iter().collect();
        if let Err(err) = validate_canonical(&bytes) {
            return (Err(err), gas);
        }
        (Ok(bytes), gas)
    }

    fn addr_humanize(&self, canonical: &[u8]) -> BackendResult<String> {
        let gas = GasInfo::with_cost(GAS_COST_HUMANIZE);
        if let Err(err) = validate_canonical(canonical) {
            return (Err(err), gas);
        }
        let hrp = match Hrp::parse(BECH32_PREFIX) {
            Ok(hrp) => hrp,
            Err(_) => return (Err(BackendError::user_err("Invalid bech32 prefix")), gas),
        };
        match encode::<Bech32>(hrp, canonical) {
            Ok(encoded) => (Ok(encoded), gas),
            Err(_) => (
                Err(BackendError::user_err(
                    "Invalid data to be encoded to bech32",
                )),
                gas,
            ),
        }
    }
}

impl MemoryStorage {
    fn new() -> Self {
        Self {
            data: BTreeMap::new(),
            iterators: BTreeMap::new(),
            next_id: 0,
        }
    }
}

impl Storage for MemoryStorage {
    fn get(&self, key: &[u8]) -> BackendResult<Option<Vec<u8>>> {
        (
            Ok(self.data.get(key).cloned()),
            GasInfo::with_cost(GAS_COST_STORAGE),
        )
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> BackendResult<()> {
        self.data.insert(key.to_vec(), value.to_vec());
        (Ok(()), GasInfo::with_cost(GAS_COST_STORAGE))
    }

    fn remove(&mut self, key: &[u8]) -> BackendResult<()> {
        self.data.remove(key);
        (Ok(()), GasInfo::with_cost(GAS_COST_STORAGE))
    }

    fn scan(
        &mut self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        order: Order,
    ) -> BackendResult<u32> {
        let gas = GasInfo::with_cost(GAS_COST_STORAGE);
        let mut keys: Vec<Vec<u8>> = self
            .data
            .keys()
            .filter(|key| {
                if let Some(start) = start {
                    if key.as_slice() < start {
                        return false;
                    }
                }
                if let Some(end) = end {
                    if key.as_slice() >= end {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        if order == Order::Descending {
            keys.reverse();
        }
        let records = keys
            .into_iter()
            .map(|key| {
                let value = self.data.get(&key).cloned().unwrap_or_default();
                (key, value)
            })
            .collect();
        let id = self.next_id.checked_add(1).expect("iterator id");
        self.next_id = id;
        self.iterators.insert(
            id,
            IterCursor {
                records,
                pos: 0,
            },
        );
        (Ok(id), gas)
    }

    fn next(&mut self, iterator_id: u32) -> BackendResult<Option<Record>> {
        let gas = GasInfo::with_cost(GAS_COST_STORAGE);
        let Some(cursor) = self.iterators.get_mut(&iterator_id) else {
            return (
                Err(BackendError::IteratorDoesNotExist { id: iterator_id }),
                gas,
            );
        };
        if cursor.pos >= cursor.records.len() {
            return (Ok(None), gas);
        }
        let record = cursor.records[cursor.pos].clone();
        cursor.pos += 1;
        (Ok(Some(record)), gas)
    }
}

fn circuit_info(registry: &Mutex<Registry>, zk_id: u64) -> SystemResult<ContractResult<Binary>> {
    let guard = match registry.lock() {
        Ok(guard) => guard,
        Err(_) => return SystemResult::Err(SystemError::NoSuchCircuit { zk_id }),
    };
    match guard.by_id.get(&zk_id) {
        Some(slot) => {
            let response = CircuitInfoResponse::new(
                zk_id,
                Addr::unchecked("sidecar"),
                Binary::from(slot.circuit_key.to_vec()),
            );
            match to_json_binary(&response) {
                Ok(bin) => SystemResult::Ok(ContractResult::Ok(bin)),
                Err(_) => SystemResult::Err(SystemError::NoSuchCircuit { zk_id }),
            }
        }
        None => SystemResult::Err(SystemError::NoSuchCircuit { zk_id }),
    }
}

impl Querier for HostQuerier {
    fn query_raw(
        &self,
        request: &[u8],
        _gas_limit: u64,
    ) -> BackendResult<SystemResult<ContractResult<Binary>>> {
        let gas = GasInfo::with_cost(GAS_COST_STORAGE);
        let parsed: QueryRequest<Empty> = match from_json(request) {
            Ok(parsed) => parsed,
            Err(_) => return (Err(BackendError::user_err("query")), gas),
        };
        let result = match parsed {
            QueryRequest::Wasm(WasmQuery::CircuitInfo { zk_id }) => {
                circuit_info(&self.registry, zk_id)
            }
            QueryRequest::Wasm(WasmQuery::Circuit { zk_id }) => {
                SystemResult::Err(SystemError::NoSuchCircuit { zk_id })
            }
            _ => SystemResult::Err(SystemError::UnsupportedRequest {
                kind: "query".into(),
            }),
        };
        (Ok(result), gas)
    }
}

fn refill(fe: &mut FunctionEnvMut<HostEnv>) -> Result<(), String> {
    let (env, mut store) = fe.data_and_store_mut();
    env.set_gas_left(&mut store, GAS_REFILL);
    Ok(())
}

fn alloc_write(fe: &mut FunctionEnvMut<HostEnv>, data: &[u8]) -> Result<u32, String> {
    let (env, mut store) = fe.data_and_store_mut();
    let result = env
        .call_function1(&mut store, "allocate", &[Value::I32(data.len() as i32)])
        .map_err(|_| "allocate")?;
    let ptr = match result {
        Value::I32(value) if value > 0 => value as u32,
        _ => return Err("allocate".into()),
    };
    write_region(env, &mut store, ptr, data).map_err(|_| "allocate")?;
    Ok(ptr)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use bech32::{encode, Bech32, Hrp};
    use cosmwasm_std::{
        to_json_vec, Binary, Empty, Order, QueryRequest, SystemError, SystemResult, WasmQuery,
    };

    use cosmwasm_vm::{BackendApi, Querier, Storage};

    use super::{HostApi, HostOutcome, HostQuerier, MemoryStorage, Registry, VmVerifier};
    use headstash_epoch::{toy_vk_proof, toy_vk_proof_offset};

    #[test]
    fn two_registered_keys_select_and_reject() {
        let first = toy_vk_proof(8, 3).expect("k8");
        let second = toy_vk_proof_offset(10, 9).expect("k10");
        assert_ne!(first.blob, second.blob);
        let vm = VmVerifier::open().expect("vm");
        vm.register(1, &first.blob).expect("reg 1");
        vm.register(2, &second.blob).expect("reg 2");
        assert_eq!(
            vm.check(1, &first.proof, &first.instances),
            HostOutcome::Valid
        );
        assert_eq!(
            vm.check(2, &second.proof, &second.instances),
            HostOutcome::Valid
        );
        assert_eq!(
            vm.check(2, &first.proof, &first.instances),
            HostOutcome::Rejected
        );
        assert_eq!(
            vm.check(1, &second.proof, &second.instances),
            HostOutcome::Rejected
        );
        assert_eq!(
            vm.check(99, &first.proof, &first.instances),
            HostOutcome::Unavailable
        );
    }

    #[test]
    fn membership_and_epoch_keys_select_and_reject() {
        use headstash_epoch::{
            epoch_circuit_blob, host_public_inputs, membership_host_proof, prove_epoch_claim,
            EpochStatement, EPOCH_VK_ID, MEMBERSHIP_VK_ID,
        };

        let membership = membership_host_proof().expect("membership");
        let recipient = [0u8; 32];
        let ctx = [0x11u8; 32];
        let destination = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw";
        let bundle = prove_epoch_claim(destination, recipient, 15, ctx).expect("epoch");
        let statement = EpochStatement {
            ctx: bundle.ctx,
            root: bundle.root,
            nullifier: bundle.nullifier,
            destination: bundle.destination.clone(),
            amount: bundle.amount,
            recipient,
        };
        let epoch_instances = host_public_inputs(&statement).expect("instances");
        let epoch_blob = epoch_circuit_blob();
        let vm = VmVerifier::open().expect("vm");
        let epoch_hash = vm.register(EPOCH_VK_ID, epoch_blob).expect("reg epoch");
        let membership_hash = vm
            .register(MEMBERSHIP_VK_ID, &membership.blob)
            .expect("reg membership");
        assert_eq!(&epoch_hash[..4], &[0x9f, 0x80, 0x9a, 0x09]);
        assert_eq!(&membership_hash[..4], &[0x94, 0xd4, 0x32, 0x61]);
        assert_eq!(
            vm.check(MEMBERSHIP_VK_ID, &membership.proof, &membership.instances),
            HostOutcome::Valid
        );
        assert_eq!(
            vm.check(EPOCH_VK_ID, &bundle.proof, &epoch_instances),
            HostOutcome::Valid
        );
        let mut padded = membership.instances.clone();
        padded.extend_from_slice(&[0u8; 32 * 3]);
        assert_eq!(
            vm.check(EPOCH_VK_ID, &membership.proof, &padded),
            HostOutcome::Rejected
        );
        assert_eq!(
            vm.check(MEMBERSHIP_VK_ID, &bundle.proof, &epoch_instances[..32 * 6]),
            HostOutcome::Rejected
        );
    }

    fn host_api_round_trips_terp_addresses() {
        let api = HostApi::new();
        let zeros = [0u8; 20];
        let (human, _) = api.addr_humanize(&zeros);
        assert_eq!(
            human.expect("humanize"),
            "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw"
        );
        let (canonical, _) =
            api.addr_canonicalize("terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw");
        assert_eq!(canonical.expect("canonicalize"), zeros);
        assert!(api
            .addr_validate("terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw")
            .0
            .is_ok());

        let wide = [0u8; 32];
        let (human32, _) = api.addr_humanize(&wide);
        let human32 = human32.expect("humanize 32");
        let (back, _) = api.addr_canonicalize(&human32);
        assert_eq!(back.expect("canonicalize 32"), wide);
        assert!(api.addr_humanize(&[0u8; 10]).0.is_err());

        let other = encode::<Bech32>(Hrp::parse("cosmos").expect("hrp"), &zeros).expect("encode");
        assert!(api.addr_canonicalize(&other).0.is_err());
    }

    #[test]
    fn memory_storage_sets_and_gets() {
        let mut storage = MemoryStorage::new();
        assert_eq!(storage.get(b"a").0.expect("get"), None);
        storage.set(b"a", b"one").0.expect("set");
        storage.set(b"b", b"two").0.expect("set");
        assert_eq!(storage.get(b"a").0.expect("get").expect("some"), b"one");
        let id = storage
            .scan(None, None, Order::Ascending)
            .0
            .expect("scan");
        assert_eq!(id, 1);
        let first = storage.next(id).0.expect("next").expect("row");
        assert_eq!(first.0, b"a");
        let second = storage.next(id).0.expect("next").expect("row");
        assert_eq!(second.0, b"b");
        assert!(storage.next(id).0.expect("done").is_none());
        assert!(storage.next(9).0.is_err());
        storage.remove(b"a").0.expect("remove");
        assert_eq!(storage.get(b"a").0.expect("get"), None);
    }

    #[test]
    fn host_querier_refuses_circuit_and_other_queries() {
        let registry = Arc::new(Mutex::new(Registry {
            by_id: HashMap::new(),
            by_key: HashMap::new(),
        }));
        let querier = HostQuerier { registry };
        let circuit = to_json_vec(&QueryRequest::<Empty>::Wasm(WasmQuery::Circuit { zk_id: 1 }))
            .expect("json");
        match querier.query_raw(&circuit, 1_000_000).0.expect("backend") {
            SystemResult::Err(SystemError::NoSuchCircuit { zk_id: 1 }) => {}
            other => panic!("circuit query was not refused: {other:?}"),
        }
        let smart = to_json_vec(&QueryRequest::<Empty>::Wasm(WasmQuery::Smart {
            contract_addr: "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw".into(),
            msg: Binary::from(b"{}".as_slice()),
        }))
        .expect("json");
        match querier.query_raw(&smart, 1_000_000).0.expect("backend") {
            SystemResult::Err(SystemError::UnsupportedRequest { .. }) => {}
            other => panic!("smart query was not refused: {other:?}"),
        }
    }
}
