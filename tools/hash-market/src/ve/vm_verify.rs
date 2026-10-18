//! Embedded CosmWasm VM verifier for halo2 claims.
//!
//! The host method is `do_proof_instance_verify` (`Api::proof_instance_verify`).
//! Keys are deserialized in `register`. A claim only selects one by `vk_id`.
//!
//! Wasmer's `Store` is not `Send`. `Halo2Host` is `Send + Sync`, so the store
//! stays on a worker thread. Callers send a check and wait for the outcome.

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use cosmwasm_std::{
    to_json_binary, Addr, Binary, CircuitInfoResponse, ContractResult, SystemError, SystemResult,
    WasmQuery,
};
use cosmwasm_vm::testing::{MockApi, MockQuerier, MockStorage};
use cosmwasm_vm::{
    compile_module, do_proof_instance_verify, write_region, AnyVerifyingKey, CircuitLoader,
    Environment, Size,
};
use sha2::{Digest, Sha256};
use wasmer::{imports, FunctionEnv, FunctionEnvMut, Instance, Module, Store, Value};

use headstash_epoch::{
    epoch_circuit_blob, epoch_vk_hash, install_halo2_host, Halo2Host, HostVerifyError, EPOCH_VK_ID,
};

/// Refill before each claim. An epoch proof is about 5.7e9 gas.
const GAS_REFILL: u64 = 50_000_000_000;

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

type HostEnv = Environment<MockApi, MockStorage, MockQuerier>;

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
        let environment = Environment::new(MockApi::default(), GAS_REFILL);
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
        let querier = circuit_querier(registry);
        {
            let mut fe = env.clone().into_mut(&mut store);
            let (data, mut store_mut) = fe.data_and_store_mut();
            data.set_wasmer_instance(Some(instance_ptr));
            data.set_gas_left(&mut store_mut, GAS_REFILL);
            data.set_storage_readonly(false);
            data.move_in(MockStorage::new(), querier);
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

fn circuit_querier(registry: Arc<Mutex<Registry>>) -> MockQuerier {
    let mut querier = MockQuerier::new(&[]);
    querier.update_wasm(move |query| match query {
        WasmQuery::CircuitInfo { zk_id } => {
            let guard = match registry.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return SystemResult::Err(SystemError::NoSuchCircuit { zk_id: *zk_id });
                }
            };
            match guard.by_id.get(zk_id) {
                Some(slot) => {
                    let response = CircuitInfoResponse::new(
                        *zk_id,
                        Addr::unchecked("sidecar"),
                        Binary::from(slot.circuit_key.to_vec()),
                    );
                    match to_json_binary(&response) {
                        Ok(bin) => SystemResult::Ok(ContractResult::Ok(bin)),
                        Err(_) => SystemResult::Err(SystemError::NoSuchCircuit { zk_id: *zk_id }),
                    }
                }
                None => SystemResult::Err(SystemError::NoSuchCircuit { zk_id: *zk_id }),
            }
        }
        WasmQuery::Circuit { zk_id } => {
            SystemResult::Err(SystemError::NoSuchCircuit { zk_id: *zk_id })
        }
        _ => SystemResult::Err(SystemError::UnsupportedRequest {
            kind: "query".into(),
        }),
    });
    querier
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
    use super::{HostOutcome, VmVerifier};
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
}
