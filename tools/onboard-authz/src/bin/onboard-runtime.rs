//! Standalone HTTP runtime (faucet-adjacent).
//!
//!   ONBOARD_DAO=terp1... ONBOARD_RUNTIME=terp1... ONBOARD_CHAIN=120u-1 \
//!     cargo run -p terp-onboard-authz --bin onboard-runtime

use terp_onboard_authz::http::serve;
use terp_onboard_authz::OnboardRuntime;

fn main() {
    let port = std::env::var("ONBOARD_PORT").unwrap_or_else(|_| "5001".into());
    let host = std::env::var("ONBOARD_BIND").unwrap_or_else(|_| "127.0.0.1".into());
    let bind = format!("{host}:{port}");
    let runtime = std::sync::Arc::new(OnboardRuntime::from_env());
    serve(&bind, runtime).expect("bind");
}
