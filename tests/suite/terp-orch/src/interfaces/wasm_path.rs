//! Shared artifact lookup for suite interfaces.
//!
//! Optimized wasm lives in `crates/terp-rs/artifacts/`. The cw-orch workspace
//! walk stops at this crate's own `artifacts/` (circuit VK blobs), so the path
//! is relative to this crate's manifest.

use std::path::PathBuf;

use cw_orch::prelude::*;

pub fn wasm(name: &str) -> WasmPath {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../artifacts");
    ArtifactsDir::new(dir)
        .find_wasm_path(name)
        .unwrap_or_else(|err| panic!("{name}.wasm is not in crates/terp-rs/artifacts ({err})"))
}
