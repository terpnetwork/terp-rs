#[cfg(not(target_arch = "wasm32"))]
include!("local-workflow_host.rs");

#[cfg(target_arch = "wasm32")]
fn main() {}
