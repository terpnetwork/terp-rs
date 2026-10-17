pub mod contract;
pub mod error;
pub mod execute;
pub mod msg;
pub mod query;
pub mod state;

pub use contract::{execute, instantiate, query, sudo, CONTRACT_NAME};
pub use error::ContractError;
pub use msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

// cw-orch's prelude is host-only. The optimizer builds default features, which
// include `interface`, so this stays out of the wasm guest.
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::NftfiLoan;
