# terp-seams

Single crate for **private DEX**, **bridge mint (Tacit hinge)**, **Cash App → ZEC corridor**, and **compose glue**.

**Domain tags live in [`src/domain.rs`](src/domain.rs)** — do not fork those bytes in other crates.

**Imports:** `use terp_seams::{dex, bridge, corridor, compose, domain};`

There is **one** seams crate. Optional features: `dex`, `bridge`, `corridor`, `compose` (default: all on). PIR and similar consumers use `default-features = false, features = ["dex"]`. Do not restore `private_dex_seams` / `seam_private_dex`.

## Deduped / reused

| Was duplicated | Now |
|----------------|-----|
| `PRICE_SCALE` (DEX + corridor) | `domain::PRICE_SCALE` |
| `DOMAIN_TAG_NOTE_OUT`, origin/nf/cm encodings | `domain::*` |
| `Hash32` / `AssetId32` | `domain::Hash32` / `AssetId32` |
| Bridge vs note origin/nf constants | single `ORIGIN_*` / `NF_*` |
| Compose pipeline | `compose` module (hinge → SEAM → swap) |

Still **two** oracle mid types (DEX height-based vs corridor unix-time) — same scale, different clocks; do not merge without a conversion API.

`AssetRegistry` (bridge mapped set) vs `AssetMap` (DEX) vs `compose::AssetRegistryView` (projections) — keep view + `to_bridge_registry` / `to_dex_asset_map`.
