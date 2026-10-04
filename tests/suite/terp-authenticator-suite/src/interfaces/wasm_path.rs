//! Optimizer wasm is `crates/terp-rs/artifacts/`, not this crate's tree.

use std::path::PathBuf;

use cw_orch::prelude::*;

pub fn wasm(name: &str) -> WasmPath {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../artifacts");
    ArtifactsDir::new(dir)
        .find_wasm_path(name)
        .unwrap_or_else(|err| panic!("{name}.wasm is not in crates/terp-rs/artifacts ({err})"))
}
