#[cfg(feature = "interface")]
pub mod headstash;
#[cfg(feature = "interface")]
pub mod no_rick;
/// Compose private-bridge suite scaffold (round-1). See `E2E-HARNESS-PLAN.md`.
#[cfg(feature = "interface")]
pub mod private_bridge;
/// cw-private-dex suite (CreatePool / SettleSwap) — G3 harness settle.
#[cfg(feature = "interface")]
pub mod private_dex;
/// Halo2 epoch + public Pedersen batch on cw-orch Mock.
#[cfg(feature = "interface")]
pub mod epoch_claim;
