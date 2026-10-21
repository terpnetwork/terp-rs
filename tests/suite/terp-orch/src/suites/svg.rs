//! SVG collection, minter, infuser, whitelist, and the shitstrap bag.

use cw_infuser::interface::CwInfuser;
use cw_orch::prelude::*;
use cw_svg_minter::InstantiateMsg as SvgMinterInitMsg;
use cw_svg_minter::interface::Cw721SvgMinter;
use cw_svg_minter::msg::ExecuteMsgFns as _;
use cw721_svg::Cw721SvgContractSuite;
use whitelist_mtree::interface::WhitelistMerkleTree;

use super::shitstrap::{CwShitstrapSuite, CwShitstrapSuiteDeployData};

#[derive(Clone, Debug)]
pub struct CwSvgSuiteDeployData {
    pub svg: Vec<(
        cw721_svg::InstantiateMsg,
        Option<whitelist_mtree::msg::InstantiateMsg>,
    )>,
    pub infuse: Option<cw_infuser::msg::InstantiateMsg>,
    pub shit: Option<CwShitstrapSuiteDeployData>,
    pub admin: Option<Addr>,
    pub infuse_coins: Vec<Coin>,
}

pub struct CwSvgSuite<Chain> {
    pub chain: Chain,
    pub infuser: CwInfuser<Chain>,
    pub cwsvg: Cw721SvgContractSuite<Chain>,
    pub minter: Cw721SvgMinter<Chain>,
    pub shit: CwShitstrapSuite<Chain>,
    pub mtree: WhitelistMerkleTree<Chain>,
    pub svgs: Vec<String>,
}

impl<Chain: CwEnv> CwSvgSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            chain: chain.clone(),
            infuser: CwInfuser::new(chain.clone()),
            cwsvg: Cw721SvgContractSuite::new(chain.clone()),
            minter: Cw721SvgMinter::new(chain.clone()),
            shit: CwShitstrapSuite::new(chain.clone()),
            mtree: WhitelistMerkleTree::new(chain),
            svgs: Vec::new(),
        }
    }

    pub fn upload(&self) -> Result<(), CwOrchError> {
        self.cwsvg.upload_if_needed()?;
        self.minter.upload_if_needed()?;
        self.mtree.upload_if_needed()?;
        self.infuser.upload_if_needed()?;
        self.shit.upload_if_needed()?;
        Ok(())
    }
}

impl<Chain: CwEnv> Deploy<Chain> for CwSvgSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = Option<CwSvgSuiteDeployData>;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.upload()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        let mut contracts = self.shit.get_contracts_mut();
        contracts.push(Box::new(&mut self.cwsvg));
        contracts.push(Box::new(&mut self.minter));
        contracts.push(Box::new(&mut self.infuser));
        contracts.push(Box::new(&mut self.mtree));
        contracts
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let mut suite = Self::store_on(chain.clone())?;
        if let Some(init) = data {
            let admin = init.admin.as_ref();
            let owner = Some(chain.sender_addr().to_string());
            suite.minter.instantiate(
                &SvgMinterInitMsg {
                    owner,
                    svg_code_id: suite.cwsvg.code_id()?,
                    dregg_ibc_only: false,
                    runtime_minters: vec![],
                },
                admin,
                &[],
            )?;
            if !init.svg.is_empty() {
                for (mut init_msg, mt) in init.svg {
                    if let Some(mtree) = mt {
                        let mtree_addr = suite
                            .mtree
                            .instantiate(&mtree, admin, &[])?
                            .instantiated_contract_address()?;
                        init_msg.collection_info_extension.whitelist = Some(mtree_addr.to_string());
                    }
                    let svg = suite
                        .minter
                        .create_svg_collection(init_msg, "cwsvg-colllection")?
                        .event_attr_value("wasm", "contract")?;
                    suite.svgs.push(svg);
                }
                suite
                    .cwsvg
                    .set_address(&Addr::unchecked(suite.svgs.last().unwrap()));
            }
            if let Some(infuse) = &init.infuse {
                suite.infuser.instantiate(infuse, admin, &[])?;
            }
        }
        Ok(suite)
    }
}
