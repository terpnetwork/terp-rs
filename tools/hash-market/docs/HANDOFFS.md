# Related handoffs

- **FROST privval sidecar (docs curate + trailmark/mermaid):** `reviews/HANDOFF-FROST-SIDECAR-DOCS-2026-09-03.md` · public draft `docs/frost-privval.md`
- **FROST privval sidecar (e2e sound/unsound):** `reviews/HANDOFF-FROST-SIDECAR-E2E-2026-09-03.md` · `scripts/e2e-frost-privval.sh`

- Module multi-source oracle: `x/hashmerchant/spec/10_multi_source_oracle.md`
- ICT L3 bootstrap e2e: `docs/bootstrap/hashmerchant-ict-bootstrap-e2e-handoff.md`
- **B4/B5 full hash-market + product e2e ladder:** `docs/bootstrap/hashmerchant-b4-b5-full-runtime-and-product-e2e.md`
- **Client→isolated server ingress (milestone conductor):** `docs/bootstrap/hashmerchant-client-ingress-sequence.md`  
  - **Green S1–S5 mock:** `crates/ict-rs/examples/hashmerchant_client_ingress_e2e.rs` — server island + client attach + DB Merkle VE + dual-path claims + optional Mode C price math  
  - On-chain dual-path gold remains: `loyalty_rewards` · offline zkjwt: `loyalty_rewards_zkjwt` · NFT Docker Mode C: `hashmerchant_nft_oracle_mint`  
  - Parallelization training signals: `docs/bootstrap/training-signals/`
- **Cashu e-cash mesh sprint (design):** `docs/plans/cashu/SPRINT-CASHU-MESH-2026-07-20.md` — wire `/cashu/*` registry + encrypted wallet stash on this server (BUD/Nostr/auth first; CDK not vendored yet)
- **Content distribution (BUD + IPFS dual-index):** `docs/content-distribution.md`
- **Prep: finish S3-IPFS + Blossom before calendar Nostr e2e:** `docs/PREP-BLOSSOM-S3-CALENDAR-NOSTR.md`
- **Obsidian docs-team dashboard (daily-driver vault):** `plans/2026-07-20-library-docs-ingress/PLAN.md` under `/Users/returniflost/daily-driver/daily-driver/`
- Support façade: `tools/hashmerchant-support/`
- ICT L3 implementation: `crates/ict-rs/examples/hashmerchant_bootstrap_ubuntu.rs`
- ICT L3 fixtures: `crates/ict-rs/examples/testdata/hashmerchant-bootstrap/`
- Oline S3→IPFS webhook-collector: `crates/o-line/plays/instant-replay/`
- Capability matrix: `docs/e2e-capability-matrix.md`
- Calendar Nostr Phase B: `content/calendar.rs` + `client/nostr/calendar_egress.rs` + powered `tests/nostr_orch_suite.rs` + FE `lib/mesh/content.ts`
- **UI handoff (dao-dao-ui marketplace + auction filters):** `websites/dao-dao-ui/docs/superpowers/specs/2026-07-20-dao-marketplace-nostr-ui-handoff.md`
- Marketplace-mirror + NIP-15 egress/ingress-intent: `contracts/hash-merchant/marketplace-mirror/`, `client/nostr/marketplace_*.rs`
- Auction CW (NFT bid/settle): `contracts/revenue/auction/` + `crates/marketplace` utils
- ICT delta vs loyalty: `crates/ict-rs/docs/MARKETPLACE_VS_LOYALTY_DELTA.md`
- **Marketplace-mirror (NIP-15 inventory + buffer escrow + hashmerchant sudo):** CosmWasm package `crates/terp-rs/contracts/hash-merchant/marketplace-mirror/` (default buffer 7; cancel = buyer/admin only during `Reserved`; local stock today, proof-gated restock = v2). Thin Nostr **egress** `client/nostr/marketplace_egress.rs` + **ingress-intent** `client/nostr/marketplace_ingress.rs` (`PurchaseIntent` only — no auto-execute). Listing body: `bind_listing_body` → bare sha256 `content_cid`.

## Verify matrix (session freeze)

Paths are relative to monorepo root `terp-core` (or `cd` as shown).  
**just (from `crates/ict-rs`):** `just l3-bootstrap-mock` · `just client-ingress-pr1` · `just verify-hm-freeze` (optional `heavy=1` for loyalty Docker).

| # | Command | Docker? | Soft / hard | Artifact | Pass rule |
|---|---------|---------|-------------|----------|-----------|
| 1 | **L0 hash-market lib** | No | **Hard** | cargo test output | exit 0; all `tests::suite` pass |
| 2 | **L3 bootstrap mock** | Yes | **Hard** | `crates/ict-rs/hashmerchant_l3_report.json` (`schema_version: 1`) | exit 0; `failures: []` |
| 3 | **Ingress milestone S1–S4** (+ optional S5) | Yes | **Hard** | `crates/ict-rs/hashmerchant_ingress_report.json` (`schema_version: 1`) | exit 0; phases true; `failures: []`; wipe false |
| 4 | **Loyalty dual-path on-chain** | Yes | **Soft / nightly** (terpd gold) | example stdout / gas report | exit 0 |
| 5 | **zk-jwt offline** | No | **Hard** offline | example stdout | exit 0 |

```bash
# 1. L0 hash-market lib (no Docker) — HARD
cd crates/terp-rs/tools/hash-market && cargo test -p hash-market --lib tests::suite

# 2. L3 bootstrap mock (Docker) — HARD; report schema_version=1
cd crates/ict-rs/ict-rs
ICT_HM_RUNTIME=mock ICT_L3_REPORT=../hashmerchant_l3_report.json \
  cargo run --example hashmerchant_bootstrap_ubuntu --features docker

# 3. Ingress milestone S1–S4 (Docker) — HARD product story
#    S1 server · S2 client attach · S3 Postgres Merkle→VE · S4 dual-path claims
#    Add S5 or ICT_INGRESS_NFT=1 for Mode C price-band math
cd crates/ict-rs/ict-rs
ICT_HM_RUNTIME=mock ICT_INGRESS_PHASES=S1,S2,S3,S4,S5 \
ICT_INGRESS_REPORT=../hashmerchant_ingress_report.json \
  cargo run --example hashmerchant_client_ingress_e2e --features docker

# 4. Loyalty dual-path on-chain (Docker) — NIGHTLY / OPTIONAL gold
cd crates/ict-rs
cargo run --example loyalty_rewards --features "docker hashmerchant"

# 5. zk-jwt offline (no Docker) — HARD offline surface
cd crates/ict-rs
cargo run --example loyalty_rewards_zkjwt
```

**Reports:** L3 + ingress JSON both use `schema_version: 1`. Ingress also records `merkle_root`, `leaf_count`, `codecs.loyalty_db_row` / `hm_loyalty_v1`. Soft notes never flip hard gates green.
