//! Standalone HTTP runtime (faucet-adjacent).
//!
//!   ONBOARD_DAO=terp1... ONBOARD_RUNTIME=terp1... ONBOARD_CHAIN=120u-1 \
//!     cargo run -p terp-onboard-authz --bin onboard-runtime
//!
//! FROST inbox is the same process on HASHMERCHANT_BIND / ITL_FROST_BIND (127.0.0.1:19091).

use terp_onboard_authz::frost_http;
use terp_onboard_authz::http::serve;
use terp_onboard_authz::OnboardRuntime;

fn main() {
    let port = std::env::var("ONBOARD_PORT").unwrap_or_else(|_| "5001".into());
    let host = std::env::var("ONBOARD_BIND").unwrap_or_else(|_| "127.0.0.1".into());
    let bind = format!("{host}:{port}");
    let runtime = std::sync::Arc::new(OnboardRuntime::from_env());
    match frost_http::loopback_bind() {
        Ok(fb) => match frost_http::shared_inbox_from_env() {
            Ok(inbox) => {
                std::thread::spawn(move || {
                    if let Err(e) = frost_http::serve(&fb, inbox) {
                        eprintln!("frost-inbox {e}");
                    }
                });
            }
            Err(e) => eprintln!("frost-inbox {e}"),
        },
        Err(e) => eprintln!("frost-inbox {e}"),
    }
    serve(&bind, runtime).expect("bind");
}
