# Atomic NFT ↔ ZEC settlement (workstream A, round 6)

**Goal.** The NFT leaves escrow only after the seller's shielded ZEC payment is in a confirmed Zcash block. Zcash
shielded notes have no scripts or timelocks, so atomicity comes from Terp verifying the payment, as the existing
zap-1 bridges already do.

## Model (zap-1 inclusion attestation, as in the bridges)
- **Light client:** workstream B's `cw-lc-zcash` (IBC v2 client over `cw-lc-registry` + `cw-lc-aggregator`).
  `VerifyPaymentInclusion` checks a zap-1 leaf and Merkle path against the committee-attested `payment_root` of a header,
  with IBC v2 `app_data_hash = H(note ‖ amount ‖ frost_group ‖ leaf)` (the existing `cw-ics08-wasm-zap1::verify_zap1_ibcv2`).
  It records the inclusion under `app_data_hash`.
- **Auction:** `FinalizePrivateAuction { …, payment_ref }` queries `PaymentInclusion { app_data_hash_hex: payment_ref }` at the
  auction's `payment_client` and requires:
  1. `status == Confirmed`. `Preconfirmed` (committee-signed, below depth) and `Reorged` never release; status is re-checked live by the client.
  2. `note == release.payment_cm`. The seller payment note is signed 6-of-6 inside the Finalize digest.
  3. `frost_group == bid.reclaim_vk`. The payment is spent from this bid's escrow group.
  4. `leaf == zap1 program_entry("tsa1:" ‖ hex(memo_cm))`. The payment memo names this contract, lot and bid index.
  5. `payment_ref` is unused (`PRIV_PAYMENTS`). One inclusion settles one auction.
- The client types are mirrored in `svg-auction/src/private.rs` (so the auction wasm does not link the light client), field for
  field, because `cw_serde` rejects unknown fields. terp-orch checks the mirror round-trips with `cw_lc_zcash::msg`
  (`payment_client_wire_matches_cw_lc_zcash`).

## Himess/seam, curated for IBC v2 + CosmWasm
seam is a Zcash light client on EVM: an Equihash header chain plus a RISC Zero payment proof bound to a memo reference.
What we take and what we change:

| seam | here |
|---|---|
| memo reference names the order | `memo_cm` (domain-separated, contract + lot + bid index) → memo `tsa1:<hex>` → zap-1 `program_entry` leaf |
| header verification in the contract (Equihash, ~576k gas) | k-of-n committee header attestation + confirmation depth (B). In-contract Equihash is the trust-minimised upgrade path. |
| STARK payment proof (RISC Zero, ~77 s on GPU) | zap-1 Merkle inclusion against `payment_root` (~4.8k SDK gas, B bench). |
| confirmation by `confirmationWork` | depth-based `Confirmed` from the aggregator; chain-work confirmation is B's suggested follow-up. |
| EVM escrow release | `FinalizePrivateAuction` needs the 6-of-6 FROST signature **and** the inclusion. |

## Protocol
1. B, P and S sign `Finalize` 6-of-6 over the agreed `release` (including `payment_cm`).
2. {B,P} spend the escrow note to the seller payment `payment_cm` (memo `tsa1:<memo_cm>`), plus fee and change.
   This consumes the escrow nullifier, so the pre-signed refund can't land after it.
3. Operators attest the header carrying the payment root; anyone submits `VerifyPaymentInclusion` to the client.
4. Once the header reaches depth (`Confirmed`), anyone submits `FinalizePrivateAuction{payment_ref}`. The NFT moves to the bidder.
- If the payment never confirms, there's no Finalize. B's solo reclaim after grace and the 5-of-6 refund still work,
  and anyone can expire the auction so the NFT returns to the seller.

## Tests
- svg-auction `finalize_waits_for_a_confirmed_zcash_payment_of_this_bid`: missing inclusion, Preconfirmed and Reorged are rejected;
  wrong note, group or memo leaf is rejected; Confirmed releases; the same `payment_ref` can't settle twice.
- terp-orch `private_auction_flow`: offline it uses `MockZcashLc`, a cw-orch contract with the same `PaymentInclusion` query,
  where the instantiator records inclusions in place of the header committee. Finalize is attempted at Preconfirmed (rejected),
  then at Confirmed (NFT moves).
- **Real path:** `PaymentClient::External(addr)` points the flow at a deployed `cw-lc-zcash`. TODO for the live suite: drive B's
  registry/aggregator header attestation and `VerifyPaymentInclusion` from the flow (B's client is shipped; only the live wiring is left).

## Trust and limits
- Trust moves from "P orders the steps honestly" to B's light client: the k-of-n header committee and confirmation depth.
  A committee majority could attest a fake payment root. In-contract Equihash or a payment proof removes that, at seam-level cost.
- The amount is not compared on Terp: it is hidden. `payment_cm` is a note commitment that binds value and recipient, and the
  seller agreed to it in the 6-of-6 signature after opening the bid.
- **Flag (zap1, unchanged):** the existing zap1 verifier's `hosting_payment` leaf hardcodes month 8 / year 2026. The auction does not
  use that kind (it uses `program_entry`), so it isn't affected. A dedicated auction/transfer leaf kind would need a zap1 change (owner call).
