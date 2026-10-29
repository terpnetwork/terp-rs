#[cfg(not(target_arch = "wasm32"))]
include!("frost-inbox_host.rs");

#[cfg(target_arch = "wasm32")]
fn main() {}
