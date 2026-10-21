//! `cw-private-dex` deploy handle. [`crate::TerpBridgeSuite`] pairs this with the spend circuit.

use cosmwasm_std::{Addr, Binary, Uint128};
use cw_orch::prelude::*;
use cw_private_dex::interface::CwPrivateDexContract;
use cw_private_dex::msg::{ExecuteMsg, InstantiateMsg};

pub struct PrivateDexSuite<Chain: CwEnv> {
    pub dex: CwPrivateDexContract<Chain>,
}

impl<Chain: CwEnv> PrivateDexSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            dex: CwPrivateDexContract::new(chain),
        }
    }

    pub fn code_id(&self) -> Result<u64, CwOrchError> {
        self.dex.code_id()
    }

    pub fn addr_str(&self) -> Result<String, CwOrchError> {
        self.dex.addr_str()
    }

    pub fn upload(&self) -> Result<(), CwOrchError> {
        self.dex.upload()?;
        Ok(())
    }

    pub fn upload_if_needed(&self) -> Result<(), CwOrchError> {
        self.dex.upload_if_needed()?;
        Ok(())
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

    pub fn instantiate(
        &self,
        mock_verify: bool,
        zkid: Option<u64>,
        allowed_root: Option<Binary>,
        admin: &Addr,
    ) -> Result<(), CwOrchError> {
        self.dex.instantiate(
            &InstantiateMsg {
                mock_verify,
                zkid,
                allowed_root,
            },
            Some(admin),
            &[],
        )?;
        Ok(())
    }
}
