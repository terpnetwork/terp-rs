//! Ave, its factory, and cw420. The contract handles are `TerpArtifactStack::products`.

use cw_orch::prelude::*;

use crate::interfaces::products::{Cw420, CwAve, CwAveFactory};

#[derive(Clone)]
pub struct CwAveSuite<Chain: CwEnv> {
    pub cw_ave: CwAve<Chain>,
    pub cw_ave_factory: CwAveFactory<Chain>,
    pub cw420: Cw420<Chain>,
}

impl<Chain: CwEnv> CwAveSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            cw_ave: CwAve::new(chain.clone()),
            cw_ave_factory: CwAveFactory::new(chain.clone()),
            cw420: Cw420::new(chain),
        }
    }
}

impl<Chain: CwEnv> Deploy<Chain> for CwAveSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = ();

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.cw_ave.upload()?;
        suite.cw_ave_factory.upload()?;
        suite.cw420.upload()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![
            Box::new(&mut self.cw_ave),
            Box::new(&mut self.cw_ave_factory),
            Box::new(&mut self.cw420),
        ]
    }

    fn deploy_on(chain: Chain, _data: Self::DeployData) -> Result<Self, Self::Error> {
        Self::store_on(chain)
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }
}
