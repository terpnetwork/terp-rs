#[cfg(not(target_arch = "wasm32"))]
include!("client_host.rs");

#[cfg(target_arch = "wasm32")]
fn main() {}
