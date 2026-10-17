//! cw-orch Daemon upload interface (`upload_if_needed`).
//! Artifact id: `cw_reg_eligibility`.

use cw_orch::{interface, prelude::*};

use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw_reg_eligibility";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CONTRACT_ID)]
pub struct CwRegEligibilityContract;

impl<Chain: CwEnv> Uploadable for CwRegEligibilityContract<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_reg_eligibility")
            .or_else(|_| {
                artifacts_dir_from_workspace!().find_wasm_path_from_crates_label("cw_reg_eligibility")
            })
            .expect("cw_reg_eligibility.wasm missing — wasm32-unknown-unknown release + copy to artifacts/")
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        unimplemented!(
            "cw_reg_eligibility is cosmwasm-std 1.5; Daemon uses wasm(). Local mock: orch/scripts/exercise-reg-vm-proof.sh"
        )
    }
}
