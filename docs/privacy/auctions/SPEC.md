# Private auctions — svg-auction extension (workstream A)

Code: `terp-rs/contracts/infuser/svg-auction/src/{private.rs,private_tests.rs}`, wiring in `src/lib.rs`.
Suite: `tests/suite/terp-orch/src/suites/privacy/{auctions,bid_view}.rs` (`private_auction_flow<Chain: CwEnv>`, `PrivateAuctionModule`,
seller view envelope, payment client); atomic settlement design: `auctions/ATOMICITY.md`;
e2e wrapper `contracts/infuser/svg-auction/e2e/tests/private_auction_e2e.rs`.
The public auction logic is unchanged; the private path is entered via a cw721 hook.

## Settlement is private by default (shielded ZEC)
No funds touch the contract. Every private message rejects attached funds. Bid amount, escrow value, payment amounts
and destinations exist only in the Zcash shielded pool. Terp stores 32-byte commitments and ciphertext, nothing else.
Optional public disclosure works by revealing a one-time outgoing viewing key (OVK).

## Flow
1. Seller `SendNft` with `Cw721HookMsg::CreatePrivateAuction { reclaim_grace?, seller_view_pk, payment_client }` (no denom,
   deposit floor or buckets). `seller_view_pk` (32 B X25519) receives every bid's view envelope; `payment_client` is the
   zap-1 payment light client (B's `cw-lc-zcash`) that gates Finalize (see ATOMICITY.md).
2. Per bid, bidder (B), platform (P) and seller (S) run **two fresh FROST-Ed25519 DKGs** over 6 shares:
   B = {1,2,3}, P = {4,5}, S = {6}. `K_reclaim` = 5-of-6, `K_finalize` = 6-of-6.
3. B funds a **shielded ZEC escrow note owned by `K_reclaim`** (`escrow_cm` = its note commitment). At bid time {B,P}
   also FROST-sign the **refund spend** (escrow → bidder) and hand it to B. That pre-signed refund is B's solo exit on
   Zcash.
4. `PlacePrivateBid { escrow_cm, commitment, sealed_bid, memo_cm, ovk_tag, reclaim_vk, finalize_vk, reclaim_bind_sig, finalize_bind_sig, dkg_transcript? }`
   - `commitment = blake2b(COMMIT_DOMAIN ‖ auction_be ‖ lp(bidder) ‖ escrow_cm ‖ amount_zat_u64_be ‖ salt32)`. It is opened to
     the seller only, through `sealed_bid` (72..=1024 B, never decrypted on chain).
   - `memo_cm = blake2b(MEMO_DOMAIN ‖ lp(chain_id) ‖ lp(contract) ‖ auction_be ‖ bid_index_be)` labels the bidding instance;
     the contract recomputes it with the next bid index and rejects any other value (wrong contract, lot or index).
   - `ovk_tag = blake2b(OVK_DOMAIN ‖ ovk)` fixes the bid's OVK at bid time. Every later release must carry this tag.
   - Bind sigs prove possession of both keys; the 6-of-6 one proves all three parties joined. Keys must be distinct and unused in the auction.
5. `ReclaimPrivateBid { release, signature? }` closes the bid and records the refund:
   - `Some(sig)`: `K_reclaim` over the `Reclaim` digest (binds `release`). Any relayer can submit, at any time.
   - `None`: **decided** — the bidder alone, after the auction closes or past `end + grace`, records the pre-signed
     refund. Kept as the liveness exit when P is gone.
6. `FinalizePrivateAuction { bid_id, release, signature, payment_ref }`: `K_finalize` over the `Finalize` digest, which binds the
   payment `release` (seller proceeds, platform fee, bidder change, `payment_cm`), **and** a `Confirmed` zap-1 inclusion of that
   seller payment at `payment_client` (`PaymentInclusion { app_data_hash_hex = payment_ref }`: note = `payment_cm`,
   frost_group = the bid's `reclaim_vk`, leaf = `program_entry("tsa1:" ‖ hex(memo_cm))`; each `payment_ref` settles once).
   `Preconfirmed` and `Reorged` do not release. Moves only the NFT to the bidder. No amount is revealed on Terp.
7. `DisclosePrivateRelease { bid_id, ovk }` (optional, anyone holding the OVK): it is accepted iff `blake2b(OVK_DOMAIN ‖ ovk) == ovk_tag`. The contract
   stores and emits it. Anyone can then decrypt `ovk_ciphertext` and check the notes against `notes_cm`.
8. `CancelPrivateAuction`: the seller **only while no bid is active**, or anyone after `end + grace`. The NFT returns to the seller.

### Seller view (`sealed_bid`)
The seller receives each bid's OVK and commitment opening, and only the seller can read them:
- `sealed_bid = epk(32) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, ovk ‖ amount_zat_be ‖ salt, aad = memo_cm)` (72 B overhead, 144 B total).
- `key = blake2b(VIEW_KDF ‖ X25519(esk, seller_view_pk) ‖ epk ‖ seller_view_pk)` and `esk = blake2b(VIEW_ESK ‖ ovk ‖ memo_cm)`,
  so the bidder re-opens its own envelope from its OVK, and keeps no extra secret.
- The seller opens all bids of its auction, checks `commitment` and `ovk_tag` against the openings, and ranks by `amount_zat`.
  The AEAD binds `memo_cm`, so an envelope copied into another auction or bid index does not open.
- Tests (terp-orch `bid_view`): `seller_opens_every_bid_and_ranks_them` (the highest wins), `third_party_and_wrong_label_cannot_open`
  (another key or another memo fails); the suite flow ranks 2 bids and finalizes the highest. svg-auction:
  `memo_must_label_this_auction_contract_and_bid_index`, `releases_use_the_ovk_fixed_at_bid_time`.

### Release record (`ShieldedRelease`)
- `notes_cm = blake2b(NOTES_DOMAIN ‖ n_u16_be ‖ lp(note_1) ‖ … ‖ lp(note_n))` over the 382-byte terp-seams SEAM-NOTE-OUT
  (`SeamNoteOutV0`) encodings of the payment outputs. Each note has a fresh random `rcm`, so the commitment hides values and recipients.
- `ovk_ciphertext`: JSON array of terp-seams `NoteEnvelope`s (XChaCha20-Poly1305 under the OVK), ≤ 8192 B.
- `ovk_tag = blake2b(OVK_DOMAIN ‖ ovk)`; it must equal the bid's `ovk_tag`.
- `payment_cm`: the seller payment note commitment. Required on Finalize (it is what the light client must attest), forbidden on Reclaim.
- Digest binding: `lp(notes_cm) ‖ lp(blake2b(ovk_ciphertext)) ‖ lp(ovk_tag) ‖ lp(payment_cm or empty)` is appended to the signing preimage. A relayer
  can't swap any part of it (`signatures_bind_the_payment_record`).

### Zcash leg (host side, reused from the tree)
Each output gets frost_escrow **Object A** (`escrow_release_digest(escrow_nf, cm, amount, ZEC)`) and **Object B**
(`ZcashSpendPackageV0`, `spend_sighash`), both FROST-signed by `K_reclaim` and checked with `verify_escrow_release_encoded` /
`verify_spend_encoded` under `encode_frost_public(5, 6, vk)`. The notes are terp-seams `SeamNoteOutV0`
(`abstract_leaf_cm`, `nullifier_lineage = escrow nf`), encrypted with `encrypt_note_out` and opened with `decrypt_note_out`.
The release spends the whole escrow: Σ outputs + `ZEC_FEE_ZAT` = escrow value.

Signing digest: `SIGN_DOMAIN(v2) ‖ tag ‖ lp(chain_id) ‖ lp(contract) ‖ auction_be ‖ lp(bidder) ‖ lp(escrow_cm) ‖ lp(commitment) ‖
lp(memo_cm) ‖ lp(ovk_tag) ‖ lp(reclaim_vk) ‖ lp(finalize_vk) ‖ bid_id_be(0 if none) ‖ lp(dkg_transcript) [‖ release]`, then `blake2b_256`. The digest
is bound to chain, contract, action, bid, both keys and the release. The `PrivateSigningDigest` query returns it.

## Threshold proof (unchanged, approved)
Share weights are w(B)=3, w(P)=2, w(S)=1, total 6. A coalition C can sign with K_t iff Σw(C) ≥ t.

| Coalition | weight | reclaim / escrow spend (≥5) | finalize (=6) |
|---|---|---|---|
| B | 3 | no | no |
| P, S, P+S | ≤3 | **no** | **no** |
| B+S | 4 | no | no |
| B+P | 5 | **yes** | no |
| B+P+S | 6 | yes | **yes** |

- No coalition without B reaches 5, so **no one can spend the bidder's escrow without the bidder**, on Terp or Zcash.
- Finalize needs all three. B+P can't force a sale, and P+S can do nothing.

## Exchange order and trust model
**Round 6: Finalize is gated on a Confirmed Zcash payment inclusion** (ATOMICITY.md). The order below stays, but step 3 now
needs the light client to attest the payment first, so the B+P residual below is closed (a refund instead of the payment never
yields a Confirmed inclusion of `payment_cm`). Original launch protocol, coordinated by P (who can never move funds without B):
1. All three sign `Finalize` over the agreed payment `release`. S's share is given now.
2. {B,P} sign the payment spends. P broadcasts them, which consumes the escrow nullifier, so the pre-signed refund can no longer land.
3. After confirmation, P submits `Finalize`. The NFT goes to B.
- S cannot cancel while bids are live (`seller_cancels_only_without_live_bids`), so S can't take the NFT back after payment.
- ~~Residual risk: B+P collusion (refund instead of pay, then finalize) robs S.~~ Closed in round 6 by the payment gate. What
  remains is the light client's trust (B's k-of-n header committee + depth); see ATOMICITY.md §Trust.
- Solo reclaim after close or grace (decided) records the pre-signed refund. If P disappears, B still exits on both chains.
- Anyone can expire an auction after grace, so the NFT never locks.

## Privacy
- Public: auction existence, bidder addresses, bid count, `escrow_cm`, bid commitment, release commitments and ciphertext sizes,
  winner identity (NFT transfer).
- Hidden: bid amounts, escrow value, payment amounts, fee and all destinations, unless the OVK is disclosed (then only that release is revealed).
- Round 6: svg-auction 20/20 (adds `memo_must_label_this_auction_contract_and_bid_index`, `releases_use_the_ovk_fixed_at_bid_time`,
  `finalize_waits_for_a_confirmed_zcash_payment_of_this_bid`); terp-orch `bid_view` 3/3 (incl. `payment_client_wire_matches_cw_lc_zcash`).
- Tests (12, svg-auction, round 5): `bids_carry_no_funds_and_no_amount`, `all_three_finalize_and_only_the_nft_moves`,
  `finalize_needs_every_party`, `signatures_bind_the_payment_record`, `bidder_and_platform_reclaim_any_time`,
  `platform_and_seller_can_never_act`, `bidder_alone_reclaims_after_close_or_timeout`, `seller_cancels_only_without_live_bids`,
  `anyone_expires_after_grace_even_with_bids`, `ovk_disclosure_is_optional_and_checked`, `bind_requires_both_groups_and_fresh_keys`,
  `digests_bind_chain_contract_action_and_release`. The suite runs the full flow, including OVK disclosure and
  `open_release` verification.
