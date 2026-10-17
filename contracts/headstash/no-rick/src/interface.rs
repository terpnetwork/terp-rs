use crate::*;
use cosmwasm_std::Empty;
use cw_orch::prelude::*;

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = NORICK_CONTRACT)]
pub struct NoRickContractSuite;

impl<Chain: CwEnv> Uploadable for NoRickContractSuite<Chain> {
    /// Path to the compiled wasm artifact (used with Daemon).
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path(NORICK_CONTRACT)
            .unwrap()
    }

    /// CosmWasm contract wrapper for cw-multi-test environments.
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}

impl<Chain: CwEnv> Deploy<Chain> for NoRickContractSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = NoRickDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let contract = Self::new(chain);
        contract.upload()?;
        Ok(contract)
    }

    fn deploy_on(chain: Chain, _data: Self::DeployData) -> Result<Self, Self::Error> {
        let contract = Self::store_on(chain)?;
        // Sender is admin for tokenfactory denom creation in instantiate.
        contract.instantiate(&InstantiateMsg {}, None, &[])?;
        Ok(contract)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(self)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoRickDeployData {}
