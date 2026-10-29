//! Loopback FROST inbox. HashMerchant adapter, not FilePV.
//!
//!   ITL_FROST_PERSIST_KEY=sixteen-chars-ok ITL_FROST_PERSIST_DIR=/tmp/itl-frost \
//!     HASHMERCHANT_BIND=127.0.0.1:19091 cargo run --bin frost-inbox

fn main() {
    let bind = match hash_market::onboard_authz::frost_http::loopback_bind() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let inbox = match hash_market::onboard_authz::frost_http::shared_inbox_from_env() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = hash_market::onboard_authz::frost_http::serve(&bind, inbox) {
        eprintln!("frost-inbox {e}");
        std::process::exit(1);
    }
}
