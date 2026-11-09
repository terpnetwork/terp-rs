// Copyright 2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

//! This is the top level documentation!

#[cfg(feature = "wasm")]
pub mod bindgen;

pub mod init;
pub mod wallet;
pub use wallet::{HeadstashWallet, MemoryHeadstashDb};

use wasm_bindgen::prelude::*;

/// The maximum number of checkpoints to store in each shard-tree
pub const PRUNING_DEPTH: usize = 100;

#[cfg(feature = "wasm-parallel")]
pub use wasm_bindgen_rayon::init_thread_pool;
// Snap workers (`@metamask/snaps-execution-environments`) do not expose
// SharedArrayBuffer. This stays a no-op so the JS API is stable; it does not
// speed Halo2. Host `multicore` is what the native bench measures.
#[cfg(not(feature = "wasm-parallel"))]
#[wasm_bindgen(js_name = initThreadPool)]
pub fn init_thread_pool(_threads: usize) {}
