//! Private-bridge (cw-private-dex) as a cw-orch suite using Path A circuits.
//!
//! Lab still allows `mock_verify`. Production path: instantiate with `zkid` +
//! [`FlockLigeritoCircuit`] footer (same Path A as circuit demos).

use cosmwasm_std::{Addr, Binary, Uint128};
use cw_orch::prelude::*;
use cw_private_dex::interface::CwPrivateDexContract;
use cw_private_dex::msg::{ExecuteMsg, InstantiateMsg};

use crate::interfaces::FlockLigeritoCircuit;

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
    pub dex: CwPrivateDexContract<Chain>,
    pub spend: FlockLigeritoCircuit<Chain>,
}

impl<Chain: CwEnv> TerpBridgeSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            dex: CwPrivateDexContract::new(chain.clone()),
            spend: FlockLigeritoCircuit::new(chain),
        }
    }

    pub fn create_lab_pool(&self) -> Result<<Chain as TxHandler>::Response, CwOrchError> {
        self.dex.execute(
            &ExecuteMsg::CreatePool {
                asset_a: Binary::from(vec![1u8; 32]),
                asset_b: Binary::from(vec![2u8; 32]),
                r_a: Uint128::new(1_000_000),
                r_b: Uint128::new(1_000_000),
                gamma: 997,
                gamma_den: 1000,
            },
            &[],
        )
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
        vec![Box::new(&mut self.dex)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = Self::store_on(chain)?;
        suite.dex.instantiate(
            &InstantiateMsg {
                mock_verify: data.mock_verify,
                zkid: data.zkid,
                allowed_root: data.allowed_root,
            },
            Some(&data.admin),
            &[],
        )?;
        Ok(suite)
    }
}
