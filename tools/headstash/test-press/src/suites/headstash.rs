//! `TerpVmSuite` implementation for the **ZK Headstash Orchard circuit**.
//!
//! This is the production ZK circuit powering `cw-headstash` shielded-pool
//! operations: spend proofs over Merkle paths with Poseidon commitments.
//!
//! # Keys directory layout
//! ```text
//! <keys_dir>/
//!   params.bin          — SRS / commitment parameters (vesta::Affine)
//!   verifying_key.bin   — halo2 VK
//!   proving_key.bin     — VK written for reference (pk rebuilt on load)
//!   vk_combined.bin     — params || vk || cs || 32-byte footer (store-circuit)
//! ```

use std::string::String;

use cw_headstash::interface::HeadstashContract;
use cw_headstash_manifold::interface::CwHeadstashManifold;
use cw_headstash_manifold::msg::{
    ExecuteMsgFns, InstantiateMsg as ManifoldInstantiateMsg, QueryMsgFns as _,
};
use cw_orch::{anyhow, prelude::*};
use zk_headstash::suite::HeadstashCircuitSuite;

use cosmwasm_std::{Addr, Binary};
use cw_headstash::tokenfactory::{HeadstashTokenObject, TokenStrategy};
use cw_headstash::wavs::{WavsAuthMetadata, WavsProofOfOwnership};
pub use zk_headstash::suite::suite::HeadstashTestDataGenerator as _;

/// ZK headstash deployment suite: cw-headstash contract + manifold factory.
///
/// Wraps the cw-orch interfaces for both contracts and provides
/// a `deploy_on` constructor matching the pattern used by other suites.
pub struct HeadstashSuite<Chain: ZkCwEnv> {
    pub headstash: HeadstashContract<Chain>,
    pub manifold: CwHeadstashManifold<Chain>,
    pub circuit: HeadstashCircuitSuite<Chain>,
}

impl<Chain: ZkCwEnv> HeadstashSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        let circuit = HeadstashCircuitSuite::new(chain.clone());
        let manifold = CwHeadstashManifold::new(chain.clone());
        let headstash = HeadstashContract::new(chain.clone());
        Self {
            headstash,
            manifold,
            circuit,
        }
    }
}

impl<Chain: ZkCwEnv + cw_orch::prelude::CircuitUploadable> Deploy<Chain> for HeadstashSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = HeadstashDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = HeadstashSuite::new(chain.clone());
        suite.circuit.upload_circuit()?;
        suite.manifold.upload()?;
        suite.headstash.upload()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(&mut self.manifold), Box::new(&mut self.headstash)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let circuit = HeadstashCircuitSuite::new(chain.clone());
        let manifold = CwHeadstashManifold::new(chain.clone());
        let headstash = HeadstashContract::new(chain.clone());

        circuit.upload_circuit()?;
        manifold.upload()?;
        headstash.upload()?;
        manifold.instantiate(
            &ManifoldInstantiateMsg {
                owner: data.owner,
                headstash_code_id: headstash.code_id()?,
            },
            None,
            &[],
        )?;
        manifold.create_headstash(data.headstash_init, None, None)?;

        Ok(Self {
            headstash,
            manifold,
            circuit,
        })
    }
}

/// Deploy configuration for the ZK headstash system.
#[derive(Clone, Debug)]
pub struct HeadstashDeployData {
    /// Owner address for the manifold factory.
    pub owner: Option<String>,
    /// Instantiate message for the cw-headstash contract.
    pub headstash_init: cw_headstash::msg::InstantiateMsg,
}

impl HeadstashDeployData {
    /// Create deploy data for a local test deployment.
    ///
    /// `genesis_root` is the hex-encoded merkle root of the headstash tree.
    /// `vk_combined_path` is the path to `vk_combined.bin` (optional).
    /// Create deploy data for a local test deployment.
    ///
    /// Uses `ExistingFungible` token strategy with `uterp` for simplicity.
    /// `genesis_root` is the raw merkle root bytes.
    /// `vk_combined_path` points to `vk_combined.bin` (optional).
    pub fn local_default(admin: Addr, genesis_root: &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            owner: Some(admin.to_string()),
            headstash_init: cw_headstash::msg::InstantiateMsg {
                genesis_root: Binary::from(genesis_root.to_vec()),
                distro_hash_domain: Default::default(),
                genesis_label: None,
                token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject {
                    proof: Binary::default(),
                    raw: "uterp".into(),
                }),
                wavs: WavsProofOfOwnership {
                    poos: vec![],
                    msg: WavsAuthMetadata {
                        aggregate_key: String::new(),
                        threshold: 0,
                        total_operators: 0,
                        nonce: 0,
                    },
                },
                circuit_id: None,
            },
        })
    }
}
