# Headstash registration through smart-account authenticators (design spec)

Status: DRAFT, design only. No code, no chain txs. Proposed by Terp Network, 2026-10-26.
Scope: registration (and analysis of claim submission). Circuit, vk_hash and claim payout path unchanged.

> Proposal (verbatim intent): smart-account auth lets eligible accounts register and prove ownership with
> no fees and no sequence mismatch, using the granular per-message tracks of smart-account authenticators;
> Solana signatures are accepted with the same method as the old public headstash claim. This is how Solana
> joins, and how users avoid a centralized relayer or ingress.

## 0. What exists today (read from the tree)

| Piece | Path | Relevant facts |
|---|---|---|
| Smart-account module | `x/smart-account/{ante,post,authenticator,keeper}` | Osmosis-derived. Per-message authenticator selection, `Authenticate` (stateless) then `Track` closures that commit only if every message authenticates, `ConfirmExecution` in post handler. |
| Ante wiring | `app/abci/ante.go` | `CircuitBreakerDecorator` picks the authenticator chain or the classic chain. Both chains run `DeductFeeDecorator` and `IncrementSequenceDecorator`. |
| Fees in smart-account | `x/smart-account/ante/ante.go:63-215` | Fee payer = first signer, cannot be overridden (`ValidateAuthenticatorFeePayer`). Payer is authenticated under a small free gas budget, then fees are deducted once. Feegrant (`FeeGranter`) is honored. **No fee bypass exists.** |
| Replay protection | `x/smart-account/authenticator/replay_protection.go`, `ante.go:171` | Ante uses `SequenceMatch` (signature sequence must equal account sequence). Post uses `NoReplayProtection`. **Sequence is still enforced.** |
| Global fee bypass | `x/globalfee/ante/fee.go`, `app/app.go:519` `GetDefaultBypassFeeMessages` | Bypass list is IBC only, capped by `maxBypassMinFeeMsgGasUsage = 2_000_000`. Only lifts the *min-fee* check; fee deduction still runs if a fee is set. |
| Message scoping | `authenticator/message_filter.go`, `all_of.go`, `any_of.go` | JSON pattern match on message type and fields; compose with `AllOf`. |
| CosmWasm authenticators | `authenticator/cosmwasm.go` | Sudo `authenticate / track / confirm_execution / on_authenticator_added / removed`. |
| ZK authenticator | `authenticator/zkproof_authenticator.go` | Shape only; `GlobalPathAVerifier` not registered by default; never spends nullifiers. |
| Ed25519 authenticator contract | `crates/terp-rs-headstash-membership-vk/contracts/smart-accounts/terp-ed25519` | Verifies `ed25519_verify(sign_mode_direct tx bytes, sig, pk)`; one pubkey per account. Signs **Cosmos tx bytes**, which Phantom/Solflare cannot produce as `signMessage` output in a human-readable way. Also `terp-eth`, `terp-passkey`, `terp-recovery`, etc. |
| cw-headstash registration | `contracts/headstash/cw-headstash/src/registration.rs`, `msg.rs:151` | `RegisterHeadstash{key_com, amount, signature, eligibility_index, eligibility_path}`. `register_message()` (l.327) builds `headstash-register-v1:{ctx}:{key_com}:{amount}`, `recover_address()` (l.335) does EIP-191 + `secp256k1_recover_pubkey` + keccak to a 20-byte address, `eligibility_leaf(address:[u8;20], amount)` (l.404), "already registered" guard (l.234). Leaf type is EVM-only. |
| Relay | `tools/hash-market/src/bin/headstash-register-relay.rs` | Current permissionless relay that pays fees for registration. |
| Old public headstash Solana support | git `fdb575c` / `92b81d8`: `scripts/headstash/solana-utils.js`, `headstash-scripts.js`, `headstash.yaml` (`stoned-ape-club`, `chain_type: solana`, `allocation_percentage: 0.10400262686567165`) | Snapshot stored the Solana address as `base64(utf8(base58 address))` "in the format used to verify the offline signature". **The old claim contract source is not in this repo or its history** (searched `-S solana/ed25519/phantom/ed25519_verify`). Section 4 reconstructs the format; it must be confirmed against the deployed old contract before freezing. |

Key correction to the proposal: on today's chain, smart-account auth **does not** bypass fees and **does** enforce
sequence. Both properties require new, small chain changes (section 3). The authenticator machinery is still the
right place for it, because of `Track` and message scoping.

## 1. Design summary

Registration becomes a **keyless-sender transaction**: the tx is signed by nobody the user has to fund. A
module-controlled *registrar account* `R` (a plain account address with no private key, derived like a module
address) carries one authenticator, `HeadstashRegistrationAuthenticator` (HRA):

```
AllOf(
  MessageFilter{ "@type": "/cosmwasm.wasm.v1.MsgExecuteContract",
                 "sender": R, "contract": <cw-headstash>, "funds": [],
                 "msg": {"register_headstash": {}} },      // exactly one msg per tx
  HeadstashOwnershipAuth   // native Go, or CosmWasm via cosmwasm.go
)
```

`HeadstashOwnershipAuth.Authenticate` verifies the **eligible key's** signature (secp256k1/EIP-191 for EVM,
ed25519 for Solana) over the registration message carried in the tx signature slot, checks the eligibility
Merkle path, and checks the address is not yet registered. `Track` records `registered[addr_tag]` in the
authenticator store (the "granular track"), so a second tx for the same address fails in ante, before
consuming block space. cw-headstash **re-verifies the same signature** in execute (defence in depth and so the
ownership check can never be skipped by a mis-configured authenticator).

Anyone (the user's own browser, a wallet, any node) broadcasts the tx straight to any RPC. No relayer, no
sidecar, no fee account.

## 2. Data flow

```mermaid
sequenceDiagram
    autonumber
    participant B as Browser (zakura app)
    participant W as Wallet (MetaMask / Phantom / Solflare)
    participant N as Any Terp RPC node
    participant A as Ante: CircuitBreaker -> Authenticator chain
    participant H as HRA (MessageFilter + OwnershipAuth)
    participant C as cw-headstash
    B->>B: secret <- random, key_com = Poseidon(secret)
    B->>B: build msg M (EVM: v1 text; Solana: v1-sol text, section 4)
    B->>W: personal_sign(M) / signMessage(utf8(M))
    W-->>B: sig (65B secp256k1 or 64B ed25519) [+ pubkey for Solana]
    B->>B: build Tx{ MsgExecuteContract(sender=R, cw-headstash, RegisterHeadstash{...}) }, fee=0, unordered=true, timeout_timestamp<=10m, TxExtension{selected_authenticators=[HRA id]}
    B->>N: BroadcastTx (CheckTx)
    N->>A: CheckTx
    A->>H: Authenticate(req{msg, signature=registration payload})
    H->>H: scope: one msg, contract, R, no funds, no other fields
    H->>H: verify sig over rebuilt M (ctx from contract config incl. chain_id)
    H->>H: verify eligibility path -> root, not registered, rate bucket ok
    H-->>A: ok (fee=0 accepted only for HRA-only txs)
    A->>H: Track -> mark pending/registered(addr_tag)
    N-->>B: txhash (mempool)
    Note over N,C: DeliverTx in block
    A->>H: Authenticate + Track again (DeliverTx)
    A->>C: execute RegisterHeadstash
    C->>C: re-verify sig, path, not registered, append key_com to Poseidon tree
    C-->>B: event leaf_index, new root
```

EVM and Solana differ only in step 3-4 and in the leaf type.

## 3. Fees and sequence

### 3.1 Fees
Today the authenticator ante deducts `fee` from the first signer and globalfee requires a min fee. Proposed:
- Add a narrow **zero-fee lane**: in `AuthenticatorDecorator`, if the tx has exactly one message, the signer is
  `R`, and the selected authenticator is the HRA id, skip `deductFeeDecorator` and require `fee == 0`
  and `gas_limit <= HRA_MAX_GAS` (e.g. 400k; measure: secp recover + 32-deep keccak path + Poseidon append).
- Add `MsgExecuteContract` to globalfee bypass **only** via a predicate (`sender == R && contract == cw-headstash`),
  not by type URL, because type-URL bypass would free every wasm execute. Prefer a dedicated
  `headstash` ante decorator placed before globalfee that marks the context `zeroFeeApproved`.
- Alternative with no new fee logic: `R` gets a feegrant from a community-pool-funded granter, `AllowedMsgAllowance`
  restricted to MsgExecuteContract plus a periodic spend cap. Simpler, but someone pays and the cap is a DoS lever.
  Recommended as the fallback if governance rejects a zero-fee lane.

### 3.2 Sequence
All registrations share one sender `R`, so ordered sequences would collide (the "sequence mismatch" the
proposal wants to avoid). SDK v0.55 (go.mod) supports **unordered transactions** (`unordered=true` +
`timeout_timestamp`, replay protected by tx hash in the unordered nonce store).
- HRA path must use unordered txs; extend `GenerateAuthenticationRequest` callers in ante with a replay mode
  that, for `R` only, accepts unordered and skips `SequenceMatch`; skip `IncrementSequenceDecorator` for `R`.
- Replay safety does not depend on sequence: the registration is idempotent-rejecting (one per address via
  Track + contract state), and the tx hash store blocks byte-identical replays within the timeout window.
- `R` needs an account (`BaseAccount` with no pubkey) created at upgrade; `SetPubKeyDecorator` is not on the
  authenticator chain, so no pubkey is needed.

## 4. Message formats

EVM (unchanged): EIP-191 `personal_sign` over
`headstash-register-v1:{ctx}:{key_com}:{amount}`.

Solana (new, matches the old public claim style: wallet `signMessage` over UTF-8 text, address as base58,
snapshot address stored as `base64(utf8(base58))`):
```
headstash-register-sol-v1:{ctx}:{key_com}:{amount}
```
- Signed bytes = exactly the UTF-8 string (Phantom/Solflare `signMessage(new TextEncoder().encode(M))`), no
  Solana off-chain-message header. If the old contract used the off-chain-message envelope
  (`\xffsolana offchain` + version + format + len), copy that instead; **confirm against the old contract (OQ1)**.
- Payload in the authenticator signature slot and in `RegisterHeadstash`:
  `{scheme:"ed25519", pubkey: 32B, signature: 64B}`; `scheme:"secp256k1-eip191"` for EVM (65B sig, address recovered).
- `ctx` = existing registration ctx (contract address + chain_id + epoch/round id, as in `validate_config`);
  must include `chain_id` explicitly (see 6.1).
- `key_com`, `amount`: same lowercase hex / decimal encodings as v1.

Eligibility leaf, typed to separate the two address spaces:
```
leaf = keccak256( tag || address_bytes || amount_be64 )
tag = 0x01 EVM (20B address) | 0x02 Solana (32B ed25519 pubkey, decoded from base58)
```
Today `eligibility_leaf(address:[u8;20], amount)` has no tag; adding the tag means **a new eligibility root**
(rebuild with stoned-ape-club leaves). If the current root is already published/locked, keep EVM leaves
byte-identical (tag only on Solana leaves, padded length differs, so no collision) and document it (OQ2).

## 5. Relayer, ingress, and claim unlinkability

### 5.1 Registration
Registration is linkable by design (the eligible key signs a `key_com`). With HRA the user's own browser
broadcasts; `headstash-register-relay` becomes optional (keep it for users behind restrictive networks).
The node operator sees the user IP, the same as any wallet broadcast; recommend multiple public RPCs.

### 5.2 Claims
Claims must not link `nullifier`/destination to the registering address. Options:
- **Claim via the same `R`/HRA lane: no.** Nothing ties a claim to the eligible key cryptographically, but
  timing (register then claim right away from the same IP/RPC), same-session browser fingerprint, and a fee
  payer would link it. A claim tx through a *separate* keyless account `Rc` with a `ZkProofAuthenticatorV1` +
  nullifier gate (`AllOf`, as the zkproof file suggests) avoids a funded sender, because the proof itself is
  the authorization and the destination is bound in the proof.
- **Leaks that remain** even via `Rc`: submission IP and timing; per-claim on-chain txs (the sidecar batch
  today hides individual claim timing inside a batch and is the anonymity-set amplifier). Direct per-claim txs
  shrink the effective anonymity set to "registered before this block" minus timing correlation.
- **Ownership requirement**: claims prove membership in the registration tree, whose leaves were admitted only
  after the ownership check. So an `Rc` lane keeps the requirement, provided the in-ante ZK verifier is the
  same vk_hash-locked circuit and the contract still verifies (no "authenticator already checked it" skip).
- **Recommendation**: phase 1 registration only. Phase 2 an optional direct-claim lane through `Rc` gated by
  the Path-A verifier, with a UI warning and a delay/jitter suggestion; keep the batching ingress as the
  default for privacy. Not before `GlobalPathAVerifier` is wired in keepers.

## 6. Security review

1. **Replay across ctx/chain**: message includes `ctx`; `ctx` must bind chain_id, cw-headstash address and
   registration round. Signatures for a testnet or another headstash instance fail. Unordered tx hash store
   plus "already registered" prevents in-chain replay.
2. **Domain separation**: distinct prefixes `headstash-register-v1` (EVM, inside EIP-191) and
   `headstash-register-sol-v1` (Solana raw). Neither can be a valid Cosmos SignDoc, Solana transaction (Phantom
   refuses to `signMessage` tx-shaped bytes, and text starting with `h` is not a valid tx header), or an EIP-712
   payload. Leaf tags 0x01/0x02 separate address spaces.
3. **Malleability**: secp256k1: require low-s and v in {27,28}/{0,1}; identity is the recovered address, not the
   signature, so a malleated sig cannot double-register. ed25519: `ed25519_verify` in cosmwasm-crypto
   (ed25519-zebra) rejects non-canonical s; registration identity is the pubkey. Never key any store by signature bytes.
4. **DoS/spam without fees**: (a) ante does all checks in CheckTx, so invalid txs never enter the mempool;
   (b) one registration per eligible address via Track (tx count is bounded by eligible-set size, roughly
   thousands); (c) `HRA_MAX_GAS` and static gas charged before crypto; (d) cap HRA txs per block (e.g. 50) and
   mempool priority below fee-paying txs; (e) CheckTx cost of a bogus tx is one sig verify, so per-peer rate
   limits in CometBFT mempool config; (f) window open/close in contract config.
5. **Ownership**: verified twice, in HRA and in cw-headstash execute. Contract must not accept an
   "authenticated by ante" flag. Requirement "no sidecar or verifier may skip the ownership check" is preserved.
6. **Authenticator scope creep**: `R` has exactly one authenticator; `R` holds no funds (contract must reject
   `funds`, and any bank send to `R` is irrelevant because HRA only signs one exact message form);
   authenticator management for `R` only by governance (no `MsgAddAuthenticator` from `R` itself, which the
   filter already excludes); MessageFilter pattern pins contract address, so a migration of cw-headstash
   requires a gov update; zero-fee lane keyed on authenticator id + `R`, not on message type.
7. **Front-running**: a mempool observer can rebroadcast the same tx; harmless (same key_com). Cannot change
   `key_com` because it is signed.
8. **Circuit breaker**: if smart-account is disabled via circuit breaker, registration falls back to the relay path; document it.

## 7. Changes per component (rough effort)

| Component | Change | Effort |
|---|---|---|
| Chain: `x/smart-account` | `HeadstashOwnershipAuth` (Go native preferred for gas; or CosmWasm via existing sudo) with Track store; unordered/no-sequence replay mode for `R`; zero-fee lane in `AuthenticatorDecorator` | 1.5-2.5 wk incl. tests |
| Chain: app/upgrade | create `R`, register HRA via upgrade handler/gov, globalfee predicate decorator, per-block cap | 3-5 d |
| cw-headstash | `RegisterHeadstash` signature payload enum `{Secp256k1Eip191, Ed25519{pubkey}}`; `register_message_sol`; ed25519 via `deps.api.ed25519_verify`; tagged leaf; reject funds; optional `sender == R or anyone` (keep permissionless) | 3-5 d |
| Eligibility root / tools/headstash | include stoned-ape-club leaves (base58 decode to 32B; snapshot holds base64(utf8(base58))); rebuild root; publish leaf spec | 2-3 d |
| Front end | Phantom/Solflare `signMessage`, unordered tx builder with `TxExtension` selected authenticator, direct broadcast, multi-RPC | 1 wk |
| Relay | keep `headstash-register-relay` as fallback, teach it ed25519 | 1 d |

## 8. vk_hash impact

None. Registration is outside the circuit: the circuit proves membership of `Poseidon(secret)` in the registration
tree and the nullifier, independent of how the leaf was admitted. The eligibility-leaf tag change touches only
the eligibility root checked in cw-headstash, not circuit public inputs. The locked vk_hash stays valid.
Only the phase-2 claim lane would touch verification, and it must reuse the same vk_hash.

## 9. Open questions / decisions

- OQ1: exact old Solana claim message (raw text vs Solana off-chain envelope, field order). Source of the old contract is not in this repo.
- OQ2: is the current eligibility root already published? If so, re-root (adds tag) or EVM-untagged + Solana-tagged.
- OQ3: zero-fee lane (chain change) vs feegranted `R` (no fee-logic change, needs funding).
- OQ4: native Go authenticator (cheaper, chain upgrade per change) vs CosmWasm authenticator contract (upgradable, heavier gas).
- OQ5: per-block cap and mempool priority values.
- OQ6: ship phase-2 direct-claim lane at all, given anonymity-set cost.
- OQ7: Solana pubkey or base58 string as on-chain identity in events (pubkey recommended).
