# Bridge — FCR-powered (next); tacit on hold — Workstream B

**2026-11-10 decision (owner):** tacit (src-company BTC→ETH→Terp) is **on hold**. The bridge will be powered by the
team's **FCR (Ethereum Fast Confirmation Rule) integration draft** — the next big step. Not implemented here.

## FCR draft located
- `~/grok-bot/xchain-liquidity-implementation-plan.html` (on the Mac, not groot-wan; 2026-11-02):
  - Design A: inbound EVM→Terp via hashmerchant validator attestations of **FCR roots** (66.7% vote), outbound
    Terp→EVM via SP1 ics07-tendermint; solidity-ibc-eureka on Ethereum/Base.
  - WP1: hashmerchant FCR attestation (Lighthouse ≥ v8.3.0 with fast confirmation; vote extension attestation mode `fcr | finalized`,
    both roots per VE; Base confirmed when its batch is in an FCR-confirmed L1 block).
  - WP2: IBC v2 client on Terp verifies EVM packet commitments (eth_getProof) against attested roots; FCR root ≤ cap
    (1,000,000 TERP/THIOL), finalized root above; unknown mode rejected; unit tests + ict-rs anvil e2e.
  - Guardrails G1–G10 (G6 cap), triggers (root divergence > 1/3, client share > 1/3 → pause FCR, over-cap → finalized),
    tests T1–T8 (T3 FCR stall 33 slots → finalized), phases (mainnet finalized-only before FCR).
  - Internal refs it cites: `research/sovereign-ibc-v2-bridge.md`, `config/bridge-policy.toml` — **not found** on the Mac or groot-wan.
- No FCR code/branch found in terp-rs, terp-core or terp-core-hm-vext (`git log --all --grep fcr`, file search).

## How B's work plugs in
- hashmerchant `CapabilityQuorum` + HashRoot is the same primitive the registry now checks (`cw-lc-registry/src/quorum.rs`);
  an FCR capability (`chain_uid = eth-mainnet`, `algo = fcr`) is the natural input for the WP2 client.
- The rotation/aggregator "preconf with k-of-n fallback" pattern matches FCR-vs-finalized with a cap.
- Suite slot renamed `bridge-fcr` (Pending) in `PrivacySuite::standard()`.

## tacit (on hold, kept for reference)
Ref clone `~/abstract/ref/tacit-src-company` (HEAD 8463046). Path was: Taproot envelope burn → SP1 proof → EVM
confidential pool → IBC v2 to Terp, mocked on anvil via ict-rs.
