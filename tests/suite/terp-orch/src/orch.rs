//! Top-level bag: circuits + optional bridge on one `CwEnv`.

use cosmwasm_std::Addr;
#[cfg(feature = "bridge")]
use cosmwasm_std::Binary;
use cw_orch::prelude::*;

use crate::suite::{TerpCircuitDemoDeployData, TerpCircuitDemoSuite};

#[cfg(feature = "bridge")]
use crate::bridge::{TerpBridgeDeployData, TerpBridgeSuite};

#[derive(Clone, Debug)]
pub struct TerpOrchDeployData {
    pub admin: Addr,
    pub circuit_zkid: u64,
    pub attach_bridge: bool,
    pub bridge_mock_verify: bool,
    pub bridge_zkid: Option<u64>,
}

impl From<Addr> for TerpOrchDeployData {
    fn from(admin: Addr) -> Self {
        Self {
            admin,
            circuit_zkid: 7,
            attach_bridge: cfg!(feature = "bridge"),
            bridge_mock_verify: true,
            bridge_zkid: None,
        }
    }
}

pub struct TerpOrchSuite<Chain: CwEnv> {
    pub circuits: TerpCircuitDemoSuite<Chain>,
    #[cfg(feature = "bridge")]
    pub bridge: Option<TerpBridgeSuite<Chain>>,
}

impl<Chain: CwEnv> TerpOrchSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            circuits: TerpCircuitDemoSuite::new(chain.clone()),
            #[cfg(feature = "bridge")]
            bridge: Some(TerpBridgeSuite::new(chain)),
        }
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for TerpOrchSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = TerpOrchDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let circuits = TerpCircuitDemoSuite::store_on(chain.clone())?;
        #[cfg(feature = "bridge")]
        let bridge = Some(TerpBridgeSuite::store_on(chain)?);
        Ok(Self {
            circuits,
            #[cfg(feature = "bridge")]
            bridge,
        })
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        let mut v = self.circuits.get_contracts_mut();
        #[cfg(feature = "bridge")]
        if let Some(b) = &mut self.bridge {
            v.extend(b.get_contracts_mut());
        }
        v
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self {
            circuits: TerpCircuitDemoSuite::load_from(chain.clone())?,
            #[cfg(feature = "bridge")]
            bridge: Some(TerpBridgeSuite::load_from(chain)?),
        })
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let circuits = TerpCircuitDemoSuite::deploy_on(
            chain.clone(),
            TerpCircuitDemoDeployData {
                admin: data.admin.clone(),
                zkid: data.circuit_zkid,
            },
        )?;
        #[cfg(feature = "bridge")]
        let bridge = if data.attach_bridge {
            Some(TerpBridgeSuite::deploy_on(
                chain,
                TerpBridgeDeployData {
                    admin: data.admin,
                    mock_verify: data.bridge_mock_verify,
                    zkid: data.bridge_zkid,
                    allowed_root: Some(Binary::from(vec![7u8; 32])),
                },
            )?)
        } else {
            None
        };
        Ok(Self {
            circuits,
            #[cfg(feature = "bridge")]
            bridge,
        })
    }
}
