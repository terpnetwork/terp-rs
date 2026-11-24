#[cfg(not(target_arch = "wasm32"))]
include!("sezs-consume-verify_host.rs");

#[cfg(target_arch = "wasm32")]
fn main() {}
