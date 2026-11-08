// Not a top-level file: cargo would build it as its own test target, where
// `crate::tokenfactory_stargate` does not exist.
#[path = "support/test_env.rs"]
pub mod test_env;
pub mod tokenfactory_stargate;
pub mod wavs_fixture;
