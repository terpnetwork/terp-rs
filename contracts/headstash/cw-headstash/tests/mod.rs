// Test modules. They live under support/ so cargo does not also build each
// one as its own test target, where `crate::tokenfactory_stargate` does not exist.
#[path = "support/test_factory.rs"]
pub mod test_factory;
#[path = "support/test_headstash.rs"]
pub mod test_headstash;
#[path = "support/test_insanity.rs"]
pub mod test_insanity;
#[path = "support/test_integration.rs"]
pub mod test_integration;
pub mod tokenfactory_stargate;
pub mod wavs_fixture;
