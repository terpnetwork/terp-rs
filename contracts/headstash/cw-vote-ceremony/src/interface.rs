//! cw-orch Daemon upload interface (`upload_if_needed`).
//! Artifact id: `cw_vote_ceremony`.

use cw_orch::{interface, prelude::*};

use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw_vote_ceremony";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CONTRACT_ID)]
pub struct CwVoteCeremonyContract;

impl<Chain: CwEnv> Uploadable for CwVoteCeremonyContract<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_vote_ceremony")
            .or_else(|_| {
                artifacts_dir_from_workspace!().find_wasm_path_from_crates_label("cw_vote_ceremony")
            })
            .expect("cw_vote_ceremony.wasm missing — wasm32-unknown-unknown release + copy to artifacts/")
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        unimplemented!(
            "cw_vote_ceremony is cosmwasm-std 1.5; Daemon uses wasm(). Local mock: orch/scripts/exercise-reg-vm-proof.sh"
        )
    }
}
