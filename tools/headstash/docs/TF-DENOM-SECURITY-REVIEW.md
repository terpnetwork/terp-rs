# TF-denom (mint-on-claim) security review

Scope: `wip/headstash-tf-denom` at e01c2ea (plus the follow-ups 2b5ebb5 and a80277f) as it was folded into v0.0.1. The review covers the cw-headstash mint-on-claim path for `TokenStrategy::NewFungible`, in the build that ships (legacy feature off, `--features zk-api`). Reviewed 2026-11-03.

Result: no exploitable issue was found. Two fail-closed hardenings in the create-denom reply (F-1, F-2) were fixed, with a test. The informational items I-1 to I-3 are documented and need no code change.

## 1. Proto encoding
Checked against terpd `proto/osmosis/tokenfactory/v1beta1/tx.proto`:

| Message | Fields (number, wire type) | `tf_proto.rs` |
|---|---|---|
| MsgCreateDenom | sender 1 (LEN), subdenom 2 (LEN) | match |
| MsgCreateDenomResponse | new_token_denom 1 (LEN) | match |
| MsgMint | sender 1 (LEN), amount 2 (LEN, cosmos.base.v1beta1.Coin {denom 1, amount 2 as a string}), mintToAddress 3 (LEN) | match |
| MsgBurn | sender 1, amount 2 (Coin), burnFromAddress 3 | match |

The type URLs are `/osmosis.tokenfactory.v1beta1.Msg{CreateDenom,Mint,Burn}`. The e2e (real terpd b2339a9c) mints and burns against them successfully.

## 2. Decoder: length overflow and malformed replies
- A varint is bounded to 10 bytes and fails with "varint overflow".
- A LEN field uses `checked_add` plus a bounds check ("truncated field"), so it cannot overflow or read out of bounds.
- Wire types other than 0 and 2 are rejected. String fields must be valid UTF-8.
- An empty `new_token_denom` is an error.
- Every error aborts the reply, so the instantiate reverts. Nothing is half-initialized.
- **F-1 (fixed):** the reply indexed `msg_responses[0]`, which could panic on an empty response list. It now uses `.first()` and returns "MsgCreateDenom: no msg response".
- **F-2 (fixed):** the chain-reported denom was saved as is, while payouts use `TokenStrategy::denom()`, which is `factory/<contract>/<subdenom>`. The reply now requires them to be equal ("MsgCreateDenomResponse: unexpected denom"), so `DENOM` can never differ from the denom that claims mint. Test: `create_denom_reply_rejects_missing_or_unexpected_denom` covers an empty list, a wrong denom (where DENOM stays unset) and the correct denom.

## 3. Mint authority
With legacy off, `MsgMint` is emitted in exactly two places:
- `epoch::push_credit`, on a verified claim. It mints exactly the credited amount, which is the note denomination checked against `DENOMS_UTERP` and the campaign class, then sends that same amount to the destination. A zero payout is rejected.
- The create-denom reply, which mints `initial_mint` (see I-1).

The owner `ExecuteMsg::Mint` / `Burn` (`execute_mint`, `execute_burn`) and `ProcessHeadstash` (the `headstash.rs` mint arm) are `#[cfg(feature = "legacy")]` and are absent from the release wasm. The contract is the only tokenfactory admin, so no external party can mint.

## 4. Supply invariant
Minted supply = initial_mint + the sum of paid claims. Each nullifier is paid at most once, and every class is capped (`DIRECT_CLASS_PAID < cap`). So supply ≤ initial_mint + Σ(class cap × class amount) = initial_mint + the note total. The e2e checks bank supply after each claim (test iii).

## 5. Reclaim, sweep and cutoff
- Reclaim (gov only, before the cutoff) sends the non-mint balances to the community pool, skips NewFungible denoms (there is nothing to return), records RECLAIMED and halts permanently. `reclaim::ensure_live` runs before every execute. After the cutoff, reclaim is refused ("campaign has ended").
- Sweep refuses NewFungible campaigns (SWEEP_MINT_REFUSED): after the cutoff there is no prefunded balance to burn, and no further minting is possible.
- Cutoff: `check_claim` rejects at or after the cutoff block ("claim window closed") before any other work.

## 6. Pause gating
`pause_gate` covers ClaimNote, RegisterHeadstash, RegisterHeadstashSol, RegisterNotes, AcceptAnchor, ApplyAcceptedBatch, ApplyPublicBatch and BridgeMintNote (ProcessHeadstash is legacy only). That is every entry point that can mint or change claim state. Reclaim and sweep deliberately stay available while paused.

## 7. Ordering: denomination cap before the nullifier check
`check_claim` checks, in order: the cutoff, canonical32, a known anchor, the amount in `DENOMS_UTERP`, the class in the campaign, `DIRECT_CLASS_PAID >= cap`, the public reserve and nullifier existence, and only then verifies the proof. `claim_note` then calls `verify_nullifier_stateful` (the nullifier must be absent) before it stores the nullifier and increments the class counter.
- **No double pay:** the nullifier is checked in check_claim and again in claim_note, inside the same tx, before any payout.
- **No DoS:** the cap counter only moves on a valid proof with a fresh nullifier. A replayed claim of a filled class fails with "denomination cap reached" instead of "nullifier exists". Both are refusals; the e2e replay negative accepts either. All cheap checks run before the expensive proof verification.

## Informational
- **I-1 initial_mint:** minted outside the class caps. A bounded-supply instance must set `initial_mint: null` (FE-TEMPLATE-SPEC.md:103). v0.0.1 is ExistingFungible uterp, so this does not apply.
- **I-2 owner trust:** the owner (gov) can re-run SetEpochCampaign. The vk stays fixed, and registration stays fixed once leaves exist, but caps, amounts and root_id can change (root_id must be a registered distro root, and nullifiers are scoped by root_id). Under gov ownership this is accepted, and it is pre-existing rather than specific to tf-denom.
- **I-3 gas:** a mint claim measured about 302k gas (MsgMint + Send), against about 262–272k for ExistingFungible. A 300k limit fails with code 11. The claim gas default and the deploy values are now 400k (see V0.0.1-DEPLOY-PACKET.md). The 4,761.7 THIOL budget was already computed at 400k.

## Tests
- `cargo test -p cw-headstash`, no legacy: 201 passed, 0 failed.
- `--features legacy`: 228 passed, 0 failed.
- E2E: see V0.0.1-DEPLOY-PACKET.md.
