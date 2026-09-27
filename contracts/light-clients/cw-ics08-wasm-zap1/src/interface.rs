//! cw-orch interface for the ZAP1 IBC v2 08-wasm client (not Crosslink).

use cw_orch::{interface, prelude::*};

use crate::{ExecuteMsg, InstantiateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw-ics08-wasm-zap1";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CONTRACT_ID)]
pub struct CwIcs08WasmZap1Contract;

impl<Chain> Uploadable for CwIcs08WasmZap1Contract<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for rel in [
            "../../artifacts/cw_ics08_wasm_zap1.wasm",
            "artifacts/cw_ics08_wasm_zap1.wasm",
            "target/wasm32-unknown-unknown/release/cw_ics08_wasm_zap1.wasm",
        ] {
            let p = root.join(rel);
            if p.is_file() {
                return WasmPath::new(p).expect("cw-ics08-wasm-zap1 wasm");
            }
        }
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_ics08_wasm_zap1")
            .unwrap_or_else(|_| {
                WasmPath::new(root.join("Cargo.toml")).expect("crate root")
            })
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            crate::execute,
            crate::instantiate,
            crate::query,
        ))
    }
}
