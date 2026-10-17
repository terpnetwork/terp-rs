//! main DeployData implementations for suites in test
#[cfg(feature = "interface")]
use crate::suites::headstash::HeadstashDeployData;
use cw_norick::interface::NoRickDeployData;
#[cfg(feature = "multicore")]
use rayon::prelude::*;

use cosmwasm_std::Addr;

pub trait DeployData {}

#[derive(Debug, thiserror::Error)]
pub enum ZkDeployError {
    #[error("cw-orch error: {0}")]
    CwEnv(#[from] cw_orch::prelude::CwOrchError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("deploy error: {0}")]
    Other(String),
}

/// Deploy configuration for the ZK headstash system.
#[derive(Clone)]
pub struct TestPressDeployData {
    pub no_rick: NoRickDeployData,
    pub headstash: HeadstashDeployData,
}

impl TestPressDeployData {
    pub fn local_default(admin: Addr, genesis_root: &[u8], keys_dir: std::path::PathBuf) -> Self {
        Self {
            no_rick: NoRickDeployData {},
            headstash: HeadstashDeployData::local_default(admin, genesis_root).expect("oooooohhh"),
        }
    }
}
// ── TestPressSuite ────────────────────────────────────────────────────────────
use cw_orch::environment::ZkCwEnv;

/// Composed suite of **all** test-press circuit suites.
///
/// Retains every method from the headstash helpers (via the same trait blanket
/// impls) and adds one `TerpVmSuite`-compatible field per test circuit.
///
/// # Feature gating
/// The circuit-suite fields (`no_rick`, …) are only present when the
/// `interface` feature is enabled (pulls in ict-rs).  The struct is still
/// usable without that feature — it is then just a zero-sized wrapper that
/// provides all the `HeadstashBitwiseInstance` / tree / IPFS / launchpad
/// helpers.
///
/// # Layout (keys dir)
/// ```text
/// <keys_base>/
///   no_rick/
///     params.bin
///     proving_key.bin
///     verifying_key.bin
/// ```
pub struct TestPressSuite<Chain: ZkCwEnv> {
    /// No-Rick circuit: key management, prove, verify, on-chain deploy.
    #[cfg(feature = "interface")]
    pub no_rick: crate::suites::no_rick::interface::NoRickSuite<Chain>,
    /// Headstash Orchard circuit: production spend-proof circuit.
    #[cfg(feature = "interface")]
    pub headstash: crate::suites::headstash::HeadstashSuite<Chain>,
    // sinsemilla merkle tree
    // zk-hashmerchant (proove know mirrored tree root path )
    // plonky3
    // groth16
    // cario
    // zk-tls
}

// ── TestPressSuite constructors & Deploy ──────────────────────────────────────

#[cfg(feature = "interface")]
impl<Chain: ZkCwEnv + cw_orch::prelude::CircuitUploadable> TestPressSuite<Chain> {
    /// Build an un-deployed suite from a chain handle.
    pub fn new(chain: Chain) -> Self
    where
        Chain: Clone,
    {
        Self {
            no_rick: crate::suites::no_rick::interface::NoRickSuite::new(chain.clone()),
            headstash: crate::suites::headstash::HeadstashSuite::new(chain),
        }
    }
}

#[cfg(feature = "interface")]
impl<Chain: ZkCwEnv> cw_orch::prelude::Deploy<Chain> for TestPressSuite<Chain> {
    type DeployData = Option<TestPressDeployData>;
    type Error = ZkDeployError;

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let no_rick = crate::suites::no_rick::interface::NoRickSuite::new(chain.clone());
        let headstash = crate::suites::headstash::HeadstashSuite::new(chain.clone());

        // Upload circuits — call on the .circuit field which implements CwOrchCircuitUpload
        use cw_orch::prelude::CwOrchCircuitUpload;
        no_rick
            .circuit
            .upload_circuit()
            .map_err(ZkDeployError::CwEnv)?;
        headstash
            .circuit
            .upload_circuit()
            .map_err(ZkDeployError::CwEnv)?;

        Ok(Self { no_rick, headstash })
    }

    fn store_on(_chain: Chain) -> Result<Self, Self::Error> {
        todo!()
    }

    fn get_contracts_mut(
        &mut self,
    ) -> Vec<Box<&mut dyn cw_orch::prelude::ContractInstance<Chain>>> {
        todo!()
    }

    fn load_from(_chain: Chain) -> Result<Self, Self::Error> {
        todo!()
    }
}

/// TerpHeadstashConfig
#[derive(Debug)]
pub struct TerpHeadstashConfig {}
