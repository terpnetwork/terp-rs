//! Private-bridge (cw-private-dex) as a cw-orch suite using Path A circuits.
//!
//! Lab still allows `mock_verify`. Production path: instantiate with `zkid` +
//! [`FlockLigeritoCircuit`] footer (same Path A as circuit demos).

use cosmwasm_std::{Addr, Binary};
use cw_orch::prelude::*;

use crate::interfaces::FlockLigeritoCircuit;
use crate::suites::private_dex::PrivateDexSuite;

#[derive(Clone, Debug)]
pub struct TerpBridgeDeployData {
    pub admin: Addr,
    /// Lab: true. Path A host verify: false + `zkid`.
    pub mock_verify: bool,
    pub zkid: Option<u64>,
    pub allowed_root: Option<Binary>,
}

impl From<Addr> for TerpBridgeDeployData {
    fn from(admin: Addr) -> Self {
        Self {
            admin,
            mock_verify: true,
            zkid: None,
            allowed_root: Some(Binary::from(vec![7u8; 32])),
        }
    }
}

/// Corridor settle + spend-circuit handle.
pub struct TerpBridgeSuite<Chain: CwEnv> {
    pub dex: PrivateDexSuite<Chain>,
    pub spend: FlockLigeritoCircuit<Chain>,
}

impl<Chain: CwEnv> TerpBridgeSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            dex: PrivateDexSuite::new(chain.clone()),
            spend: FlockLigeritoCircuit::new(chain),
        }
    }

    pub fn create_lab_pool(&self) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.dex.create_lab_pool()
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for TerpBridgeSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = TerpBridgeDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.dex.upload_if_needed()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(&mut self.dex.dex)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = Self::store_on(chain)?;
        suite
            .dex
            .instantiate(data.mock_verify, data.zkid, data.allowed_root, &data.admin)?;
        Ok(suite)
    }
}
