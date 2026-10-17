pub use interface::NoRickCircuitSuite;
pub mod interface {
    use crate::circuits::no_rick::ProvingKey;
    use cw_orch::prelude::*;

    use cw_norick::interface::NoRickDeployData;
    use cw_norick::{NORICK_CONTRACT, NoRickContractSuite};

    pub struct NoRickSuite<Chain: ZkCwEnv> {
        /// The zero-sized circuit type; provides keygen / prove / verify.
        pub circuit: NoRickCircuitSuite<Chain>,
        /// The zero-sized contract type; provides wasm bytes + VK validation.
        pub contract: NoRickContractSuite<Chain>,
    }

    impl<Chain: ZkCwEnv> NoRickSuite<Chain> {
        pub fn new(chain: Chain) -> Self {
            Self {
                circuit: NoRickCircuitSuite::new(chain.clone()),
                contract: NoRickContractSuite::new(chain.clone()),
            }
        }
        pub fn deploy_on(chain: Chain, data: NoRickDeployData) -> Result<Self, CwOrchError> {
            Ok(Self {
                circuit: NoRickCircuitSuite::deploy_on(chain.clone(), data.clone())?,
                contract: NoRickContractSuite::deploy_on(chain.clone(), data)?,
            })
        }
    }

    #[cw_orch::circuit_interface(id = NORICK_CONTRACT, artifacts_dir = "artifacts")]
    pub struct NoRickCircuitSuite;

    impl<Chain: ZkCwEnv> NoRickCircuitSuite<Chain> {
        /// Generate a complete E2E test bundle with circuit keys, merkle tree, and proofs.
        ///
        /// This is the canonical method for generating headstash test data.
        /// Returns all artifacts needed for E2E testing.
        pub fn build_keys(&self) -> Result<ProvingKey, cw_orch::anyhow::Error> {
            Ok(ProvingKey::build_and_write(Self::vk_path())
                .map_err(|e| cw_orch::anyhow::format_err!(e.to_string()))?)
        }
    }

    impl<Chain: ZkCwEnv> Deploy<Chain> for NoRickCircuitSuite<Chain> {
        type Error = CwOrchError;
        type DeployData = NoRickDeployData;
        fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
            Ok(Self::store_on(chain.clone())?)
        }

        fn store_on(chain: Chain) -> Result<Self, Self::Error> {
            let suite = Self::new(chain);
            suite.upload_circuit()?;
            Ok(suite)
        }

        fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
            // noop
            vec![]
        }

        fn load_from(_chain: Chain) -> Result<Self, Self::Error> {
            todo!()
        }
    }
}
