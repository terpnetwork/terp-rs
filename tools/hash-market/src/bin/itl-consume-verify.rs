#[cfg(not(target_arch = "wasm32"))]
include!("itl-consume-verify_host.rs");

#[cfg(target_arch = "wasm32")]
fn main() {}
