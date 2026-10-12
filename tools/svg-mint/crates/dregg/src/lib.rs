//! Permissionless SVG mint cell (P0).
//!
//! User pays on ZEC Ironwood / ETH / Base / BTC. The cell attests deposit → entitle →
//! settle. IBC packets and mint metadata carry **commitments only** — never the
//! payment rail or foreign txid. Mentor can open `C_deposit`/`C_action` off-chain
//! and get `bound: bool`.
//!
//! Not HashMerchant. No feegrant. No 07-tendermint trusting period here (P4).

pub mod adr36;
pub mod commit;
pub mod packet;
pub mod cell;
pub mod mentor;
pub mod observer;
pub mod payout;
pub mod qc;
pub mod store;
pub mod memo;
pub mod zip321;
pub mod pedersen;
pub mod hd;
pub mod prices;
pub mod hopalong;
pub mod checkout;
pub mod httpx;
pub mod rpc_allowlist;
pub mod zec_tip;
pub mod zec_wallet;
pub mod zec_notes;

pub use cell::{Cell, CellError, SoldOutRefundAnswer, Turn};
pub use commit::{ActionOpening, DepositOpening, ForeignWitness, Rail};
pub use mentor::{Mentor, MentorOpening};
pub use packet::MintPacket;
