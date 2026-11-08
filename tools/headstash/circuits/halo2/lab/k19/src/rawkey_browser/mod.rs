//! Browser build of the sound raw-key claim: the secp chip and `blind_k19` compiled from their
//! host sources without libsecp (the host `circuit` module needs host-crypto).
#![allow(dead_code, unused_imports)]
#[path = "../circuit/gadget/secp_host_field.rs"]
mod secp_host_field;
#[path = "../circuit/gadget/secp256k1_chip.rs"]
pub mod secp256k1_chip;
pub mod circuit {
    pub use super::secp256k1_chip::{Secp256k1Chip, Secp256k1Config, Secp256k1Fp, Secp256k1Fq};
    pub use super::gadget;
}
pub mod gadget {
    pub use super::secp256k1_chip;
}
#[path = "../blind_k19.rs"]
pub mod blind_k19;
