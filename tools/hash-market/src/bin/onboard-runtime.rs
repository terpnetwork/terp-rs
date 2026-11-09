#[cfg(not(target_arch = "wasm32"))]
include!("onboard-runtime_host.rs");

#[cfg(target_arch = "wasm32")]
fn main() {}
