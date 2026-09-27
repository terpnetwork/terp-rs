//! TerpNetworkSuite — compose product cw-orch suites the operator actually uses.
//!
//! Default `cargo check -p terp-scripts` stays green: auth / orch / pdex are feature-gated.
//! Private-bridge Path A lives in `terp-orch` (`--features orch-suite`).

use cw_orch::prelude::*;
use std::collections::HashMap;

use super::deploy_data::TerpNetworkDeployData;

/// Unified handle bag. Missing feature = field absent, not a stub suite.
pub struct TerpNetworkSuite<Chain: CwEnv> {
    pub chain: Chain,
    #[cfg(feature = "auth-suite-full")]
    pub auth: Option<terp_authenticator_suite::TerpAuthenticatorSuite<Chain>>,
    #[cfg(feature = "orch-suite")]
    pub orch: Option<terp_orch::TerpOrchSuite<Chain>>,
    #[cfg(feature = "zk")]
    pub pdex: Option<zk_test_press::suites::private_dex::PrivateDexSuite<Chain>>,
}

impl<Chain: CwEnv> TerpNetworkSuite<Chain>
where
    CwOrchError: From<<Chain as TxHandler>::Error>,
{
    pub fn new(chain: Chain) -> Self {
        Self {
            chain,
            #[cfg(feature = "auth-suite-full")]
            auth: None,
            #[cfg(feature = "orch-suite")]
            orch: None,
            #[cfg(feature = "zk")]
            pdex: None,
        }
    }

    /// Bind suites from Daemon / Mock state (no txs).
    pub fn load_from(chain: Chain, data: &TerpNetworkDeployData) -> Result<Self, CwOrchError> {
        let mut suite = Self::new(chain.clone());
        #[cfg(feature = "auth-suite-full")]
        if data.attach_auth {
            suite.auth = Some(<terp_authenticator_suite::TerpAuthenticatorSuite<Chain> as cw_orch::contract::Deploy<Chain>>::load_from(
                chain.clone(),
            )?);
        }
        #[cfg(feature = "orch-suite")]
        if data.attach_orch {
            suite.orch = Some(<terp_orch::TerpOrchSuite<Chain> as cw_orch::contract::Deploy<Chain>>::load_from(
                chain.clone(),
            )?);
        }
        #[cfg(feature = "zk")]
        if data.attach_pdex {
            suite.pdex = Some(<zk_test_press::suites::private_dex::PrivateDexSuite<Chain> as cw_orch::contract::Deploy<Chain>>::load_from(
                chain,
            )?);
        }
        #[cfg(not(any(feature = "auth-suite-full", feature = "orch-suite", feature = "zk")))]
        let _ = (chain, data);
        Ok(suite)
    }

    /// Upload wasm only when code_ids are missing.
    pub fn store_if_needed(
        chain: Chain,
        data: &TerpNetworkDeployData,
    ) -> Result<Self, CwOrchError> {
        let mut suite = Self::load_from(chain.clone(), data)?;
        #[cfg(feature = "auth-suite-full")]
        if let Some(auth) = &suite.auth {
            auth.upload_if_needed()?;
        }
        #[cfg(feature = "zk")]
        if let Some(pdex) = &suite.pdex {
            pdex.upload_if_needed()?;
        }
        let _ = chain;
        Ok(suite)
    }

    pub fn deploy_on(chain: Chain, data: TerpNetworkDeployData) -> Result<Self, CwOrchError> {
        Self::store_if_needed(chain, &data)
    }

    pub fn collect_addresses(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    pub fn print_addresses(&self) {
        for (key, addr) in self.collect_addresses() {
            println!("CONTRACT_ADDR:{key}={addr}");
        }
    }
}

