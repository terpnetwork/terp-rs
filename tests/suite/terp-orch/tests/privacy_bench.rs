//! Workstream A baselines for `BENCHMARKS.md [A]`: valar `vote_proof` keygen / prove /
//! verify times, and cw-vote-ceremony gas on the optimized wasm (cosmwasm-vm metering,
//! same columns as `cw-lc-zcash/tests/vm_gas.rs`). Auction gas: `svg-auction`
//! `private_tests::vm_gas`.
//!
//! Run (release; keygen + 5 proofs take minutes in debug):
//! `BENCH_WASM_DIR=<dir with cw_vote_ceremony.wasm> BENCH_OUT=<file.md> cargo test
//! -p terp-orch --features privacy --release --test privacy_bench -- --ignored --nocapture`
#![cfg(feature = "privacy")]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use cosmwasm_std::testing::{message_info, mock_env};
use cosmwasm_std::{
    Addr, Binary, ContractResult, CosmosMsg, Env, HexBinary, Order, Response, SystemResult,
    Timestamp, Uint128, WasmQuery, coins, from_json, to_json_binary,
};
use cosmwasm_vm::testing::{
    MockApi, MockInstanceOptions, MockQuerier, MockStorage, execute, instantiate,
    mock_instance_with_options, query,
};
use cosmwasm_vm::{Instance, Storage};
use cw_vote_ceremony::msg::{
    AnchorRef, ExecuteMsg as ZkExec, InstantiateMsg as ZkInit, QueryMsg as ZkQuery,
    TallyAttestation, TallyAuth, ZkModuleInit, ZkProposal,
};
use cw_vote_ceremony::zk::{self, slot};
use serde::Serialize;
use terp_orch::suites::privacy::frost_util::Group;
use terp_orch::suites::privacy::zk_voting::{CastProver, ValarVoteProver};

type Inst = Instance<MockApi, MockStorage, MockQuerier>;
const ZKID: u64 = 7;
const BALLOTS: usize = 5;
const PERIOD: u64 = 3600;
const CAPS: &str = "iterator,staking,stargate,bulk_memory,ibc2,hash_blake,hash_poseidon,bn254,redpallas,cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1,cosmwasm_2_2,cosmwasm_3_0";

fn med(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}
fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1e3)
}
fn kv(i: &mut Inst) -> BTreeMap<Vec<u8>, Vec<u8>> {
    i.with_storage(|s| {
        let id = s.scan(None, None, Order::Ascending).0.unwrap();
        Ok(s.all(id).0.unwrap().into_iter().collect())
    })
    .unwrap()
}
fn used(i: &mut Inst) -> u64 {
    let r = i.create_gas_report();
    r.used_internally + r.used_externally
}

/// (call, vm_gas, keys written, bytes written, wall time)
#[derive(Default)]
struct Meter(Vec<(String, u64, usize, usize, Duration)>);
impl Meter {
    fn exec<M: Serialize>(&mut self, call: &str, i: &mut Inst, env: Env, who: &Addr, msg: &M) {
        let (k0, g0, t) = (kv(i), used(i), Instant::now());
        let r: ContractResult<Response> = if call.starts_with("instantiate") {
            instantiate(i, env, message_info(who, &[]), msg)
        } else {
            execute(i, env, message_info(who, &[]), msg)
        };
        let dt = t.elapsed();
        r.into_result().unwrap_or_else(|e| panic!("{call}: {e}"));
        let (k1, g1) = (kv(i), used(i));
        let w: Vec<_> = k1.iter().filter(|(k, v)| k0.get(*k) != Some(*v)).collect();
        let bytes = w.iter().map(|(k, v)| k.len() + v.len()).sum();
        self.0.push((call.into(), g1 - g0, w.len(), bytes, dt));
    }
    fn table(&self) -> String {
        let mut s = String::from(
            "| contract | call | vm_gas | sdk_vm | kv_w | kv_bytes | sdk_kv_w | sdk_est (lower bound) | VM wall (ms) |\n|---|---|---:|---:|---:|---:|---:|---:|---:|\n",
        );
        for (call, vm, n, b, dt) in &self.0 {
            let (sdk_vm, sdk_kv) = (vm.div_ceil(140_000), *n as u64 * 2000 + *b as u64 * 30);
            s += &format!(
                "| cw-vote-ceremony | {call} | {vm} | {sdk_vm} | {n} | {b} | {sdk_kv} | {} | {} |\n",
                sdk_vm + sdk_kv,
                ms(*dt)
            );
        }
        s
    }
}
fn q<M: Serialize, R: serde::de::DeserializeOwned>(i: &mut Inst, msg: &M) -> R {
    from_json(query(i, mock_env(), msg).into_result().unwrap()).unwrap()
}
fn env_at(t: u64) -> Env {
    let mut e = mock_env();
    e.block.time = Timestamp::from_seconds(t);
    e
}

#[test]
#[ignore = "release-mode benchmark; needs BENCH_WASM_DIR"]
fn vote_proof_timing_and_vote_gas() {
    let api = cosmwasm_std::testing::MockApi::default();
    // Keygen: params setup + keygen_vk + keygen_pk (first call in the process), then the
    // chain blob (params || cs || vk || footer).
    let t = Instant::now();
    let mut prover = ValarVoteProver::new(ZKID).unwrap();
    let keygen_total = t.elapsed();
    let (blob, key) = (prover.stored.blob.clone(), prover.stored.circuit_key().unwrap());
    let verify = prover.stored.host_verifier().unwrap();

    // VM: the module, with a querier serving voting power and the stored circuit.
    let dir = std::env::var("BENCH_WASM_DIR").expect("BENCH_WASM_DIR");
    let wasm = std::fs::read(format!("{dir}/cw_vote_ceremony.wasm")).unwrap();
    let mut i = mock_instance_with_options(
        &wasm,
        MockInstanceOptions {
            gas_limit: u64::MAX / 2,
            available_capabilities: cosmwasm_vm::capabilities_from_csv(CAPS),
            ..Default::default()
        },
    );
    let (dao, voting, member) = (api.addr_make("dao"), api.addr_make("voting"), api.addr_make("member"));
    let (served_key, served_blob, v) = (key.to_vec(), blob.clone(), voting.to_string());
    i.with_querier(|qr| {
        qr.update_wasm(move |w| {
            let r = match w {
                WasmQuery::Smart { contract_addr, msg } if *contract_addr == v => {
                    let total = String::from_utf8_lossy(msg).contains("total_power");
                    serde_json::json!({ "power": if total { "5" } else { "1" }, "height": 1 })
                }
                WasmQuery::CircuitInfo { zk_id } => serde_json::json!({
                    "zk_id": zk_id,
                    "creator": "uploader",
                    "circuit_key": Binary::from(served_key.clone()),
                }),
                WasmQuery::Circuit { .. } => {
                    serde_json::json!({ "data": Binary::from(served_blob.clone()) })
                }
                _ => panic!("unexpected query {w:?}"),
            };
            SystemResult::Ok(ContractResult::Ok(to_json_binary(&r).unwrap()))
        });
        Ok(())
    })
    .unwrap();
    let committee = Group::dealer(3, 2).unwrap();
    let mut m = Meter::default();
    let t0 = mock_env().block.time.seconds();
    m.exec(
        "instantiate (zk module, pinned circuit key)",
        &mut i,
        env_at(t0),
        &dao,
        &ZkInit {
            admin: None,
            max_nullifiers_per_mark: 8,
            zk_proposals: Some(ZkModuleInit {
                dao: Some(dao.to_string()),
                voting_module: Some(voting.to_string()),
                cast_zkid: ZKID,
                cast_circuit_key: Some(HexBinary::from(key.to_vec())),
                anchor_registry: None,
                tally: TallyAuth::Committee { group_vk: committee.vk().unwrap() },
                voting_period: PERIOD,
                threshold_bps: 5_000,
                quorum_bps: 3_000,
                require_rvpk_sig: true,
            }),
        },
    );
    let env = mock_env();
    let round = zk::round_id(&api, &env.block.chain_id, env.contract.address.as_str(), 1);
    let e = prover.election(round).unwrap();
    m.exec(
        "ZkPropose",
        &mut i,
        env_at(t0),
        &member,
        &ZkExec::ZkPropose {
            title: "bench".into(),
            description: String::new(),
            msgs: vec![CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
                to_address: member.to_string(),
                amount: coins(1, "uterp"),
            })],
            ea_pk: HexBinary::from(e.ea_pk.to_vec()),
            anchor: AnchorRef::Root {
                root: HexBinary::from(e.tree_root.to_vec()),
                height: e.anchor_height,
            },
        },
    );
    let p: ZkProposal = q(&mut i, &ZkQuery::ZkProposal { proposal_id: 1 });

    let (mut verify_t, mut proof_len, mut ballot_len) = (vec![], 0, 0);
    let relayer = api.addr_make("relayer");
    for voter in 0..BALLOTS {
        let b = prover.prove(&p, voter).unwrap();
        for _ in 0..3 {
            let t = Instant::now();
            assert!(verify(&b.proof, &b.instances).unwrap(), "host verify");
            verify_t.push(t.elapsed());
        }
        (proof_len, ballot_len) = (b.proof.len(), b.ballot.len());
        let vc = slot::VOTE_COMMITMENT * 32..slot::VOTE_COMMITMENT * 32 + 32;
        let sighash: HexBinary = q(
            &mut i,
            &ZkQuery::ZkCastSighash {
                proposal_id: 1,
                nullifier: HexBinary::from(&b.instances[..32]),
                vote_commitment: HexBinary::from(&b.instances[vc]),
                ballot: b.ballot.clone(),
            },
        );
        let sig = prover.sign(&b, sighash.as_slice()).unwrap();
        m.exec(
            &format!("ZkCastVote #{} (Halo2 k=11 verify + RedPallas)", voter + 1),
            &mut i,
            env_at(t0 + 10),
            &relayer,
            &ZkExec::ZkCastVote {
                proposal_id: 1,
                proof: b.proof,
                instances: Binary::from(b.instances),
                ballot: b.ballot,
                rvpk_sig: Some(sig),
            },
        );
    }
    let (yes, no, abstain) = (Uint128::new(3), Uint128::new(2), Uint128::zero());
    let digest: HexBinary =
        q(&mut i, &ZkQuery::ZkTallyDigest { proposal_id: 1, yes, no, abstain });
    m.exec(
        "ZkSubmitTally (FROST 2-of-3 committee)",
        &mut i,
        env_at(t0 + PERIOD + 1),
        &relayer,
        &ZkExec::ZkSubmitTally {
            proposal_id: 1,
            yes,
            no,
            abstain,
            attestation: TallyAttestation::Committee {
                signature: committee.sign(digest.as_slice(), &[1, 2]).unwrap(),
            },
        },
    );
    m.exec(
        "ZkExecute",
        &mut i,
        env_at(t0 + PERIOD + 2),
        &relayer,
        &ZkExec::ZkExecute { proposal_id: 1 },
    );

    let prove = prover.prove_times().to_vec();
    let units = 1 + proof_len.div_ceil(1024).max(1) + 11;
    let mut s = format!(
        "| metric | value |\n|---|---:|\n\
         | keygen: params + keygen_vk + keygen_pk + chain blob (ms, once) | {} |\n\
         | prove `build_vote_proof_from_delegation` (ms, median of {}) | {} |\n\
         | prove min / max (ms) | {} / {} |\n\
         | verify off-chain, host dispatch (`AnyVerifyingKey`, ms, median of {}) | {} |\n\
         | proof bytes | {proof_len} |\n\
         | public inputs | 11 × 32 B = 352 B |\n\
         | ballot ciphertext bytes (16 ElGamal shares) | {ballot_len} |\n\
         | stored circuit blob bytes (params ‖ cs ‖ vk ‖ footer) | {} |\n\
         | `proof_instance_verify` scheduled units | {units} |\n\n",
        ms(keygen_total),
        prove.len(),
        ms(med(prove.clone())),
        ms(*prove.iter().min().unwrap()),
        ms(*prove.iter().max().unwrap()),
        verify_t.len(),
        ms(med(verify_t)),
        blob.len(),
    );
    s += &m.table();
    println!("{s}");
    if let Ok(out) = std::env::var("BENCH_OUT") {
        std::fs::write(out, &s).unwrap();
    }
}
