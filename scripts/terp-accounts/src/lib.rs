pub mod suite;
pub use suite::TerpAccountSuite;

// re-export contract cw-orch functions & entrypoint types
pub use terp721_account::msg::{
    AsyncTerp721AccountsQueryMsgFns, ExecuteMsg as Bs721AccountExecuteMsgTypes,
    ExecuteMsgFns as TerpAccountExecuteFns, Terp721AccountsQueryMsg, Terp721AccountsQueryMsgFns,
};
pub use terp_account::manifold::{
    AsyncQueryMsgFns as TerpAccountMinterAsyncQueryMsgFns,
    ExecuteMsg as Bs721AccountMinterExecuteMsgTypes, ExecuteMsgFns as TerpAccountMinterExecuteFns,
    QueryMsg as TerpAccountMinterQueryMsgTypes, QueryMsgFns as TerpAccountMinterQueryMsgFns,
};
// pub use cw721_base::msg::{
//     AsyncQueryMsgFns as TerpAccountMinterAsyncQueryMsgFn, ExecuteMsgFns as Terp721BaseExecuteFns,
//     QueryMsg as TerpAccountQueryMsgTypes, QueryMsgFns as TerpAccountQueryMsgFns,
// };
pub use terp_account::manifold::{
    AsyncQueryMsgFns as TerpAccountMarketAsyncQueryMsgFns,
    ExecuteMsg as Bs721AccountMarketExecuteMsgTypes, ExecuteMsgFns as TerpAccountMarketExecuteFns,
    QueryMsg as TerpAccountMarketQueryMsgTypes, QueryMsgFns as TerpAccountMarketQueryFns,
};

// pub use account_registry_middleware::{
//     AsyncQueryMsgFns as AccountRegistryAsyncQueryMsgFns,
//     ExecuteMsg as AccountRegistryExecuteMsgTypes, ExecuteMsgFns as AccountRegistryExecuteFns,
//     QueryMsg as AccountRegistryQueryMsgTypes, QueryMsgFns as AccountRegistryQueryFns,
// };

pub use ownership_verifier::{
    ExecuteMsg as TestOwnershipExecuteMsg, ExecuteMsgFns as TestOwnershipExecuteMsgFns,
    InstantiateMsg as TestOwnershipInitMsg, QueryMsg as TestOwnershipQueryMsg,
    QueryMsgFns as TestOwnershipQueryMsgFns,
};

#[cfg(test)]
pub mod test;
