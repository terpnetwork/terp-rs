//! Benchmark-only contract. The headstash lane verifies proofs off-chain; this
//! crate exists to measure what on-chain verification would cost.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{entry_point, Binary, DepsMut, Env, MessageInfo, Response, StdResult};

#[cw_serde]
pub struct InstantiateMsg {}

#[cw_serde]
pub enum ExecuteMsg {
    /// Same calldata as BatchVerify, no host call: isolates verify gas.
    Baseline { zkid: u64, proofs: Vec<Binary>, instances: Vec<Binary> },
    Verify { zkid: u64, proof: Binary, instance: Binary },
    BatchVerify { zkid: u64, proofs: Vec<Binary>, instances: Vec<Binary> },
}

#[entry_point]
pub fn instantiate(_d: DepsMut, _e: Env, _i: MessageInfo, _m: InstantiateMsg) -> StdResult<Response> {
    Ok(Response::new())
}

#[entry_point]
pub fn execute(deps: DepsMut, _e: Env, _i: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    let (kind, n, res) = match msg {
        ExecuteMsg::Baseline { proofs, instances, .. } => {
            let bytes: usize = proofs.iter().chain(instances.iter()).map(|b| b.len()).sum();
            ("baseline", proofs.len(), Ok(bytes > 0))
        }
        ExecuteMsg::Verify { zkid, proof, instance } => {
            ("verify", 1, deps.api.proof_instance_verify(zkid, &proof, &instance))
        }
        ExecuteMsg::BatchVerify { zkid, proofs, instances } => {
            let ids = vec![zkid; proofs.len()];
            let p: Vec<&[u8]> = proofs.iter().map(|b| b.as_slice()).collect();
            let i: Vec<&[u8]> = instances.iter().map(|b| b.as_slice()).collect();
            ("batch", proofs.len(), deps.api.proof_instance_batch_verify(&ids, &p, &i))
        }
    };
    let (ok, err) = match res {
        Ok(b) => (b.to_string(), String::new()),
        Err(e) => ("error".to_string(), e.to_string()),
    };
    Ok(Response::new()
        .add_attribute("kind", kind)
        .add_attribute("n", n.to_string())
        .add_attribute("ok", ok)
        .add_attribute("err", err))
}
