#[cfg(not(target_arch = "wasm32"))]
include!("server_host.rs");

#[cfg(target_arch = "wasm32")]
fn main() {}
