use cw_orch::{
    core::CwEnvError,
    prelude::{ContractInstance, CwEnv, CwOrchUpload, Deploy},
};

use crate::interfaces::{cw_420::Cw420, CwAve, CwAveFactory};

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
            cw420: Cw420::new(chain.clone()),
        }
    }
}

impl<Chain: CwEnv> Deploy<Chain> for CwAveSuite<Chain> {
    type Error = CwEnvError;

    type DeployData = ();

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let cw_ave = CwAve::new(chain.clone());
        let cw_ave_factory = CwAveFactory::new(chain.clone());
        let cw420 = Cw420::new(chain.clone());

        cw_ave.upload()?;
        cw_ave_factory.upload()?;
        cw420.upload()?;

        Ok(Self {
            cw_ave,
            cw_ave_factory,
            cw420,
        })
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![
            Box::new(&mut self.cw_ave),
            Box::new(&mut self.cw_ave_factory),
        ]
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = Self::store_on(chain.clone())?;

        Ok(suite)
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        // grab data from deployment state
        todo!()
    }
}
