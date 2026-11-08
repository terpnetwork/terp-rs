//! Path A circuit bag (Stwo / Flock). Field `circuits` on `TerpArtifactStack`.

use cosmwasm_std::Addr;
use cw_orch::prelude::*;

use crate::interfaces::{CwZkVerifyDemo, FlockLigeritoCircuit, StwoSsleCircuit};

#[derive(Clone, Debug)]
pub struct CircuitDemoDeployData {
    pub admin: Addr,
    pub zkid: u64,
}

impl From<Addr> for CircuitDemoDeployData {
    fn from(admin: Addr) -> Self {
        Self { admin, zkid: 7 }
    }
}

pub struct CircuitDemoSuite<Chain: CwEnv> {
    pub verify: CwZkVerifyDemo<Chain>,
    pub stwo: StwoSsleCircuit<Chain>,
    pub flock: FlockLigeritoCircuit<Chain>,
}

impl<Chain: CwEnv> CircuitDemoSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            verify: CwZkVerifyDemo::new(chain.clone()),
            stwo: StwoSsleCircuit::new(chain.clone()),
            flock: FlockLigeritoCircuit::new(chain),
        }
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for CircuitDemoSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = CircuitDemoDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.verify.upload_if_needed()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(&mut self.verify)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = Self::store_on(chain)?;
        suite.verify.instantiate(
            &cw_zk_verify_demo::InstantiateMsg { zkid: data.zkid },
            Some(&data.admin),
            &[],
        )?;
        Ok(suite)
    }
}
